//! Background task that owns the network backend, applies commands and publishes the state.

#[cfg(feature = "dev-mode")]
use super::simulated::Simulated;
use super::{Availability, NetworkCommand, NetworkError, NetworkNotice, NetworkState};
use std::time::Duration;
use tokio::sync::{mpsc, watch};

/// NetworkManager does not push changes to us; while the UI shows the networks they are read this often.
const REFRESH_INTERVAL: Duration = Duration::from_secs(5);

/// Which implementation manages the networks.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum NetworkBackend {
    /// NetworkManager of this host (Linux only).
    #[default]
    System,
    /// In-memory networks for development and screenshots.
    #[cfg(feature = "dev-mode")]
    Simulated,
}

impl NetworkBackend {
    /// Simulated networks count as local, so they can be tried against any Moonraker.
    pub fn is_simulated(self) -> bool {
        match self {
            Self::System => false,
            #[cfg(feature = "dev-mode")]
            Self::Simulated => true,
        }
    }
}

/// Handle for the UI; cheap to clone.
#[derive(Clone)]
pub struct NetworkService {
    requests: mpsc::UnboundedSender<Request>,
}

enum Request {
    Command(NetworkCommand),
    /// Whether the UI shows the networks and wants regular refreshes.
    Watch(bool),
}

pub struct NetworkChannels {
    pub states: watch::Receiver<NetworkState>,
    pub notices: mpsc::UnboundedReceiver<NetworkNotice>,
}

impl NetworkService {
    pub fn start(runtime: &tokio::runtime::Handle, backend: NetworkBackend) -> (Self, NetworkChannels) {
        let (requests, requests_rx) = mpsc::unbounded_channel();
        let (state_tx, states) = watch::channel(NetworkState::default());
        let (notice_tx, notices) = mpsc::unbounded_channel();
        runtime.spawn(run(backend, requests_rx, Outputs { state: state_tx, notices: notice_tx }));
        (Self { requests }, NetworkChannels { states, notices })
    }

    pub fn send(&self, command: NetworkCommand) {
        let _ = self.requests.send(Request::Command(command));
    }

    /// Reads the state at once when `visible`, then regularly until hidden again.
    pub fn watch(&self, visible: bool) {
        let _ = self.requests.send(Request::Watch(visible));
    }
}

/// Without Linux and without `dev-mode` this enum has no variant: `open` then always reports unavailable.
enum Backend {
    #[cfg(target_os = "linux")]
    NetworkManager(super::networkmanager::NetworkManager),
    #[cfg(feature = "dev-mode")]
    Simulated(Simulated),
}

impl Backend {
    async fn open(choice: NetworkBackend) -> Result<Self, NetworkError> {
        match choice {
            #[cfg(feature = "dev-mode")]
            NetworkBackend::Simulated => Ok(Self::Simulated(Simulated::default())),
            #[cfg(target_os = "linux")]
            NetworkBackend::System => super::networkmanager::NetworkManager::connect().await.map(Self::NetworkManager),
            #[cfg(not(target_os = "linux"))]
            NetworkBackend::System => {
                Err(NetworkError::Unavailable("only supported on Linux with NetworkManager".to_owned()))
            }
        }
    }

    // `match *self` instead of `match self`: it also compiles when the enum has no variant.
    async fn read(&self) -> Result<NetworkState, NetworkError> {
        match *self {
            #[cfg(target_os = "linux")]
            Self::NetworkManager(ref manager) => manager.read().await,
            #[cfg(feature = "dev-mode")]
            Self::Simulated(ref simulated) => Ok(simulated.read()),
        }
    }

    #[cfg_attr(not(any(target_os = "linux", feature = "dev-mode")), allow(unused_variables))]
    async fn apply(&mut self, command: &NetworkCommand) -> Result<(), NetworkError> {
        match *self {
            #[cfg(target_os = "linux")]
            Self::NetworkManager(ref manager) => manager.apply(command).await,
            #[cfg(feature = "dev-mode")]
            Self::Simulated(ref mut simulated) => simulated.apply(command).await,
        }
    }
}

struct Outputs {
    state: watch::Sender<NetworkState>,
    notices: mpsc::UnboundedSender<NetworkNotice>,
}

impl Outputs {
    fn notice(&self, message: String, error: bool) {
        let _ = self.notices.send(NetworkNotice { message, error });
    }

    fn unavailable(&self, error: &NetworkError) {
        let reason = match error {
            NetworkError::Unavailable(reason) => reason.clone(),
            other => other.to_string(),
        };
        self.state
            .send_replace(NetworkState { availability: Availability::Unavailable(reason), ..NetworkState::default() });
    }

    async fn refresh(&self, backend: &Backend) {
        match backend.read().await {
            Ok(state) => {
                self.state.send_replace(state);
            }
            Err(error) => {
                log::warn!("reading network state failed: {error}");
                self.unavailable(&error);
            }
        }
    }

    async fn execute(&self, backend: &mut Backend, command: NetworkCommand) {
        if let Some(busy) = busy_text(&command) {
            self.state.send_modify(|state| state.busy = Some(busy));
        }
        match backend.apply(&command).await {
            Ok(()) => {
                if let Some(message) = success_text(&command) {
                    self.notice(message, false);
                }
            }
            Err(error) => {
                // The error names the network, so it only goes to the verbose log.
                log::info!("network command {} failed", describe(&command));
                log::debug!("network command {} failed: {error}", describe_in_full(&command));
                self.notice(error.to_string(), true);
            }
        }
        self.refresh(backend).await;
    }
}

async fn run(choice: NetworkBackend, mut requests: mpsc::UnboundedReceiver<Request>, outputs: Outputs) {
    let mut backend = match Backend::open(choice).await {
        Ok(backend) => backend,
        Err(error) => {
            log::info!("{error}");
            outputs.unavailable(&error);
            // Keep the channel open so the UI can send commands without effect.
            while requests.recv().await.is_some() {}
            return;
        }
    };
    // One read decides whether the UI offers management at all.
    outputs.refresh(&backend).await;
    let mut watching = false;
    let mut refresh = tokio::time::interval(REFRESH_INTERVAL);
    loop {
        tokio::select! {
            request = requests.recv() => match request {
                Some(Request::Command(command)) => outputs.execute(&mut backend, command).await,
                Some(Request::Watch(visible)) => {
                    watching = visible;
                    if visible {
                        refresh.reset();
                        outputs.refresh(&backend).await;
                    }
                }
                None => return,
            },
            _ = refresh.tick(), if watching => outputs.refresh(&backend).await,
        }
    }
}

fn busy_text(command: &NetworkCommand) -> Option<String> {
    match command {
        NetworkCommand::Connect { ssid, .. } => Some(format!("Connecting to {ssid}")),
        NetworkCommand::Scan => Some("Searching for networks".to_owned()),
        _ => None,
    }
}

fn success_text(command: &NetworkCommand) -> Option<String> {
    match command {
        NetworkCommand::Refresh | NetworkCommand::Scan => None,
        NetworkCommand::Connect { ssid, .. } => Some(format!("Connected to {ssid}")),
        NetworkCommand::Disconnect => Some("Wi-Fi disconnected".to_owned()),
        NetworkCommand::Forget { ssid } => Some(format!("Removed {ssid}")),
        NetworkCommand::SetWifiEnabled(true) => Some("Wi-Fi switched on".to_owned()),
        NetworkCommand::SetWifiEnabled(false) => Some("Wi-Fi switched off".to_owned()),
        NetworkCommand::SetWired { interface, connected: true } => Some(format!("{interface} connected")),
        NetworkCommand::SetWired { interface, connected: false } => Some(format!("{interface} disconnected")),
    }
}

/// Description for the log without secrets.
/// The command for the normal log, without the name of a network.
fn describe(command: &NetworkCommand) -> String {
    match command {
        NetworkCommand::Connect { hidden, .. } => format!("connect (hidden network: {hidden})"),
        NetworkCommand::Forget { .. } => "forget".to_owned(),
        other => format!("{other:?}"),
    }
}

/// The command for the verbose log, with the name of a network but never its password.
fn describe_in_full(command: &NetworkCommand) -> String {
    match command {
        NetworkCommand::Connect { ssid, hidden, .. } => format!("connect to '{ssid}' (hidden: {hidden})"),
        other => format!("{other:?}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(feature = "dev-mode")]
    #[tokio::test]
    async fn simulated_service_publishes_state_and_notices() {
        let runtime = tokio::runtime::Handle::current();
        let (service, mut channels) = NetworkService::start(&runtime, NetworkBackend::Simulated);
        channels.states.wait_for(|state| state.availability == Availability::Managed).await.unwrap();

        service.send(NetworkCommand::Forget { ssid: "Garage".into() });
        let notice = channels.notices.recv().await.unwrap();
        assert_eq!(notice, NetworkNotice { message: "Removed Garage".into(), error: false });
        channels
            .states
            .wait_for(|state| !state.networks.iter().any(|network| network.ssid == "Garage" && network.known))
            .await
            .unwrap();

        service.send(NetworkCommand::Forget { ssid: "Unknown".into() });
        assert!(channels.notices.recv().await.unwrap().error);
    }

    #[test]
    fn log_description_hides_the_password() {
        let command =
            NetworkCommand::Connect { ssid: "Home".into(), password: Some("secret-pw".into()), hidden: false };
        assert!(!describe(&command).contains("secret-pw"));
    }

    #[cfg(not(target_os = "linux"))]
    #[tokio::test]
    async fn system_backend_is_unavailable_outside_linux() {
        let (_service, mut channels) =
            NetworkService::start(&tokio::runtime::Handle::current(), NetworkBackend::System);
        let state =
            channels.states.wait_for(|state| state.availability != Availability::Checking).await.unwrap().clone();
        assert!(matches!(state.availability, Availability::Unavailable(_)));
    }
}
