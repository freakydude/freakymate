//! Background service: keeps the connection to Moonraker, maintains the state and executes commands.
//!
//! The UI talks to the printer exclusively through [`PrinterService`]. State arrives as a
//! [`Snapshot`] over a `watch` channel, one-off results as [`Event`].

use crate::commands::Command;
use crate::credentials::{Credential, CredentialStore};
use crate::moonraker::access::{self, AccessError};
use crate::moonraker::{Call, CallError, Connection, Endpoint, Notification, PendingCall};
use crate::sources::klipper::{FanWatch, HeightWatch, KlippyState, PendingTargets, PrintState};
use crate::sources::moonraker_server::files::{self, FileEntry, FileMetadata, ThumbnailImage};
use crate::sources::moonraker_server::gcode_store::{self, ConsoleKind, ConsoleLine};
use crate::sources::moonraker_server::history::{self, PrintHistory};
use crate::sources::moonraker_server::machine::{self, HostInterface};
use crate::sources::moonraker_server::power::{self, PowerDevice};
use crate::sources::moonraker_server::temperature_store::{self, TemperatureSeries};
use crate::sources::moonraker_server::update_manager::{self, UpdateInfo};
use crate::sources::moonraker_server::wled::{self, WledStrip};
use crate::sources::moonraker_server::{server_info, ServerComponents};
use crate::sources::spoolman::{self, Spool, SpoolmanState};
use crate::sources::{ObjectStore, PrinterStatus, Sources};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use tokio::sync::{mpsc, watch, Semaphore};

const RECONNECT_DELAY: Duration = Duration::from_secs(2);
const KLIPPY_POLL_INTERVAL: Duration = Duration::from_secs(2);
const QUERY_TIMEOUT: Duration = Duration::from_secs(15);
/// File details (metadata and thumbnail) fetched at the same time; the file list asks for up to 40 at once.
const PARALLEL_FILE_DETAILS: usize = 4;
const CONSOLE_HISTORY: u32 = 100;
/// Newest jobs read from the job history; files last printed before that show the slicer estimate.
const PRINT_HISTORY_LIMIT: u32 = 300;
/// Moonraker reports the active spool and the connection itself; the weights of the spools it does not, so they are read again now and then.
const SPOOLMAN_POLL_INTERVAL: Duration = Duration::from_secs(60);
/// Moonraker does not report WLED changes, and Mainsail or a macro may switch a strip too.
const WLED_POLL_INTERVAL: Duration = Duration::from_secs(5);
/// Spools for the picker: the most recently used ones.
const SPOOL_LIST_LIMIT: u32 = 100;
const CLIENT_NAME: &str = "freakymate";
/// A command whose answer takes longer is shown as running (busy indicator).
const LONG_COMMAND: Duration = Duration::from_secs(1);
/// Moonraker sends nothing more to a connection that lost its authorization (the same user logged out elsewhere),
/// not even the logout; so an authorized session asks now and then whether it still may.
const ACCESS_CHECK_INTERVAL: Duration = Duration::from_secs(30);
/// Ids of running commands; unique across reconnections, where answers of an old connection may still arrive.
static NEXT_COMMAND_ID: AtomicU64 = AtomicU64::new(0);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ConnectionState {
    #[default]
    Connecting,
    Connected,
    Disconnected,
    /// Connected, but Moonraker answers only after a login ([`PrinterService::log_in`]).
    LoginRequired,
}

/// How the display got into Moonraker.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum Access {
    /// Without credentials: Moonraker trusts this machine or has no authorization.
    #[default]
    Open,
    /// With the API key from the credentials file.
    ApiKey,
    /// As this user, logged in on the display.
    User(String),
}

impl Access {
    fn is_authorized(&self) -> bool {
        *self != Self::Open
    }
}

/// Current overall state for display.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Snapshot {
    pub endpoint: String,
    pub connection: ConnectionState,
    pub access: Access,
    pub status: PrinterStatus,
    /// What this printer has; detected again with every subscription.
    pub sources: Sources,
}

/// One-off requests for data that is not part of the running subscription.
#[derive(Debug, Clone, PartialEq)]
pub enum Query {
    Files,
    /// Metadata and then the thumbnail of a file.
    FileDetails {
        filename: String,
    },
    /// The last completed print of each file and the newest job, from the Moonraker job history.
    PrintHistory,
    PowerDevices,
    Updates,
    ConsoleHistory,
    /// Network interfaces of the printer host.
    HostNetwork,
    /// The last minutes of every heater and sensor from Moonraker's temperature store.
    TemperatureHistory,
    /// Spoolman's state and the spools of the gates and the active spool; needs Moonraker's `spoolman` component.
    Spoolman,
    /// The most recently used spools of Spoolman, for the picker.
    SpoolList,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Event {
    Files(Vec<FileEntry>),
    FileMetadata(Box<FileMetadata>),
    PrintHistory(PrintHistory),
    Thumbnail {
        filename: String,
        image: ThumbnailImage,
    },
    PowerDevices(Vec<PowerDevice>),
    Updates(Vec<UpdateInfo>),
    /// Moonraker's update manager looks for or installs updates (from here or another client).
    UpdateManagerBusy(bool),
    ConsoleHistory(Vec<ConsoleLine>),
    ConsoleLine(ConsoleLine),
    HostNetwork(Vec<HostInterface>),
    TemperatureHistory(HashMap<String, TemperatureSeries>),
    Spoolman(Box<SpoolmanState>),
    SpoolList(Vec<Spool>),
    /// A command changed a spool in Spoolman or the active spool; the state is out of date.
    SpoolmanChanged,
    CommandFailed {
        command: String,
        message: String,
    },
    /// A command sent from here waits longer than `LONG_COMMAND` for its answer (homing, leveling, an update, ...).
    CommandRunning {
        id: u64,
        command: String,
    },
    /// The command of an earlier `CommandRunning` has its answer, or failed.
    CommandFinished {
        id: u64,
    },
    /// Moonraker refused the login; Moonraker's reason.
    LoginFailed(String),
}

enum Request {
    Command(Command),
    Query(Query),
    Connect(Endpoint),
    LogIn { username: String, password: String },
    LogOut,
}

/// Handle for the UI; cheap to clone.
#[derive(Clone)]
pub struct PrinterService {
    requests: mpsc::UnboundedSender<Request>,
}

pub struct ServiceChannels {
    pub snapshots: watch::Receiver<Snapshot>,
    pub events: mpsc::UnboundedReceiver<Event>,
}

impl PrinterService {
    /// Starts the service on the given Tokio runtime; `credentials` holds the access to Moonraker servers that do not
    /// trust this machine.
    pub fn start(
        runtime: &tokio::runtime::Handle,
        endpoint: Endpoint,
        credentials: CredentialStore,
    ) -> (Self, ServiceChannels) {
        let (requests, requests_rx) = mpsc::unbounded_channel();
        let (snapshot_tx, snapshots) = watch::channel(Snapshot { endpoint: endpoint.display(), ..Snapshot::default() });
        let (events_tx, events) = mpsc::unbounded_channel();
        let outputs = Outputs { snapshot: snapshot_tx, events: events_tx };
        runtime.spawn(run(endpoint, credentials, requests_rx, outputs));
        (Self { requests }, ServiceChannels { snapshots, events })
    }

    pub fn send(&self, command: Command) {
        let _ = self.requests.send(Request::Command(command));
    }

    pub fn query(&self, query: Query) {
        let _ = self.requests.send(Request::Query(query));
    }

    /// Reconnects, e.g. after the address was changed in the settings.
    pub fn connect_to(&self, endpoint: Endpoint) {
        let _ = self.requests.send(Request::Connect(endpoint));
    }

    /// Logs in while the connection is [`ConnectionState::LoginRequired`]; only the refresh token is kept, the
    /// password goes to Moonraker alone. A refusal arrives as [`Event::LoginFailed`].
    pub fn log_in(&self, username: &str, password: &str) {
        let _ = self.requests.send(Request::LogIn { username: username.to_owned(), password: password.to_owned() });
    }

    /// Logs out at Moonraker, forgets the stored credential of this address and connects again without it.
    pub fn log_out(&self) {
        let _ = self.requests.send(Request::LogOut);
    }
}

enum SessionEnd {
    ConnectionLost,
    SwitchTo(Endpoint),
    Shutdown,
}

struct Outputs {
    snapshot: watch::Sender<Snapshot>,
    events: mpsc::UnboundedSender<Event>,
}

impl Outputs {
    fn event(&self, event: Event) {
        let _ = self.events.send(event);
    }
}

async fn run(
    mut endpoint: Endpoint,
    credentials: CredentialStore,
    mut requests: mpsc::UnboundedReceiver<Request>,
    outputs: Outputs,
) {
    let http = reqwest::Client::new();
    // A printer that stays off would otherwise fill the log (an SD card) with the same failure on every retry.
    let mut reported_failure: Option<String> = None;
    loop {
        publish_connection(&outputs, &endpoint, ConnectionState::Connecting);
        let end = match Connection::open(&endpoint.websocket_url()).await {
            Ok((connection, notifications)) => {
                reported_failure = None;
                log::info!("connected to {}", endpoint.websocket_url());
                let (wled_answers, wled_answers_rx) = mpsc::unbounded_channel();
                let session =
                    Session::new(connection, http.clone(), endpoint.clone(), &credentials, &outputs, wled_answers);
                session.run(notifications, wled_answers_rx, &mut requests).await
            }
            Err(error) => {
                let failure = error.to_string();
                if reported_failure.as_deref() != Some(failure.as_str()) {
                    log::info!("cannot connect to {}: {failure}", endpoint.websocket_url());
                    reported_failure = Some(failure);
                }
                SessionEnd::ConnectionLost
            }
        };
        match end {
            SessionEnd::Shutdown => return,
            SessionEnd::SwitchTo(next) => endpoint = next,
            SessionEnd::ConnectionLost => {
                publish_connection(&outputs, &endpoint, ConnectionState::Disconnected);
                match wait_for_retry(&mut requests, &outputs, &endpoint, &credentials).await {
                    Retry::Same => {}
                    Retry::SwitchTo(next) => endpoint = next,
                    Retry::Shutdown => return,
                }
            }
        }
    }
}

fn publish_connection(outputs: &Outputs, endpoint: &Endpoint, connection: ConnectionState) {
    outputs.snapshot.send_replace(Snapshot { endpoint: endpoint.display(), connection, ..Snapshot::default() });
}

enum Retry {
    Same,
    SwitchTo(Endpoint),
    Shutdown,
}

/// Waits before the next connection attempt; commands fail immediately during this time.
async fn wait_for_retry(
    requests: &mut mpsc::UnboundedReceiver<Request>,
    outputs: &Outputs,
    endpoint: &Endpoint,
    credentials: &CredentialStore,
) -> Retry {
    let delay = tokio::time::sleep(RECONNECT_DELAY);
    tokio::pin!(delay);
    loop {
        tokio::select! {
            () = &mut delay => return Retry::Same,
            request = requests.recv() => match request {
                None => return Retry::Shutdown,
                Some(Request::Connect(endpoint)) => return Retry::SwitchTo(endpoint),
                Some(Request::Command(command)) => outputs.event(Event::CommandFailed {
                    command: command.describe(),
                    message: "not connected to Moonraker".to_owned(),
                }),
                Some(Request::LogIn { .. }) => outputs.event(Event::LoginFailed("not connected to Moonraker".to_owned())),
                // Moonraker is not reachable to end the login there; the display forgets it anyway.
                Some(Request::LogOut) => forget_credential(credentials, endpoint),
                Some(Request::Query(_)) => {}
            }
        }
    }
}

fn remember_credential(credentials: &CredentialStore, endpoint: &Endpoint, credential: Option<Credential>) {
    if let Err(error) = credentials.store(endpoint, credential) {
        log::warn!("cannot store the access to {}: {error}", endpoint.display());
    }
}

fn forget_credential(credentials: &CredentialStore, endpoint: &Endpoint) {
    remember_credential(credentials, endpoint, None);
}

/// What the display tells Moonraker about itself in `server.connection.identify`.
fn identity() -> Value {
    json!({
        "client_name": CLIENT_NAME,
        "version": crate::VERSION,
        "type": "display",
        "url": env!("CARGO_PKG_REPOSITORY"),
    })
}

struct Session<'a> {
    connection: Connection,
    http: reqwest::Client,
    endpoint: Endpoint,
    credentials: &'a CredentialStore,
    access: Access,
    outputs: &'a Outputs,
    store: ObjectStore,
    sources: Sources,
    /// From `server.info`, known before Klipper is ready; part of [`Sources`] once subscribed.
    components: ServerComponents,
    klippy_ready: bool,
    job_filename: String,
    job_state: PrintState,
    /// Targets sent but not yet reported back; the snapshot shows them.
    pending_targets: PendingTargets,
    /// What each fan's tachometer has measured since Klipper started; the snapshot marks the stalled ones.
    fan_watch: FanWatch,
    height_watch: HeightWatch,
    /// The spools of the gates at the last look; `None` until Spoolman was asked once.
    spoolman_gate_spools: Option<Vec<u32>>,
    /// Limits the file details in flight, so a long file list does not flood Moonraker.
    file_details: Arc<Semaphore>,
    /// Moonraker's WLED strips; part of the snapshot's lights.
    wled: Vec<WledStrip>,
    /// The answers of the commands to a WLED strip, which carry its new state.
    wled_answers: mpsc::UnboundedSender<Vec<WledStrip>>,
}

impl<'a> Session<'a> {
    fn new(
        connection: Connection,
        http: reqwest::Client,
        endpoint: Endpoint,
        credentials: &'a CredentialStore,
        outputs: &'a Outputs,
        wled_answers: mpsc::UnboundedSender<Vec<WledStrip>>,
    ) -> Self {
        Self {
            connection,
            http,
            endpoint,
            credentials,
            access: Access::Open,
            outputs,
            store: ObjectStore::default(),
            sources: Sources::default(),
            components: ServerComponents::default(),
            klippy_ready: false,
            job_filename: String::new(),
            job_state: PrintState::default(),
            pending_targets: PendingTargets::default(),
            fan_watch: FanWatch::default(),
            height_watch: HeightWatch::default(),
            spoolman_gate_spools: None,
            file_details: Arc::new(Semaphore::new(PARALLEL_FILE_DETAILS)),
            wled: Vec::new(),
            wled_answers,
        }
    }

    async fn run(
        mut self,
        mut notifications: mpsc::UnboundedReceiver<Notification>,
        mut wled_answers: mpsc::UnboundedReceiver<Vec<WledStrip>>,
        requests: &mut mpsc::UnboundedReceiver<Request>,
    ) -> SessionEnd {
        match self.introduce().await {
            Ok(()) => {}
            Err(AccessError::Refused(message)) => {
                log::info!("Moonraker wants a login: {message}");
                if let Some(end) = self.wait_for_login(&mut notifications, requests).await {
                    return end;
                }
            }
            Err(error) => {
                log::warn!("Moonraker handshake failed: {error}");
                return SessionEnd::ConnectionLost;
            }
        }
        if let Err(error) = self.refresh_klippy().await {
            log::warn!("Moonraker handshake failed: {error}");
            return SessionEnd::ConnectionLost;
        }
        self.publish();
        let mut klippy_poll = tokio::time::interval(KLIPPY_POLL_INTERVAL);
        let mut spoolman_poll = tokio::time::interval(SPOOLMAN_POLL_INTERVAL);
        let mut wled_poll = tokio::time::interval(WLED_POLL_INTERVAL);
        let mut access_check = tokio::time::interval(ACCESS_CHECK_INTERVAL);
        loop {
            tokio::select! {
                notification = notifications.recv() => match notification {
                    Some(notification) => self.handle_notification(notification).await,
                    None => return SessionEnd::ConnectionLost,
                },
                request = requests.recv() => match request {
                    None => return SessionEnd::Shutdown,
                    Some(Request::Connect(endpoint)) if endpoint != self.endpoint => return SessionEnd::SwitchTo(endpoint),
                    Some(Request::Connect(_)) => {}
                    Some(Request::Command(command)) => self.execute(command),
                    Some(Request::Query(query)) => self.answer(query),
                    Some(Request::LogIn { .. }) => log::debug!("login ignored: Moonraker already answers"),
                    Some(Request::LogOut) => return self.log_out().await,
                },
                _ = access_check.tick(), if self.access.is_authorized() => {
                    if !self.still_authorized().await {
                        log::info!("Moonraker withdrew the authorization of this display");
                        return SessionEnd::SwitchTo(self.endpoint.clone());
                    }
                }
                _ = spoolman_poll.tick(), if self.klippy_ready && self.components.spoolman => {
                    self.answer(Query::Spoolman);
                }
                Some(strips) = wled_answers.recv() => self.update_wled(strips),
                _ = wled_poll.tick(), if self.klippy_ready && !self.wled.is_empty() => {
                    let strips = self.fetch_wled().await;
                    self.update_wled(strips);
                }
                _ = klippy_poll.tick(), if !self.klippy_ready => {
                    if let Err(error) = self.refresh_klippy().await {
                        log::debug!("klippy not ready yet: {error}");
                    }
                }
            }
        }
    }

    /// Identifies with Moonraker, with the stored credential of this address if there is one. Refused means
    /// Moonraker wants a login.
    async fn introduce(&mut self) -> Result<(), AccessError> {
        let (access, credential) = match self.credentials.credential(&self.endpoint) {
            None => (Access::Open, None),
            Some(Credential::ApiKey { api_key }) => (Access::ApiKey, Some(("api_key", api_key))),
            Some(Credential::Login { username, refresh_token }) => {
                match access::refresh(&self.connection, &refresh_token).await {
                    Ok(token) => (Access::User(username), Some(("access_token", token))),
                    Err(AccessError::Refused(message)) => {
                        log::info!("the login of {username} is no longer valid: {message}");
                        (Access::Open, None)
                    }
                    Err(error) => return Err(error),
                }
            }
        };
        let presented = credential.as_ref().map(|(field, value)| (*field, value.as_str()));
        match access::identify(&self.connection, identity(), presented).await {
            Ok(()) => self.access = access,
            // A wrong key is refused even where Moonraker would trust this machine.
            Err(AccessError::Refused(message)) if presented.is_some() => {
                log::warn!("Moonraker refused the stored access of this display: {message}");
                access::identify(&self.connection, identity(), None).await?;
                self.access = Access::Open;
            }
            Err(error) => return Err(error),
        }
        Ok(())
    }

    /// Moonraker wants a login: answers the requests until one succeeds (`None`), or says how the session ends.
    async fn wait_for_login(
        &mut self,
        notifications: &mut mpsc::UnboundedReceiver<Notification>,
        requests: &mut mpsc::UnboundedReceiver<Request>,
    ) -> Option<SessionEnd> {
        publish_connection(self.outputs, &self.endpoint, ConnectionState::LoginRequired);
        loop {
            tokio::select! {
                // An unauthorized connection gets no notifications; this only notices its end.
                notification = notifications.recv() => if notification.is_none() {
                    return Some(SessionEnd::ConnectionLost);
                },
                request = requests.recv() => match request {
                    None => return Some(SessionEnd::Shutdown),
                    Some(Request::Connect(endpoint)) if endpoint != self.endpoint => {
                        return Some(SessionEnd::SwitchTo(endpoint));
                    }
                    Some(Request::LogIn { username, password }) => match self.log_in(&username, &password).await {
                        Ok(()) => return None,
                        Err(AccessError::Call(error)) => {
                            log::warn!("login failed: {error}");
                            return Some(SessionEnd::ConnectionLost);
                        }
                        Err(error) => self.outputs.event(Event::LoginFailed(error.to_string())),
                    },
                    Some(Request::Command(command)) => self.outputs.event(Event::CommandFailed {
                        command: command.describe(),
                        message: "Moonraker wants a login".to_owned(),
                    }),
                    Some(Request::LogOut) => forget_credential(self.credentials, &self.endpoint),
                    Some(Request::Connect(_) | Request::Query(_)) => {}
                },
            }
        }
    }

    /// Logs in on this connection and keeps the refresh token for the next ones.
    async fn log_in(&mut self, username: &str, password: &str) -> Result<(), AccessError> {
        let login = access::log_in(&self.connection, username, password).await?;
        let credential = Credential::Login { username: login.username.clone(), refresh_token: login.refresh_token };
        remember_credential(self.credentials, &self.endpoint, Some(credential));
        access::identify(&self.connection, identity(), Some(("access_token", &login.token))).await?;
        log::info!("logged in at Moonraker as {}", login.username);
        self.access = Access::User(login.username);
        Ok(())
    }

    /// Ends the login at Moonraker (an API key stays valid there), forgets the credential and connects again
    /// without it.
    async fn log_out(&mut self) -> SessionEnd {
        if let Access::User(username) = &self.access {
            match access::log_out(&self.connection).await {
                Ok(()) => log::info!("logged out {username} at Moonraker"),
                Err(error) => log::info!("logout at Moonraker failed: {error}"),
            }
        }
        forget_credential(self.credentials, &self.endpoint);
        SessionEnd::SwitchTo(self.endpoint.clone())
    }

    /// False once Moonraker refuses a request that needs authorization; other failures leave that open.
    async fn still_authorized(&self) -> bool {
        let answer = self.connection.call("server.info", None).await;
        !matches!(answer.map_err(AccessError::from), Err(AccessError::Refused(_)))
    }

    async fn refresh_klippy(&mut self) -> Result<(), CallError> {
        let info = server_info::parse_server_info(&self.connection.call("server.info", None).await?);
        log_fact("Moonraker", &info.moonraker_version);
        self.components = ServerComponents::from_names(info.components.iter().map(String::as_str));
        let state =
            if info.klippy_connected { KlippyState::parse(&info.klippy_state) } else { KlippyState::Disconnected };
        if state == KlippyState::Ready {
            self.subscribe().await?;
        } else {
            self.mark_klippy(state, "").await;
        }
        Ok(())
    }

    /// Detects the sources and subscribes to their objects.
    async fn subscribe(&mut self) -> Result<(), CallError> {
        let listing = self.connection.call("printer.objects.list", None).await?;
        let available: Vec<String> = listing
            .get("objects")
            .and_then(Value::as_array)
            .map(|names| names.iter().filter_map(Value::as_str).map(str::to_owned).collect())
            .unwrap_or_default();
        let mut sources = Sources::detect(&available, self.components.clone());
        let parameters = sources.subscription(&available);
        let result = self.connection.call("printer.objects.subscribe", Some(parameters)).await?;
        sources.apply_settings(result.pointer("/status/configfile/settings"));
        self.log_detected(&sources, &result).await;
        self.wled = self.fetch_wled().await;
        sources.wled_strips = self.wled.iter().map(WledStrip::object).collect();
        self.sources = sources;
        self.store.replace(result.get("status").unwrap_or(&Value::Null));
        // After a restart the configuration or the hardware may have changed: what was learned about the fans is gone.
        self.fan_watch = FanWatch::default();
        self.klippy_ready = true;
        self.publish();
        Ok(())
    }

    /// Notes the versions and the detected sources for the log, which a bug report needs.
    async fn log_detected(&self, sources: &Sources, subscription: &Value) {
        let klipper = self.connection.call("printer.info", None).await.ok();
        let version = |value: Option<&Value>, key: &str| {
            value.and_then(|value| value.get(key)).and_then(Value::as_str).unwrap_or_default().to_owned()
        };
        log_fact("Klipper", &version(klipper.as_ref(), "software_version"));
        let machine = subscription.pointer("/status/mmu_machine");
        log_fact("Happy Hare", &version(machine, "happy_hare_version"));
        log_fact("Detected", &sources.summary());
    }

    /// Moonraker's WLED strips; none without its `wled` component or when it cannot be asked.
    async fn fetch_wled(&self) -> Vec<WledStrip> {
        if !self.components.wled {
            return Vec::new();
        }
        match self.connection.call("machine.wled.strips", None).await {
            Ok(result) => wled::parse_strip_list(&result),
            Err(error) => {
                log::info!("WLED strips unavailable: {error}");
                Vec::new()
            }
        }
    }

    /// Takes the state of known strips; the strips themselves only change with Moonraker's configuration, which
    /// ends the connection.
    fn update_wled(&mut self, strips: Vec<WledStrip>) {
        let mut changed = false;
        for strip in strips {
            if let Some(known) = self.wled.iter_mut().find(|known| known.name == strip.name) {
                changed |= *known != strip;
                *known = strip;
            }
        }
        if changed {
            self.publish();
        }
    }

    /// Sets the Klippy state without a subscription, e.g. on shutdown or while Klipper starts.
    async fn mark_klippy(&mut self, state: KlippyState, message: &str) {
        self.klippy_ready = false;
        if state == KlippyState::Disconnected {
            self.store.clear();
        }
        // Without Klippy Moonraker has no `printer.info` (it answers "Method not found").
        let message = if message.is_empty() && state != KlippyState::Disconnected {
            self.klippy_message().await
        } else {
            message.to_owned()
        };
        self.store.merge(&json!({ "webhooks": { "state": state.as_str(), "state_message": message } }));
        self.publish();
    }

    /// Klipper reports the reason for errors and shutdown via `printer.info`.
    async fn klippy_message(&self) -> String {
        match self.connection.call("printer.info", None).await {
            Ok(info) => info.get("state_message").and_then(Value::as_str).unwrap_or_default().trim().to_owned(),
            Err(_) => String::new(),
        }
    }

    async fn handle_notification(&mut self, notification: Notification) {
        match notification.method.as_str() {
            "notify_status_update" => {
                if let Some(changes) = notification.params.get(0) {
                    self.store.merge(changes);
                    self.publish();
                }
            }
            "notify_klippy_ready" => {
                if let Err(error) = self.subscribe().await {
                    log::warn!("subscription after klippy ready failed: {error}");
                }
            }
            "notify_klippy_shutdown" => self.mark_klippy(KlippyState::Shutdown, "").await,
            "notify_klippy_disconnected" => self.mark_klippy(KlippyState::Disconnected, "").await,
            "notify_gcode_response" => {
                if let Some(message) = notification.params.get(0).and_then(Value::as_str) {
                    self.outputs.event(Event::ConsoleLine(console_line(ConsoleKind::Response, message)));
                }
            }
            "notify_power_changed" => self.answer(Query::PowerDevices),
            // Moonraker may record the end of a job after the state change was published.
            "notify_history_changed" => self.answer(Query::PrintHistory),
            "notify_active_spool_set" | "notify_spoolman_status_changed" => self.answer(Query::Spoolman),
            "notify_update_refreshed" => {
                for event in update_status_events(notification.params.get(0).unwrap_or(&Value::Null)) {
                    self.outputs.event(event);
                }
            }
            // The output of an update in progress, also one started by another client; the last one is `complete`.
            "notify_update_response" => {
                let complete = update_manager::is_response_complete(notification.params.get(0).unwrap_or(&Value::Null));
                self.outputs.event(Event::UpdateManagerBusy(!complete));
                if complete {
                    self.answer(Query::Updates);
                }
            }
            _ => {}
        }
    }

    /// The state with the heater targets that were sent but not yet reported back, the fans that do not turn and the
    /// height of the layer being printed.
    fn current_status(&mut self) -> PrinterStatus {
        let mut status = PrinterStatus::from_store(&self.store);
        let now = Instant::now();
        self.pending_targets.apply(&mut status.heaters, now);
        self.fan_watch.apply(&mut status.fans, now);
        self.height_watch.apply(&mut status.job, status.motion.gcode_position[2], now);
        status.leds.extend(self.wled.iter().map(WledStrip::led));
        status
    }

    /// Publishes the state, loads the print file's metadata when the job changes and the history when it has ended
    /// (when it ended and what it took).
    fn publish(&mut self) {
        let status = self.current_status();
        if status.job.filename != self.job_filename {
            self.job_filename.clone_from(&status.job.filename);
            if !self.job_filename.is_empty() {
                self.answer(Query::FileDetails { filename: self.job_filename.clone() });
            }
        }
        if status.job.state != self.job_state {
            self.job_state = status.job.state;
            if self.job_state.is_finished() {
                self.answer(Query::PrintHistory);
            }
        }
        if self.components.spoolman {
            let gate_spools = gate_spool_ids(&status);
            if self.spoolman_gate_spools.as_ref() != Some(&gate_spools) {
                self.spoolman_gate_spools = Some(gate_spools);
                self.answer(Query::Spoolman);
            }
        }
        self.outputs.snapshot.send_replace(Snapshot {
            endpoint: self.endpoint.display(),
            connection: ConnectionState::Connected,
            access: self.access.clone(),
            status,
            sources: self.sources.clone(),
        });
    }

    /// Carries out a command; an intent first becomes the commands this printer needs.
    fn execute(&mut self, command: Command) {
        let status = self.current_status();
        let description = command.describe();
        if command.moves_machine() && status.print_moving() {
            let message = "not possible while a print is moving the machine".to_owned();
            self.outputs.event(Event::CommandFailed { command: description, message });
            return;
        }
        let commands = match command.resolve(&self.sources, &status) {
            Ok(commands) => commands,
            Err(message) => {
                self.outputs.event(Event::CommandFailed { command: description, message });
                return;
            }
        };
        let mut targets_changed = false;
        for command in commands {
            targets_changed |= self.dispatch(command, &status);
        }
        if targets_changed {
            self.publish();
        }
    }

    /// Sends one concrete command; true if it set a heater target, which the snapshot shows at once.
    fn dispatch(&mut self, command: Command, status: &PrinterStatus) -> bool {
        if command.interrupts_print() && status.print_running() {
            self.outputs.event(Event::CommandFailed {
                command: command.describe(),
                message: "not possible while a print is running".to_owned(),
            });
            return false;
        }
        let Some(call) = command.to_call(&status.motion) else { return false };
        // Every G-code script goes to the console, as Moonraker stores it in the console history: typed commands,
        // macros and the scripts behind the buttons alike.
        if let Some(script) = gcode_script(&call) {
            self.outputs.event(Event::ConsoleLine(console_line(ConsoleKind::Command, script)));
        }
        let target_set = match &command {
            Command::SetHeaterTarget { heater, target } => {
                self.pending_targets.request(heater, target.max(0.0), Instant::now());
                true
            }
            _ => false,
        };
        let spools_change = command.changes_spools();
        // Busy from the start: Moonraker's own `busy` and notifications only follow once it has begun.
        let update_manager_call = matches!(command, Command::RefreshUpdates | Command::Upgrade { .. });
        if update_manager_call {
            self.outputs.event(Event::UpdateManagerBusy(true));
        }
        let wled_answers = matches!(command, Command::SetWledStrip { .. }).then(|| self.wled_answers.clone());
        let label = running_label(&call, &command);
        let spool_events = self.outputs.events.clone();
        let running_events = self.outputs.events.clone();
        let events = self.outputs.events.clone();
        let failed = move |error: CallError| {
            let message = match error {
                CallError::Rpc(rpc) => rpc.message,
                CallError::Closed => "connection to Moonraker closed".to_owned(),
            };
            let _ = events.send(Event::CommandFailed { command: command.describe(), message });
        };
        // Sent here, in the order of the commands; a task per command could overtake an earlier one.
        match self.connection.start_call(call.method, call.params) {
            Ok(pending) => {
                tokio::spawn(async move {
                    let answer = answer_or_report_running(pending, label, &running_events).await;
                    if update_manager_call {
                        let _ = running_events.send(Event::UpdateManagerBusy(false));
                    }
                    match answer {
                        Ok(_) if spools_change => {
                            let _ = spool_events.send(Event::SpoolmanChanged);
                        }
                        Ok(answer) => {
                            if let (Some(sender), Some(strips)) = (wled_answers, answer.as_object()) {
                                let _ = sender.send(wled::parse_strips(strips));
                            }
                        }
                        Err(error) => failed(error),
                    }
                });
            }
            Err(error) => failed(error),
        }
        target_set
    }

    fn answer(&self, query: Query) {
        if query == Query::PrintHistory && !self.components.history {
            log::debug!("no job history: Moonraker has no history component");
            return;
        }
        if matches!(query, Query::Spoolman | Query::SpoolList) && !self.components.spoolman {
            log::debug!("no Spoolman: Moonraker has no spoolman component");
            return;
        }
        let connection = self.connection.clone();
        let downloads = Downloads {
            http: self.http.clone(),
            endpoint: self.endpoint.clone(),
            authorized: self.access.is_authorized(),
        };
        let events = self.outputs.events.clone();
        let gate_spools = self.spoolman_gate_spools.clone().unwrap_or_default();
        let permits = matches!(query, Query::FileDetails { .. }).then(|| self.file_details.clone());
        tokio::spawn(async move {
            // The timeout starts once it is this query's turn.
            let _permit = match permits {
                Some(permits) => permits.acquire_owned().await.ok(),
                None => None,
            };
            let fetched =
                tokio::time::timeout(QUERY_TIMEOUT, fetch(&connection, &downloads, &query, &gate_spools)).await;
            match fetched {
                Ok(Ok(results)) => results.into_iter().for_each(|event| {
                    let _ = events.send(event);
                }),
                Ok(Err(error)) => log::info!("query {query:?} failed: {error}"),
                Err(_) => log::info!("query {query:?} timed out"),
            }
        });
    }
}

/// Files over HTTP (thumbnails); an authorized display shows a one-time token from its connection with each.
struct Downloads {
    http: reqwest::Client,
    endpoint: Endpoint,
    authorized: bool,
}

impl Downloads {
    async fn get(&self, connection: &Connection, path: &str) -> Result<Vec<u8>, String> {
        let mut url = self.endpoint.http_url(path);
        if self.authorized {
            let token = access::oneshot_token(connection).await.map_err(|error| error.to_string())?;
            url = format!("{url}?token={token}");
        }
        let response = self.http.get(url).send().await.map_err(|error| error.to_string())?;
        let response = response.error_for_status().map_err(|error| error.to_string())?;
        Ok(response.bytes().await.map_err(|error| error.to_string())?.to_vec())
    }
}

async fn fetch(
    connection: &Connection,
    downloads: &Downloads,
    query: &Query,
    gate_spools: &[u32],
) -> Result<Vec<Event>, String> {
    let call = |method: &'static str, params: Option<Value>| async move {
        connection.call(method, params).await.map_err(|error| error.to_string())
    };
    let events = match query {
        Query::Files => {
            let listing = call("server.files.list", Some(json!({ "root": "gcodes" }))).await?;
            vec![Event::Files(files::parse_file_list(&listing))]
        }
        Query::FileDetails { filename } => {
            let result = call("server.files.metadata", Some(json!({ "filename": filename }))).await?;
            let metadata = files::parse_metadata(&result).ok_or("unreadable metadata")?;
            let mut events = Vec::new();
            if let Some(path) = metadata.thumbnail_path() {
                match downloads.get(connection, &path).await {
                    Ok(data) => match ThumbnailImage::decode(&data) {
                        Some(image) => events.push(Event::Thumbnail { filename: filename.clone(), image }),
                        None => log::info!("thumbnail of {filename} cannot be decoded"),
                    },
                    Err(error) => log::info!("thumbnail of {filename} unavailable: {error}"),
                }
            }
            events.insert(0, Event::FileMetadata(Box::new(FileMetadata { filename: filename.clone(), ..metadata })));
            events
        }
        Query::PrintHistory => {
            let jobs = call("server.history.list", Some(json!({ "limit": PRINT_HISTORY_LIMIT }))).await?;
            vec![Event::PrintHistory(history::parse_print_history(&jobs))]
        }
        Query::Spoolman => vec![Event::Spoolman(Box::new(fetch_spoolman(connection, gate_spools).await?))],
        Query::SpoolList => {
            let query = format!("sort=last_used%3Adesc&limit={SPOOL_LIST_LIMIT}&allow_archived=false");
            let spools = spoolman_get(connection, "/v1/spool", Some(&query)).await?;
            vec![Event::SpoolList(spoolman::parse_spool_list(&spools))]
        }
        Query::PowerDevices => {
            vec![Event::PowerDevices(power::parse_power_devices(&call("machine.device_power.devices", None).await?))]
        }
        Query::Updates => update_status_events(&call("machine.update.status", None).await?),
        Query::ConsoleHistory => vec![Event::ConsoleHistory(gcode_store::parse_gcode_store(
            &call("server.gcode_store", Some(json!({ "count": CONSOLE_HISTORY }))).await?,
        ))],
        Query::HostNetwork => {
            vec![Event::HostNetwork(machine::parse_host_network(&call("machine.system_info", None).await?))]
        }
        Query::TemperatureHistory => {
            let store = call("server.temperature_store", Some(json!({ "include_monitors": false }))).await?;
            vec![Event::TemperatureHistory(temperature_store::parse_temperature_store(&store))]
        }
    };
    Ok(events)
}

/// The versions and whether the update manager is busy, from `machine.update.status` or `notify_update_refreshed`.
fn update_status_events(status: &Value) -> Vec<Event> {
    vec![
        Event::Updates(update_manager::parse_update_status(status)),
        Event::UpdateManagerBusy(update_manager::is_busy(status)),
    ]
}

/// Waits for the answer of a call; if it takes longer than `LONG_COMMAND`, reports the command as running until then.
async fn answer_or_report_running(
    pending: PendingCall,
    label: String,
    events: &mpsc::UnboundedSender<Event>,
) -> Result<Value, CallError> {
    let response = pending.response();
    tokio::pin!(response);
    tokio::select! {
        answer = &mut response => answer,
        () = tokio::time::sleep(LONG_COMMAND) => {
            let id = NEXT_COMMAND_ID.fetch_add(1, Ordering::Relaxed);
            let _ = events.send(Event::CommandRunning { id, command: label });
            let answer = response.await;
            let _ = events.send(Event::CommandFinished { id });
            answer
        }
    }
}

/// What a running command is called in the UI: the first line of a G-code script, as in the console, otherwise
/// the command's short name.
fn running_label(call: &Call, command: &Command) -> String {
    match gcode_script(call).and_then(|script| script.lines().next()) {
        Some(line) => line.to_owned(),
        None => command.describe(),
    }
}

/// The script of a call that runs G-code.
/// A fact about the printer for the log header; nothing when the printer did not report it.
fn log_fact(name: &str, value: &str) {
    if !value.is_empty() {
        log::info!(target: crate::HEADER_TARGET, "{name}: {value}");
    }
}

fn gcode_script(call: &Call) -> Option<&str> {
    if call.method != "printer.gcode.script" {
        return None;
    }
    call.params.as_ref().and_then(|params| params.get("script")).and_then(Value::as_str)
}

fn console_line(kind: ConsoleKind, message: &str) -> ConsoleLine {
    let time = SystemTime::now().duration_since(UNIX_EPOCH).map_or(0.0, |elapsed| elapsed.as_secs_f64());
    ConsoleLine { kind, message: message.to_owned(), time }
}

/// The spools of the gates, without duplicates.
fn gate_spool_ids(status: &PrinterStatus) -> Vec<u32> {
    let mut ids: Vec<u32> =
        status.happy_hare().iter().flat_map(|mmu| mmu.gates.iter()).filter_map(|gate| gate.spool_id).collect();
    ids.sort_unstable();
    ids.dedup();
    ids
}

/// Asks Spoolman through Moonraker; Spoolman has no API of its own for the touch screen.
async fn spoolman_get(connection: &Connection, path: &str, query: Option<&str>) -> Result<Value, String> {
    let parameters = json!({ "request_method": "GET", "path": path, "query": query, "use_v2_response": true });
    let result = connection.call("server.spoolman.proxy", Some(parameters)).await.map_err(|error| error.to_string())?;
    spoolman::proxy_response(&result)
}

/// The state of Spoolman and the spools the printer needs now: those of the gates and the active one. A spool that
/// cannot be read is left out; the rest still arrives.
async fn fetch_spoolman(connection: &Connection, gate_spools: &[u32]) -> Result<SpoolmanState, String> {
    let status = connection.call("server.spoolman.status", None).await.map_err(|error| error.to_string())?;
    let (connected, pending_reports, active_spool_id) = spoolman::parse_status(&status);
    let mut state = SpoolmanState { connected, pending_reports, active_spool_id, ..SpoolmanState::default() };
    if !connected {
        return Ok(state);
    }
    let mut ids = gate_spools.to_vec();
    ids.extend(active_spool_id);
    ids.sort_unstable();
    ids.dedup();
    for id in ids {
        match spoolman_get(connection, &format!("/v1/spool/{id}"), None).await {
            Ok(spool) => state.spools.extend(Spool::from_json(&spool).map(|spool| (id, spool))),
            Err(error) => log::info!("spool {id} cannot be read: {error}"),
        }
    }
    Ok(state)
}
