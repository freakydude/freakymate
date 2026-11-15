//! In-memory networks for development without NetworkManager (Windows, screenshots, tests).

use super::{
    AccessPoint, Availability, NetworkCommand, NetworkError, NetworkState, Security, WiredLink, wifi_networks,
    wifi_profile,
};
use std::time::Duration;

/// Simulated time a connection attempt takes.
const CONNECT_DELAY: Duration = Duration::from_millis(800);
/// Password that the simulation rejects, to try the error path.
const WRONG_PASSWORD: &str = "wrongpassword";

pub struct Simulated {
    access_points: Vec<AccessPoint>,
    known: Vec<String>,
    active: Option<String>,
    wifi_enabled: bool,
    wired: Vec<WiredLink>,
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
            wired: vec![WiredLink {
                interface: "eth0".to_owned(),
                cable: true,
                connected: true,
                address: Some("192.168.1.42".to_owned()),
            }],
        }
    }
}

impl Simulated {
    pub async fn apply(&mut self, command: &NetworkCommand) -> Result<(), NetworkError> {
        match command {
            NetworkCommand::Refresh | NetworkCommand::Scan => Ok(()),
            NetworkCommand::Connect { ssid, password, hidden } => {
                self.connect(ssid, password.as_deref(), *hidden).await
            }
            NetworkCommand::Disconnect => {
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

    pub fn read(&self) -> NetworkState {
        let active = self.active.as_deref().filter(|_| self.wifi_enabled);
        NetworkState {
            availability: Availability::Managed,
            wifi_present: true,
            wifi_enabled: self.wifi_enabled,
            active_ssid: active.map(str::to_owned),
            wifi_address: active.map(|_| "192.168.1.57".to_owned()),
            networks: if self.wifi_enabled {
                wifi_networks(&self.access_points, &self.known, active)
            } else {
                Vec::new()
            },
            wired: self.wired.clone(),
            busy: None,
        }
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
        let link = self
            .wired
            .iter_mut()
            .find(|link| link.interface == interface)
            .ok_or_else(|| NetworkError::NotFound(interface.to_owned()))?;
        link.connected = connected && link.cable;
        link.address = link.connected.then(|| "192.168.1.42".to_owned());
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn joins_forgets_and_switches() {
        let mut network = Simulated::default();
        assert_eq!(network.read().active_ssid.as_deref(), Some("Workshop"));

        assert_eq!(network.connect("Cafe Guest", None, false).await, Ok(()));
        assert!(network.read().networks.iter().any(|network| network.ssid == "Cafe Guest" && network.known));
        network.disconnect();
        assert_eq!(network.read().active_ssid, None);
        network.connect("Cafe Guest", None, false).await.unwrap();
        assert_eq!(
            network.connect("Neighbour", Some(WRONG_PASSWORD), false).await,
            Err(NetworkError::ConnectFailed("Neighbour".into()))
        );

        network.forget("Cafe Guest").unwrap();
        assert_eq!(network.read().active_ssid, None);

        network.set_wifi_enabled(false);
        assert!(network.read().networks.is_empty());
        network.set_wired("eth0", false).unwrap();
        assert!(!network.read().wired[0].connected);
    }
}
