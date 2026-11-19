//! Network management of the host sailhand runs on: Wi-Fi and Ethernet via NetworkManager
//! (Linux), or a simulation for development and tests. Every Wi-Fi and Ethernet adapter of the host is
//! listed; the UI shows one of them at a time.
//!
//! Moonraker can only report the printer host's interfaces; changing them needs local access to
//! NetworkManager. The UI therefore offers management only when this host is the printer host.

mod service;
#[cfg(feature = "dev-mode")]
mod simulated;

#[cfg(target_os = "linux")]
mod networkmanager;

pub use service::{NetworkBackend, NetworkChannels, NetworkService};

use thiserror::Error;

/// Minimum and maximum length of a WPA passphrase.
const PASSPHRASE_LENGTH: std::ops::RangeInclusive<usize> = 8..=63;

/// Access point flags of NetworkManager (`NM80211ApFlags`, `NM80211ApSecurityFlags`).
const AP_FLAG_PRIVACY: u32 = 0x1;
const AP_KEY_MGMT_PSK: u32 = 0x100;
const AP_KEY_MGMT_802_1X: u32 = 0x200;
const AP_KEY_MGMT_SAE: u32 = 0x400;

/// Protection of a Wi-Fi network as far as sailhand can join it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Security {
    #[default]
    Open,
    /// WPA/WPA2 personal (pre-shared key).
    Wpa,
    /// WPA3 personal (SAE) only.
    Wpa3,
    /// 802.1X enterprise or WEP; not supported.
    Unsupported,
}

impl Security {
    /// Derives the security from NetworkManager's access point flags.
    pub fn from_flags(flags: u32, wpa_flags: u32, rsn_flags: u32) -> Self {
        let key_management = wpa_flags | rsn_flags;
        if key_management & AP_KEY_MGMT_802_1X != 0 {
            Self::Unsupported
        } else if key_management & AP_KEY_MGMT_PSK != 0 {
            Self::Wpa
        } else if key_management & AP_KEY_MGMT_SAE != 0 {
            Self::Wpa3
        } else if flags & AP_FLAG_PRIVACY != 0 {
            Self::Unsupported
        } else {
            Self::Open
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct WifiNetwork {
    pub ssid: String,
    /// Signal strength in percent.
    pub signal: u8,
    pub security: Security,
    /// A saved connection exists for this network.
    pub known: bool,
    pub active: bool,
    /// Another Wi-Fi adapter (interface) has joined this network; `active` only refers to the selected one.
    pub active_on_other: Option<String>,
    /// Currently received; saved networks out of range are listed too.
    pub in_range: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum AdapterKind {
    #[default]
    Ethernet,
    Wifi,
}

/// NetworkManager device state reduced to what the UI distinguishes; the order is the preference order.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Default)]
pub enum AdapterState {
    #[default]
    Unavailable,
    Disconnected,
    Connecting,
    Connected,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Adapter {
    pub interface: String,
    pub kind: AdapterKind,
    pub state: AdapterState,
    /// Wi-Fi network the adapter has joined.
    pub ssid: Option<String>,
    /// First IPv4 address, e.g. "192.168.1.42", and its prefix length (24).
    pub address: Option<String>,
    pub prefix: Option<u8>,
    pub gateway: Option<String>,
    /// Link speed in Mbit/s; NetworkManager reports 0 when unknown, that becomes None.
    pub speed_mbit: Option<u32>,
    /// Hardware (MAC) address.
    pub mac: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum Availability {
    #[default]
    Checking,
    /// Networks of this host can be managed.
    Managed,
    /// No management possible; the reason is shown to the user.
    Unavailable(String),
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct NetworkState {
    pub availability: Availability,
    pub wifi_enabled: bool,
    /// Wi-Fi adapters first, then Ethernet, each sorted by interface name.
    pub adapters: Vec<Adapter>,
    /// Interface shown on the right, see [`selected_adapter`]; None without adapters.
    pub selected: Option<String>,
    /// Networks of the selected adapter when it is Wi-Fi, else empty; visible and saved networks in one
    /// list, see [`wifi_networks`].
    pub networks: Vec<WifiNetwork>,
    /// Running operation, e.g. "Connecting to Workshop".
    pub busy: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NetworkCommand {
    Refresh,
    /// Chooses the adapter shown on the right; handled by the service, not by the backends.
    Select {
        interface: String,
    },
    Scan {
        interface: String,
    },
    /// Joins a network; without password a saved or open network is used.
    Connect {
        interface: String,
        ssid: String,
        password: Option<String>,
        hidden: bool,
    },
    /// Leaves the current Wi-Fi network and keeps it saved.
    Disconnect {
        interface: String,
    },
    Forget {
        ssid: String,
    },
    SetWifiEnabled(bool),
    SetWired {
        interface: String,
        connected: bool,
    },
}

/// Result message of an operation for a short notice in the UI.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NetworkNotice {
    pub message: String,
    pub error: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum NetworkError {
    #[error("network management is not available: {0}")]
    Unavailable(String),
    #[error("the password must have {min} to {max} characters", min = PASSPHRASE_LENGTH.start(), max = PASSPHRASE_LENGTH.end())]
    InvalidPassword,
    #[error("network '{0}' requires a password")]
    PasswordRequired(String),
    #[error("networks with 802.1X (enterprise) or WEP security are not supported")]
    UnsupportedSecurity,
    #[error("network '{0}' not found")]
    NotFound(String),
    #[error("could not connect to '{0}' (wrong password or weak signal?)")]
    ConnectFailed(String),
    #[error("NetworkManager: {0}")]
    Backend(String),
}

/// Settings for a new Wi-Fi connection, independent of the D-Bus representation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WifiProfile {
    pub ssid: String,
    pub hidden: bool,
    /// NetworkManager `key-mgmt`; `None` for open networks.
    pub key_management: Option<&'static str>,
    pub passphrase: Option<String>,
}

/// Checks the input and decides how to secure the new connection.
pub fn wifi_profile(
    ssid: &str,
    security: Security,
    password: Option<&str>,
    hidden: bool,
) -> Result<WifiProfile, NetworkError> {
    let password = password.filter(|password| !password.is_empty());
    let key_management = match (security, password) {
        (Security::Unsupported, _) => return Err(NetworkError::UnsupportedSecurity),
        (Security::Open, None) => None,
        // Hidden networks do not reveal their security; a password means WPA.
        (Security::Open, Some(_)) if hidden => Some("wpa-psk"),
        (Security::Open, Some(_)) => None,
        (Security::Wpa | Security::Wpa3, None) => return Err(NetworkError::PasswordRequired(ssid.to_owned())),
        (Security::Wpa, Some(_)) => Some("wpa-psk"),
        (Security::Wpa3, Some(_)) => Some("sae"),
    };
    let passphrase = key_management.and(password).map(str::to_owned);
    if passphrase.as_ref().is_some_and(|passphrase| !PASSPHRASE_LENGTH.contains(&passphrase.chars().count())) {
        return Err(NetworkError::InvalidPassword);
    }
    Ok(WifiProfile { ssid: ssid.to_owned(), hidden, key_management, passphrase })
}

/// One access point as reported by the backend; several may share a network name.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AccessPoint {
    pub ssid: String,
    pub signal: u8,
    pub security: Security,
}

/// One entry per network name: received networks (active first, then by signal), followed by
/// saved networks that are out of range (by name).
pub fn wifi_networks(access_points: &[AccessPoint], known: &[String], active: Option<&str>) -> Vec<WifiNetwork> {
    let mut networks: Vec<WifiNetwork> = Vec::new();
    for point in access_points.iter().filter(|point| !point.ssid.is_empty()) {
        match networks.iter_mut().find(|network| network.ssid == point.ssid) {
            Some(network) if network.signal < point.signal => network.signal = point.signal,
            Some(_) => {}
            None => networks.push(WifiNetwork {
                ssid: point.ssid.clone(),
                signal: point.signal,
                security: point.security,
                known: known.contains(&point.ssid),
                active: active == Some(point.ssid.as_str()),
                active_on_other: None,
                in_range: true,
            }),
        }
    }
    networks
        .sort_by(|left, right| (right.active, right.signal, &left.ssid).cmp(&(left.active, left.signal, &right.ssid)));
    let mut out_of_range: Vec<&String> =
        known.iter().filter(|name| !networks.iter().any(|network| &network.ssid == *name)).collect();
    out_of_range.sort();
    out_of_range.dedup();
    networks.extend(out_of_range.into_iter().map(|ssid| WifiNetwork {
        ssid: ssid.clone(),
        known: true,
        ..WifiNetwork::default()
    }));
    networks
}

/// Wi-Fi adapters first, then Ethernet, each by interface name.
pub fn sort_adapters(adapters: &mut [Adapter]) {
    adapters.sort_by(|left, right| {
        (left.kind == AdapterKind::Ethernet, &left.interface)
            .cmp(&(right.kind == AdapterKind::Ethernet, &right.interface))
    });
}

/// The adapter the UI shows: the requested one if it exists, else the Wi-Fi adapter with the best state
/// (the first on a tie), else the first adapter.
///
/// With a second Wi-Fi adapter (say a USB stick next to the built-in one) the UI must default to the
/// one that carries the connection; acting on another would move the saved profile over to it.
pub fn selected_adapter(adapters: &[Adapter], requested: Option<&str>) -> Option<String> {
    let chosen = requested
        .and_then(|interface| adapters.iter().find(|adapter| adapter.interface == interface))
        .or_else(|| {
            // `max_by_key` returns the last maximum; reversing keeps the first on a tie.
            adapters
                .iter()
                .rev()
                .filter(|adapter| adapter.kind == AdapterKind::Wifi)
                .max_by_key(|adapter| adapter.state)
        })
        .or_else(|| adapters.first());
    chosen.map(|adapter| adapter.interface.clone())
}

/// Marks networks that another Wi-Fi adapter than `selected` has joined.
pub fn mark_other_adapters(networks: &mut [WifiNetwork], adapters: &[Adapter], selected: &str) {
    for network in networks {
        network.active_on_other = adapters
            .iter()
            .find(|adapter| {
                adapter.kind == AdapterKind::Wifi
                    && adapter.interface != selected
                    && adapter.state == AdapterState::Connected
                    && adapter.ssid.as_deref() == Some(network.ssid.as_str())
            })
            .map(|adapter| adapter.interface.clone());
    }
}

/// Signal strength as bars 1..=3 for the icon.
pub fn signal_bars(signal: u8) -> u8 {
    match signal {
        0..=39 => 1,
        40..=69 => 2,
        _ => 3,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn point(ssid: &str, signal: u8, security: Security) -> AccessPoint {
        AccessPoint { ssid: ssid.into(), signal, security }
    }

    #[test]
    fn merges_access_points_and_appends_saved_networks_out_of_range() {
        let points = [
            point("Garage", 40, Security::Wpa),
            point("Cafe", 90, Security::Open),
            point("Workshop", 30, Security::Wpa),
            point("Garage", 70, Security::Wpa),
            point("", 99, Security::Open),
        ];
        let known = ["Home".into(), "Garage".into(), "Attic".into(), "Home".into()];
        let networks = wifi_networks(&points, &known, Some("Workshop"));

        let names: Vec<&str> = networks.iter().map(|network| network.ssid.as_str()).collect();
        assert_eq!(names, ["Workshop", "Cafe", "Garage", "Attic", "Home"]);
        assert!(networks[0].active);
        assert!(networks[2].known && networks[2].in_range);
        assert_eq!(networks[2].signal, 70);
        assert!(networks[3].known && !networks[3].in_range);
    }

    #[test]
    fn chooses_key_management_by_security() {
        let wpa = wifi_profile("Home", Security::Wpa, Some("secret-password"), false).unwrap();
        assert_eq!(wpa.key_management, Some("wpa-psk"));
        assert_eq!(wpa.passphrase.as_deref(), Some("secret-password"));
        assert_eq!(
            wifi_profile("Home", Security::Wpa3, Some("secret-password"), false).unwrap().key_management,
            Some("sae")
        );
        assert_eq!(wifi_profile("Cafe", Security::Open, None, false).unwrap().key_management, None);
        assert_eq!(wifi_profile("Cafe", Security::Open, Some("ignored"), false).unwrap().passphrase, None);
        assert_eq!(
            wifi_profile("Hidden", Security::Open, Some("secret-password"), true).unwrap().key_management,
            Some("wpa-psk")
        );
    }

    #[test]
    fn rejects_invalid_passwords_and_unsupported_security() {
        assert_eq!(wifi_profile("Home", Security::Wpa, Some("short"), false), Err(NetworkError::InvalidPassword));
        assert_eq!(
            wifi_profile("Home", Security::Wpa, None, false),
            Err(NetworkError::PasswordRequired("Home".into()))
        );
        assert_eq!(
            wifi_profile("Office", Security::Unsupported, Some("whatever1"), false),
            Err(NetworkError::UnsupportedSecurity)
        );
    }

    #[test]
    fn derives_security_from_access_point_flags() {
        assert_eq!(Security::from_flags(0, 0, 0), Security::Open);
        assert_eq!(Security::from_flags(AP_FLAG_PRIVACY, 0, AP_KEY_MGMT_PSK), Security::Wpa);
        assert_eq!(Security::from_flags(AP_FLAG_PRIVACY, 0, AP_KEY_MGMT_PSK | AP_KEY_MGMT_SAE), Security::Wpa);
        assert_eq!(Security::from_flags(AP_FLAG_PRIVACY, 0, AP_KEY_MGMT_SAE), Security::Wpa3);
        assert_eq!(Security::from_flags(AP_FLAG_PRIVACY, AP_KEY_MGMT_802_1X, 0), Security::Unsupported);
        assert_eq!(Security::from_flags(AP_FLAG_PRIVACY, 0, 0), Security::Unsupported);
    }

    fn adapter(interface: &str, kind: AdapterKind, state: AdapterState, ssid: Option<&str>) -> Adapter {
        Adapter { interface: interface.into(), kind, state, ssid: ssid.map(str::to_owned), ..Adapter::default() }
    }

    #[test]
    fn sorts_wifi_before_ethernet_by_name() {
        let mut adapters = [
            adapter("eth1", AdapterKind::Ethernet, AdapterState::Connected, None),
            adapter("wlan1", AdapterKind::Wifi, AdapterState::Connected, None),
            adapter("eth0", AdapterKind::Ethernet, AdapterState::Connected, None),
            adapter("wlan0", AdapterKind::Wifi, AdapterState::Connected, None),
        ];
        sort_adapters(&mut adapters);
        let names: Vec<&str> = adapters.iter().map(|adapter| adapter.interface.as_str()).collect();
        assert_eq!(names, ["wlan0", "wlan1", "eth0", "eth1"]);
    }

    #[test]
    fn selects_requested_then_best_wifi_then_first() {
        use AdapterKind::{Ethernet, Wifi};
        use AdapterState::{Connected, Connecting, Disconnected, Unavailable};
        let adapters = [
            adapter("wlan0", Wifi, Disconnected, None),
            adapter("wlan1", Wifi, Connected, Some("Home")),
            adapter("wlan2", Wifi, Connected, None),
            adapter("eth0", Ethernet, Connected, None),
        ];
        assert_eq!(selected_adapter(&adapters, Some("eth0")).as_deref(), Some("eth0"));
        assert_eq!(selected_adapter(&adapters, Some("gone")).as_deref(), Some("wlan1"));
        assert_eq!(selected_adapter(&adapters, None).as_deref(), Some("wlan1"));

        let idle = [adapter("wlan0", Wifi, Unavailable, None), adapter("wlan1", Wifi, Connecting, None)];
        assert_eq!(selected_adapter(&idle, None).as_deref(), Some("wlan1"));
        let wired_only = [adapter("eth0", Ethernet, Connected, None), adapter("eth1", Ethernet, Connected, None)];
        assert_eq!(selected_adapter(&wired_only, None).as_deref(), Some("eth0"));
        assert_eq!(selected_adapter(&[], Some("wlan0")), None);
    }

    #[test]
    fn marks_networks_joined_by_another_adapter() {
        use AdapterKind::Wifi;
        use AdapterState::{Connected, Connecting};
        let adapters = [
            adapter("wlan0", Wifi, Connected, Some("Home")),
            adapter("wlan1", Wifi, Connected, Some("Workshop")),
            adapter("wlan2", Wifi, Connecting, Some("Cafe")),
        ];
        let points =
            [point("Home", 80, Security::Wpa), point("Workshop", 70, Security::Wpa), point("Cafe", 60, Security::Open)];
        let mut networks = wifi_networks(&points, &[], Some("Home"));
        mark_other_adapters(&mut networks, &adapters, "wlan0");
        let marks: Vec<(&str, Option<&str>)> =
            networks.iter().map(|network| (network.ssid.as_str(), network.active_on_other.as_deref())).collect();
        assert_eq!(marks, [("Home", None), ("Workshop", Some("wlan1")), ("Cafe", None)]);
    }

    #[test]
    fn maps_signal_to_bars() {
        assert_eq!(signal_bars(10), 1);
        assert_eq!(signal_bars(55), 2);
        assert_eq!(signal_bars(100), 3);
    }
}
