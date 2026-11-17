//! Network management of the host sailhand runs on: Wi-Fi and Ethernet via NetworkManager
//! (Linux), or a simulation for development and tests.
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
    /// Currently received; saved networks out of range are listed too.
    pub in_range: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct WiredLink {
    pub interface: String,
    /// A cable is plugged in.
    pub cable: bool,
    pub connected: bool,
    pub address: Option<String>,
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
    pub wifi_present: bool,
    pub wifi_enabled: bool,
    pub active_ssid: Option<String>,
    pub wifi_address: Option<String>,
    /// Visible and saved networks in one list, see [`wifi_networks`].
    pub networks: Vec<WifiNetwork>,
    pub wired: Vec<WiredLink>,
    /// Running operation, e.g. "Connecting to Workshop".
    pub busy: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NetworkCommand {
    Refresh,
    Scan,
    /// Joins a network; without password a saved or open network is used.
    Connect {
        ssid: String,
        password: Option<String>,
        hidden: bool,
    },
    /// Leaves the current Wi-Fi network and keeps it saved.
    Disconnect,
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

    #[test]
    fn maps_signal_to_bars() {
        assert_eq!(signal_bars(10), 1);
        assert_eq!(signal_bars(55), 2);
        assert_eq!(signal_bars(100), 3);
    }
}
