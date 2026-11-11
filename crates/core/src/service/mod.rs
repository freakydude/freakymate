//! Background service: keeps the connection to Moonraker, maintains the state and executes commands.
//!
//! The UI talks to the printer exclusively through [`PrinterService`]. State arrives as a
//! [`Snapshot`] over a `watch` channel, one-off results as [`Event`].

mod dispatch;
mod queries;
mod session;

use crate::commands::{Command, CommandError};
use crate::credentials::CredentialStore;
use crate::domain::machine_tree::MachineTree;
pub use crate::moonraker::ConnectionState;
use crate::moonraker::Endpoint;
use crate::sources::moonraker_server::files::{FileEntry, FileMetadata, ThumbnailImage};
use crate::sources::moonraker_server::gcode_store::ConsoleLine;
use crate::sources::moonraker_server::history::PrintHistory;
use crate::sources::moonraker_server::machine::HostInterface;
use crate::sources::moonraker_server::power::PowerDevice;
use crate::sources::moonraker_server::temperature_store::TemperatureSeries;
use crate::sources::moonraker_server::update_manager::UpdateInfo;
use crate::sources::spoolman::{Spool, SpoolmanState};
use crate::sources::{PrinterStatus, Sources};
use std::collections::HashMap;
use std::sync::atomic::AtomicU64;
use std::time::Duration;
use tokio::sync::{mpsc, watch};

const RECONNECT_DELAY: Duration = Duration::from_secs(2);
const KLIPPY_POLL_INTERVAL: Duration = Duration::from_secs(2);
const QUERY_TIMEOUT: Duration = Duration::from_secs(15);
/// File details (metadata and thumbnail) fetched at the same time; the file list asks for up to 40 at once.
const PARALLEL_FILE_DETAILS: usize = 4;
/// Moonraker reports the active spool and the connection itself; the weights of the spools it does not, so they are read again now and then.
const SPOOLMAN_POLL_INTERVAL: Duration = Duration::from_secs(60);
/// Moonraker does not report WLED changes, and Mainsail or a macro may switch a strip too.
const WLED_POLL_INTERVAL: Duration = Duration::from_secs(5);
const CLIENT_NAME: &str = "freakymate";
/// A command whose answer takes longer is shown as running (busy indicator).
const LONG_COMMAND: Duration = Duration::from_secs(1);
/// Moonraker sends nothing more to a connection that lost its authorization (the same user logged out elsewhere),
/// not even the logout; so an authorized session asks now and then whether it still may.
const ACCESS_CHECK_INTERVAL: Duration = Duration::from_secs(30);
/// Ids of running commands; unique across reconnections, where answers of an old connection may still arrive.
static NEXT_COMMAND_ID: AtomicU64 = AtomicU64::new(0);

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
    /// The machine from carriage to lane, built from `status` with every update.
    pub tree: MachineTree,
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
        command: Command,
        error: CommandError,
    },
    /// A command sent from here waits longer than `LONG_COMMAND` for its answer (homing, leveling, an update, ...).
    CommandRunning {
        id: u64,
        command: Command,
        /// The first line of the G-code script the command runs, as the console shows it.
        script_line: Option<String>,
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
        runtime.spawn(session::run(endpoint, credentials, requests_rx, outputs));
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
