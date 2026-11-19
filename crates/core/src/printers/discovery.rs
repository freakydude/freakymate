//! Background task that, while the Printers page is shown, looks for Moonraker servers announced over zeroconf and
//! checks whether the saved printers answer.
//!
//! Moonraker announces itself only with `[zeroconf]` in `moonraker.conf`: service type `_moonraker._tcp.local.`, the
//! HTTP port, and the TXT keys `uuid`, `https_port`, `version` and `route_prefix`
//! (`moonraker/components/zeroconf.py`, `ZC_SERVICE_TYPE` and `zc_service_props`).

use super::{FoundPrinter, ReachableAddresses};
use crate::moonraker::Endpoint;
use mdns_sd::{ServiceDaemon, ServiceEvent};
use std::collections::BTreeMap;
use std::net::Ipv4Addr;
use std::time::Duration;
use tokio::sync::{mpsc, watch};

const SERVICE_TYPE: &str = "_moonraker._tcp.local.";
const CHECK_INTERVAL: Duration = Duration::from_secs(10);
/// A printer on the local network answers within milliseconds; longer means it is off or gone.
const CHECK_TIMEOUT: Duration = Duration::from_secs(2);

/// How printers are found and checked.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum DiscoveryBackend {
    /// zeroconf on the local network, and a TCP connection to each saved address.
    #[default]
    Network,
    /// One found printer that never answers; only addresses on this machine count as reachable (snapshots).
    #[cfg(feature = "dev-mode")]
    Simulated,
}

/// What the Printers page shows besides the saved printers; empty while the page is hidden.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DiscoveryState {
    /// Announced printers, by name.
    pub found: Vec<FoundPrinter>,
    /// Whether each saved address answered at the last check; missing until checked.
    pub reachable: ReachableAddresses,
}

/// Handle for the UI; cheap to clone.
#[derive(Clone)]
pub struct DiscoveryService {
    requests: mpsc::UnboundedSender<Request>,
}

enum Request {
    Watch(bool),
    Check(Vec<String>),
}

impl DiscoveryService {
    pub fn start(
        runtime: &tokio::runtime::Handle,
        backend: DiscoveryBackend,
    ) -> (Self, watch::Receiver<DiscoveryState>) {
        let (requests, requests_rx) = mpsc::unbounded_channel();
        let (state_tx, states) = watch::channel(DiscoveryState::default());
        runtime.spawn(run(backend, requests_rx, state_tx));
        (Self { requests }, states)
    }

    /// Searches and checks while `visible`; hidden, the state empties.
    pub fn watch(&self, visible: bool) {
        let _ = self.requests.send(Request::Watch(visible));
    }

    /// The saved addresses to check, as [`Endpoint::display`] writes them; checked at once while watching.
    pub fn check(&self, addresses: Vec<String>) {
        let _ = self.requests.send(Request::Check(addresses));
    }
}

async fn run(
    backend: DiscoveryBackend,
    mut requests: mpsc::UnboundedReceiver<Request>,
    state: watch::Sender<DiscoveryState>,
) {
    let mut addresses = Vec::new();
    let mut search: Option<Search> = None;
    let mut ticker = tokio::time::interval(CHECK_INTERVAL);
    ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    loop {
        let events = search.as_ref().and_then(|search| search.events.clone());
        tokio::select! {
            request = requests.recv() => match request {
                None => break,
                Some(Request::Watch(true)) => {
                    if search.is_none() {
                        let started = Search::start(backend);
                        state.send_modify(|state| state.found = started.found_list());
                        search = Some(started);
                        ticker.reset_immediately();
                    }
                }
                Some(Request::Watch(false)) => {
                    search = None;
                    state.send_replace(DiscoveryState::default());
                }
                Some(Request::Check(list)) => {
                    addresses = list;
                    if search.is_some() {
                        ticker.reset_immediately();
                    }
                }
            },
            _ = ticker.tick(), if search.is_some() => {
                let reachable = check_all(backend, &addresses).await;
                state.send_modify(|state| state.reachable = reachable);
            }
            event = next_event(events) => {
                let Some(search) = search.as_mut() else { continue };
                match event {
                    Some(event) => {
                        if search.apply(event) {
                            state.send_modify(|state| state.found = search.found_list());
                        }
                    }
                    // The daemon stopped; checks go on without it.
                    None => search.events = None,
                }
            }
        }
    }
}

async fn next_event(events: Option<mdns_sd::Receiver<ServiceEvent>>) -> Option<ServiceEvent> {
    match events {
        Some(events) => events.recv_async().await.ok(),
        None => std::future::pending().await,
    }
}

/// A running zeroconf search; dropping it stops the daemon.
struct Search {
    daemon: Option<ServiceDaemon>,
    events: Option<mdns_sd::Receiver<ServiceEvent>>,
    /// By the full service name, which removals refer to.
    found: BTreeMap<String, FoundPrinter>,
}

impl Search {
    fn start(backend: DiscoveryBackend) -> Self {
        match backend {
            DiscoveryBackend::Network => Self::on_network(),
            #[cfg(feature = "dev-mode")]
            DiscoveryBackend::Simulated => Self::simulated(),
        }
    }

    fn on_network() -> Self {
        let started =
            ServiceDaemon::new().and_then(|daemon| daemon.browse(SERVICE_TYPE).map(|events| (daemon, events)));
        match started {
            Ok((daemon, events)) => Self { daemon: Some(daemon), events: Some(events), found: BTreeMap::new() },
            Err(error) => {
                log::warn!("printer search over zeroconf unavailable: {error}");
                Self { daemon: None, events: None, found: BTreeMap::new() }
            }
        }
    }

    #[cfg(feature = "dev-mode")]
    fn simulated() -> Self {
        // TEST-NET-1 (RFC 5737): never reachable, so the row stays "found" in every snapshot.
        let printer = announced("sidewinder.local.", 7125, &[Ipv4Addr::new(192, 0, 2, 7)], Some("simulated"));
        let found = printer.into_iter().map(|printer| ("Moonraker @ sidewinder".to_owned(), printer)).collect();
        Self { daemon: None, events: None, found }
    }

    /// Returns whether the list of found printers changed.
    fn apply(&mut self, event: ServiceEvent) -> bool {
        match event {
            ServiceEvent::ServiceResolved(service) => {
                let addresses: Vec<Ipv4Addr> = service.get_addresses_v4().into_iter().collect();
                let uuid = service.get_property_val_str("uuid");
                match announced(&service.host, service.port, &addresses, uuid) {
                    Some(printer) => self.found.insert(service.fullname.clone(), printer.clone()) != Some(printer),
                    None => false,
                }
            }
            ServiceEvent::ServiceRemoved(_, fullname) => self.found.remove(&fullname).is_some(),
            _ => false,
        }
    }

    fn found_list(&self) -> Vec<FoundPrinter> {
        let mut list: Vec<FoundPrinter> = self.found.values().cloned().collect();
        list.sort_by_cached_key(|printer| printer.name.to_lowercase());
        list
    }
}

impl Drop for Search {
    fn drop(&mut self) {
        if let Some(daemon) = self.daemon.take() {
            let _ = daemon.shutdown();
        }
    }
}

/// The printer behind an announcement: its lowest IPv4 address (stable between announcements), the HTTP port, and
/// the host name without `.local.` as name. Without an IPv4 address it is left out.
fn announced(host: &str, port: u16, addresses: &[Ipv4Addr], uuid: Option<&str>) -> Option<FoundPrinter> {
    let address = addresses.iter().min()?;
    let endpoint = Endpoint::parse(&format!("{address}:{port}")).ok()?;
    let host = host.trim_end_matches('.').trim_end_matches(".local");
    let name = if host.is_empty() { address.to_string() } else { host.to_owned() };
    let uuid = uuid.filter(|uuid| !uuid.is_empty()).map(str::to_owned);
    Some(FoundPrinter { name, address: endpoint.display(), uuid })
}

async fn check_all(backend: DiscoveryBackend, addresses: &[String]) -> ReachableAddresses {
    let checks = addresses.iter().map(|address| async move { (address.clone(), answers(backend, address).await) });
    futures_util::future::join_all(checks).await.into_iter().collect()
}

/// Something accepts a connection at the address; whether it is Moonraker shows once connected.
async fn answers(backend: DiscoveryBackend, address: &str) -> bool {
    let Ok(endpoint) = Endpoint::parse(address) else { return false };
    match backend {
        DiscoveryBackend::Network => {
            let connect = tokio::net::TcpStream::connect(endpoint.socket_address());
            matches!(tokio::time::timeout(CHECK_TIMEOUT, connect).await, Ok(Ok(_)))
        }
        #[cfg(feature = "dev-mode")]
        DiscoveryBackend::Simulated => endpoint.is_local(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_announcement_names_the_printer_after_its_host() {
        let addresses = [Ipv4Addr::new(192, 168, 40, 9), Ipv4Addr::new(192, 168, 40, 6)];

        let printer = announced("voron.local.", 7125, &addresses, Some("29f7565a")).unwrap();

        let expected = FoundPrinter {
            name: "voron".to_owned(),
            address: "http://192.168.40.6:7125".to_owned(),
            uuid: Some("29f7565a".to_owned()),
        };
        assert_eq!(printer, expected);
    }

    #[test]
    fn an_announcement_without_ipv4_or_uuid() {
        assert_eq!(announced("voron.local.", 7125, &[], Some("x")), None);
        let printer = announced("", 80, &[Ipv4Addr::new(10, 0, 0, 2)], Some("")).unwrap();
        assert_eq!((printer.name.as_str(), printer.uuid), ("10.0.0.2", None));
    }

    #[tokio::test]
    async fn a_listening_address_answers_a_closed_one_does_not() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let open = format!("http://{}", listener.local_addr().unwrap());
        let closed_port = {
            let probe = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
            probe.local_addr().unwrap().port()
        };
        let closed = format!("http://127.0.0.1:{closed_port}");

        let reachable = check_all(DiscoveryBackend::Network, &[open.clone(), closed.clone()]).await;

        assert_eq!(reachable, [(open, true), (closed, false)].into());
    }

    /// By hand, with a Moonraker that has `[zeroconf]` on the local network:
    /// `cargo test -p sailhand-core finds_printers_on_this_network -- --ignored --nocapture`
    #[tokio::test]
    #[ignore = "needs a Moonraker with [zeroconf] on the local network"]
    async fn finds_printers_on_this_network() {
        let (service, mut states) =
            DiscoveryService::start(&tokio::runtime::Handle::current(), DiscoveryBackend::Network);
        service.watch(true);

        let found =
            tokio::time::timeout(Duration::from_secs(5), states.wait_for(|state| !state.found.is_empty())).await;

        let found = found.expect("no printer announced within 5 s").unwrap().found.clone();
        println!("{found:#?}");
    }
}
