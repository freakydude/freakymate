//! One connection to Moonraker: reconnecting, handshake and login, subscription and the loop that serves requests.

use super::dispatch::console_line;
use super::queries::{gate_spool_ids, update_status_events};
use super::Snapshot;
use super::{
    Access, Event, Outputs, Query, Request, SessionEnd, ACCESS_CHECK_INTERVAL, CLIENT_NAME, KLIPPY_POLL_INTERVAL,
    PARALLEL_FILE_DETAILS, RECONNECT_DELAY, SPOOLMAN_POLL_INTERVAL, WLED_POLL_INTERVAL,
};
use crate::commands::CommandError;
use crate::credentials::{Credential, CredentialStore};
use crate::domain::machine_tree::MachineTree;
use crate::moonraker::access::{self, AccessError};
use crate::moonraker::{CallError, Connection, ConnectionState, Endpoint, Notification};
use crate::sources::happy_hare::stale_gate_map;
use crate::sources::klipper::{self, FanWatch, HeightWatch, KlippyState, PendingTargets, PrintState};
use crate::sources::moonraker_server::gcode_store::ConsoleKind;
use crate::sources::moonraker_server::update_manager;
use crate::sources::moonraker_server::wled::{self, WledStrip};
use crate::sources::moonraker_server::{server_info, ServerComponents};
use crate::sources::{ObjectStore, PrinterStatus, Sources};
use serde_json::{json, Value};
use std::sync::Arc;
use std::time::Instant;
use tokio::sync::{mpsc, Semaphore};

pub(super) async fn run(
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
                    command,
                    error: CommandError::NotConnected,
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

pub(super) struct Session<'a> {
    pub(super) connection: Connection,
    pub(super) http: reqwest::Client,
    pub(super) endpoint: Endpoint,
    pub(super) credentials: &'a CredentialStore,
    pub(super) access: Access,
    pub(super) outputs: &'a Outputs,
    pub(super) store: ObjectStore,
    pub(super) sources: Sources,
    /// From `server.info`, known before Klipper is ready; part of [`Sources`] once subscribed.
    pub(super) components: ServerComponents,
    pub(super) klippy_ready: bool,
    pub(super) job_filename: String,
    pub(super) job_state: PrintState,
    /// Targets sent but not yet reported back; the snapshot shows them.
    pub(super) pending_targets: PendingTargets,
    /// What each fan's tachometer has measured since Klipper started; the snapshot marks the stalled ones.
    pub(super) fan_watch: FanWatch,
    pub(super) height_watch: HeightWatch,
    /// The spools of the gates at the last look; `None` until Spoolman was asked once.
    pub(super) spoolman_gate_spools: Option<Vec<u32>>,
    /// Limits the file details in flight, so a long file list does not flood Moonraker.
    pub(super) file_details: Arc<Semaphore>,
    /// Moonraker's WLED strips; part of the snapshot's lights.
    pub(super) wled: Vec<WledStrip>,
    /// The answers of the commands to a WLED strip, which carry its new state.
    pub(super) wled_answers: mpsc::UnboundedSender<Vec<WledStrip>>,
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
                    self.poll_wled();
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
                        command,
                        error: CommandError::LoginRequired,
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
        sources.klipper.tool_commands = self.fetch_tool_commands().await;
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

    /// The `T<n>` commands Klipper knows, from its command list; `None` when it cannot be asked.
    async fn fetch_tool_commands(&self) -> Option<Vec<usize>> {
        let help = self.connection.call("printer.gcode.help", None).await.ok()?;
        Some(klipper::tool_commands(&help))
    }

    /// Moonraker's WLED strips; none without its `wled` component or when it cannot be asked.
    async fn fetch_wled(&self) -> Vec<WledStrip> {
        if !self.components.wled {
            return Vec::new();
        }
        fetch_wled_strips(&self.connection).await
    }

    /// Asks for the strips in a task, so a slow answer does not hold up the loop; the answer arrives like that of a
    /// command to a strip.
    fn poll_wled(&self) {
        let connection = self.connection.clone();
        let answers = self.wled_answers.clone();
        tokio::spawn(async move {
            let strips = fetch_wled_strips(&connection).await;
            let _ = answers.send(strips);
        });
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

    /// Workaround for gate map changes Happy Hare makes in place and Klipper never sends; see `stale_gate_map`.
    async fn requery_stale_gate_map(&mut self, changes: &Value) {
        let Some(parameters) = stale_gate_map::query_after(changes) else { return };
        match self.connection.call("printer.objects.query", Some(parameters)).await {
            Ok(result) => self.store.merge(result.get("status").unwrap_or(&Value::Null)),
            Err(error) => log::warn!("gate map query failed: {error}"),
        }
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
                    self.requery_stale_gate_map(changes).await;
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
            // Files added, removed or renamed, from here or another client.
            "notify_filelist_changed" => self.answer(Query::Files),
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
    pub(super) fn current_status(&mut self) -> PrinterStatus {
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
    pub(super) fn publish(&mut self) {
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
            tree: MachineTree::build(&status),
            status,
            sources: self.sources.clone(),
        });
    }
}

/// A fact about the printer for the log header; nothing when the printer did not report it.
fn log_fact(name: &str, value: &str) {
    if !value.is_empty() {
        log::info!(target: crate::HEADER_TARGET, "{name}: {value}");
    }
}

async fn fetch_wled_strips(connection: &Connection) -> Vec<WledStrip> {
    let call = wled::strips_call();
    match connection.call(call.method, call.params).await {
        Ok(result) => wled::parse_strip_list(&result),
        Err(error) => {
            log::info!("WLED strips unavailable: {error}");
            Vec::new()
        }
    }
}
