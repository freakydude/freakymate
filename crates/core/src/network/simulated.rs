//! In-memory networks for development without NetworkManager (Windows, screenshots, tests).

use super::{
    AccessPoint, Adapter, AdapterKind, AdapterState, Availability, NetworkCommand, NetworkError, NetworkState,
    Security, mark_other_adapters, selected_adapter, sort_adapters, wifi_networks, wifi_profile,
};
use std::time::Duration;

/// Simulated time a connection attempt takes.
const CONNECT_DELAY: Duration = Duration::from_millis(800);
/// Password that the simulation rejects, to try the error path.
const WRONG_PASSWORD: &str = "wrongpassword";
/// Interfaces of the simulated adapters; addresses, gateway, speed and MACs below are assumed example values.
const WIFI_INTERFACE: &str = "wlan0";
const ETHERNET_INTERFACE: &str = "eth0";
const GATEWAY: &str = "192.168.1.1";

pub struct Simulated {
    access_points: Vec<AccessPoint>,
    known: Vec<String>,
    active: Option<String>,
    wifi_enabled: bool,
    /// The wired adapter is connected; its cable stays plugged.
    wired_connected: bool,
}

impl Default for Simulated {
    fn default() -> Self {
        let point = |ssid: &str, signal, security| AccessPoint { ssid: ssid.to_owned(), signal, security };
        Self {
            access_points: vec![
                point("Workshop", 82, Security::Wpa),
                point("Garage", 48, Security::Wpa),
                point("Cafe Guest", 64, Security::Open),
                point("Neighbour", 22, Security::Wpa3),
                point("Office", 35, Security::Unsupported),
                point("Workshop", 41, Security::Wpa),
            ],
            known: vec!["Workshop".to_owned(), "Garage".to_owned(), "Holiday Flat".to_owned()],
            active: Some("Workshop".to_owned()),
            wifi_enabled: true,
            wired_connected: true,
        }
    }
}

impl Simulated {
    pub async fn apply(&mut self, command: &NetworkCommand) -> Result<(), NetworkError> {
        match command {
            NetworkCommand::Refresh | NetworkCommand::Select { .. } => Ok(()),
            NetworkCommand::Scan { interface } => self.require_wifi(interface),
            NetworkCommand::Connect { interface, ssid, password, hidden } => {
                self.require_wifi(interface)?;
                self.connect(ssid, password.as_deref(), *hidden).await
            }
            NetworkCommand::Disconnect { interface } => {
                self.require_wifi(interface)?;
                self.disconnect();
                Ok(())
            }
            NetworkCommand::Forget { ssid } => self.forget(ssid),
            NetworkCommand::SetWifiEnabled(enabled) => {
                self.set_wifi_enabled(*enabled);
                Ok(())
            }
            NetworkCommand::SetWired { interface, connected } => self.set_wired(interface, *connected),
        }
    }

    pub fn read(&self, requested: Option<&str>) -> NetworkState {
        let active = self.active.as_deref().filter(|_| self.wifi_enabled);
        let mut adapters = vec![self.wifi_adapter(active), self.ethernet_adapter()];
        sort_adapters(&mut adapters);
        let selected = selected_adapter(&adapters, requested);
        let mut networks = Vec::new();
        if self.wifi_enabled && selected.as_deref() == Some(WIFI_INTERFACE) {
            networks = wifi_networks(&self.access_points, &self.known, active);
            mark_other_adapters(&mut networks, &adapters, WIFI_INTERFACE);
        }
        NetworkState {
            availability: Availability::Managed,
            wifi_enabled: self.wifi_enabled,
            adapters,
            selected,
            networks,
            busy: None,
        }
    }

    fn wifi_adapter(&self, active: Option<&str>) -> Adapter {
        let state = match (self.wifi_enabled, active) {
            (false, _) => AdapterState::Unavailable,
            (true, Some(_)) => AdapterState::Connected,
            (true, None) => AdapterState::Disconnected,
        };
        Adapter {
            interface: WIFI_INTERFACE.to_owned(),
            kind: AdapterKind::Wifi,
            state,
            ssid: active.map(str::to_owned),
            address: active.map(|_| "192.168.1.57".to_owned()),
            prefix: active.map(|_| 24),
            gateway: active.map(|_| GATEWAY.to_owned()),
            speed_mbit: None,
            mac: Some("dc:a6:32:01:02:03".to_owned()),
        }
    }

    fn ethernet_adapter(&self) -> Adapter {
        let connected = self.wired_connected;
        Adapter {
            interface: ETHERNET_INTERFACE.to_owned(),
            kind: AdapterKind::Ethernet,
            state: if connected { AdapterState::Connected } else { AdapterState::Disconnected },
            ssid: None,
            address: connected.then(|| "192.168.1.42".to_owned()),
            prefix: connected.then_some(24),
            gateway: connected.then(|| GATEWAY.to_owned()),
            speed_mbit: connected.then_some(1000),
            mac: Some("dc:a6:32:01:02:04".to_owned()),
        }
    }

    fn require_wifi(&self, interface: &str) -> Result<(), NetworkError> {
        if interface == WIFI_INTERFACE { Ok(()) } else { Err(NetworkError::NotFound(interface.to_owned())) }
    }

    pub async fn connect(&mut self, ssid: &str, password: Option<&str>, hidden: bool) -> Result<(), NetworkError> {
        tokio::time::sleep(CONNECT_DELAY).await;
        let reuse_saved = password.is_none() && !hidden && self.known.iter().any(|known| known == ssid);
        if !reuse_saved {
            let security = if hidden {
                Security::Open
            } else {
                self.access_points
                    .iter()
                    .find(|point| point.ssid == ssid)
                    .map(|point| point.security)
                    .ok_or_else(|| NetworkError::NotFound(ssid.to_owned()))?
            };
            let profile = wifi_profile(ssid, security, password, hidden)?;
            if profile.passphrase.as_deref() == Some(WRONG_PASSWORD) {
                return Err(NetworkError::ConnectFailed(ssid.to_owned()));
            }
            if hidden && !self.access_points.iter().any(|point| point.ssid == ssid) {
                self.access_points.push(AccessPoint { ssid: ssid.to_owned(), signal: 55, security: Security::Wpa });
            }
            if !self.known.iter().any(|known| known == ssid) {
                self.known.push(ssid.to_owned());
            }
        }
        self.active = Some(ssid.to_owned());
        Ok(())
    }

    pub fn disconnect(&mut self) {
        self.active = None;
    }

    pub fn forget(&mut self, ssid: &str) -> Result<(), NetworkError> {
        let before = self.known.len();
        self.known.retain(|known| known != ssid);
        if self.known.len() == before {
            return Err(NetworkError::NotFound(ssid.to_owned()));
        }
        if self.active.as_deref() == Some(ssid) {
            self.active = None;
        }
        Ok(())
    }

    pub fn set_wifi_enabled(&mut self, enabled: bool) {
        self.wifi_enabled = enabled;
    }

    pub fn set_wired(&mut self, interface: &str, connected: bool) -> Result<(), NetworkError> {
        if interface != ETHERNET_INTERFACE {
            return Err(NetworkError::NotFound(interface.to_owned()));
        }
        self.wired_connected = connected;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn wifi(state: &NetworkState) -> &Adapter {
        state.adapters.iter().find(|adapter| adapter.interface == WIFI_INTERFACE).unwrap()
    }

    #[tokio::test]
    async fn joins_forgets_and_switches() {
        let mut network = Simulated::default();
        let state = network.read(None);
        assert_eq!(wifi(&state).ssid.as_deref(), Some("Workshop"));
        assert_eq!(state.selected.as_deref(), Some(WIFI_INTERFACE));
        assert_eq!(state.adapters[1].speed_mbit, Some(1000));

        assert_eq!(network.connect("Cafe Guest", None, false).await, Ok(()));
        assert!(network.read(None).networks.iter().any(|network| network.ssid == "Cafe Guest" && network.known));
        network.disconnect();
        assert_eq!(wifi(&network.read(None)).ssid, None);
        network.connect("Cafe Guest", None, false).await.unwrap();
        assert_eq!(
            network.connect("Neighbour", Some(WRONG_PASSWORD), false).await,
            Err(NetworkError::ConnectFailed("Neighbour".into()))
        );

        network.forget("Cafe Guest").unwrap();
        assert_eq!(wifi(&network.read(None)).ssid, None);

        network.set_wifi_enabled(false);
        let state = network.read(None);
        assert!(state.networks.is_empty());
        assert_eq!(wifi(&state).state, AdapterState::Unavailable);
        network.set_wired("eth0", false).unwrap();
        let state = network.read(Some("eth0"));
        assert_eq!(state.selected.as_deref(), Some("eth0"));
        assert_eq!(state.adapters[1].state, AdapterState::Disconnected);
        assert_eq!(state.adapters[1].address, None);
    }

    #[tokio::test]
    async fn rejects_unknown_interfaces() {
        let mut network = Simulated::default();
        let scan = NetworkCommand::Scan { interface: "eth0".into() };
        assert_eq!(network.apply(&scan).await, Err(NetworkError::NotFound("eth0".into())));
        assert_eq!(network.set_wired("wlan0", true), Err(NetworkError::NotFound("wlan0".into())));
    }
}
