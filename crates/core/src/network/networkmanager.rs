//! NetworkManager over D-Bus, see <https://networkmanager.dev/docs/api/latest/>.

use super::{
    AccessPoint, Availability, NetworkCommand, NetworkError, NetworkState, Security, WiredLink, wifi_networks,
    wifi_profile,
};
use std::collections::HashMap;
use std::time::Duration;
use zbus::proxy::CacheProperties;
use zbus::zvariant::{ObjectPath, OwnedObjectPath, OwnedValue, Value};
use zbus::{Connection, proxy};

const DEVICE_TYPE_ETHERNET: u32 = 1;
const DEVICE_TYPE_WIFI: u32 = 2;
/// Device state below which a wired device has no cable (`NM_DEVICE_STATE_UNAVAILABLE`).
const DEVICE_STATE_UNAVAILABLE: u32 = 20;
const DEVICE_STATE_ACTIVATED: u32 = 100;
const ACTIVE_CONNECTION_ACTIVATED: u32 = 2;
const ACTIVE_CONNECTION_DEACTIVATED: u32 = 4;
const ACTIVATION_TIMEOUT: Duration = Duration::from_secs(30);
const ACTIVATION_POLL: Duration = Duration::from_millis(500);
const WIFI_TYPE: &str = "802-11-wireless";
const NO_OBJECT: &str = "/";

#[proxy(
    interface = "org.freedesktop.NetworkManager",
    default_service = "org.freedesktop.NetworkManager",
    default_path = "/org/freedesktop/NetworkManager"
)]
trait Manager {
    fn get_devices(&self) -> zbus::Result<Vec<OwnedObjectPath>>;

    fn activate_connection(
        &self,
        connection: &ObjectPath<'_>,
        device: &ObjectPath<'_>,
        specific_object: &ObjectPath<'_>,
    ) -> zbus::Result<OwnedObjectPath>;

    fn add_and_activate_connection(
        &self,
        connection: HashMap<&str, HashMap<&str, Value<'_>>>,
        device: &ObjectPath<'_>,
        specific_object: &ObjectPath<'_>,
    ) -> zbus::Result<(OwnedObjectPath, OwnedObjectPath)>;

    #[zbus(property)]
    fn wireless_enabled(&self) -> zbus::Result<bool>;

    #[zbus(property)]
    fn set_wireless_enabled(&self, enabled: bool) -> zbus::Result<()>;
}

#[proxy(interface = "org.freedesktop.NetworkManager.Device", default_service = "org.freedesktop.NetworkManager")]
trait Device {
    fn disconnect(&self) -> zbus::Result<()>;

    #[zbus(property)]
    fn interface(&self) -> zbus::Result<String>;

    #[zbus(property)]
    fn device_type(&self) -> zbus::Result<u32>;

    #[zbus(property)]
    fn state(&self) -> zbus::Result<u32>;

    #[zbus(property)]
    fn ip4_config(&self) -> zbus::Result<OwnedObjectPath>;
}

#[proxy(
    interface = "org.freedesktop.NetworkManager.Device.Wireless",
    default_service = "org.freedesktop.NetworkManager"
)]
trait Wireless {
    fn get_all_access_points(&self) -> zbus::Result<Vec<OwnedObjectPath>>;

    fn request_scan(&self, options: HashMap<&str, Value<'_>>) -> zbus::Result<()>;

    #[zbus(property)]
    fn active_access_point(&self) -> zbus::Result<OwnedObjectPath>;
}

#[proxy(interface = "org.freedesktop.NetworkManager.AccessPoint", default_service = "org.freedesktop.NetworkManager")]
trait Ap {
    #[zbus(property)]
    fn ssid(&self) -> zbus::Result<Vec<u8>>;

    #[zbus(property)]
    fn strength(&self) -> zbus::Result<u8>;

    #[zbus(property)]
    fn flags(&self) -> zbus::Result<u32>;

    #[zbus(property)]
    fn wpa_flags(&self) -> zbus::Result<u32>;

    #[zbus(property)]
    fn rsn_flags(&self) -> zbus::Result<u32>;
}

#[proxy(interface = "org.freedesktop.NetworkManager.IP4Config", default_service = "org.freedesktop.NetworkManager")]
trait Ip4Config {
    #[zbus(property)]
    fn address_data(&self) -> zbus::Result<Vec<HashMap<String, OwnedValue>>>;
}

#[proxy(
    interface = "org.freedesktop.NetworkManager.Settings",
    default_service = "org.freedesktop.NetworkManager",
    default_path = "/org/freedesktop/NetworkManager/Settings"
)]
trait Settings {
    fn list_connections(&self) -> zbus::Result<Vec<OwnedObjectPath>>;
}

#[proxy(
    interface = "org.freedesktop.NetworkManager.Settings.Connection",
    default_service = "org.freedesktop.NetworkManager"
)]
trait SavedConnection {
    fn get_settings(&self) -> zbus::Result<HashMap<String, HashMap<String, OwnedValue>>>;

    fn delete(&self) -> zbus::Result<()>;
}

#[proxy(
    interface = "org.freedesktop.NetworkManager.Connection.Active",
    default_service = "org.freedesktop.NetworkManager"
)]
trait ActiveConnection {
    #[zbus(property)]
    fn state(&self) -> zbus::Result<u32>;
}

/// Builds a proxy for an object path without property cache (values are read once per refresh).
macro_rules! at {
    ($proxy:ident, $connection:expr, $path:expr) => {
        $proxy::builder($connection).path($path.clone())?.cache_properties(CacheProperties::No).build().await?
    };
}

impl From<zbus::Error> for NetworkError {
    fn from(error: zbus::Error) -> Self {
        let message = error.to_string();
        if message.contains("PermissionDenied") || message.contains("not authorized") {
            Self::Backend("permission denied; is the polkit rule of sailhand installed?".to_owned())
        } else {
            Self::Backend(message)
        }
    }
}

/// A saved Wi-Fi connection.
struct SavedWifi {
    ssid: String,
    path: OwnedObjectPath,
}

pub struct NetworkManager {
    connection: Connection,
    manager: ManagerProxy<'static>,
}

impl NetworkManager {
    /// Connects to the system bus; fails when NetworkManager is not running.
    pub async fn connect() -> Result<Self, NetworkError> {
        let unavailable =
            |error: zbus::Error| NetworkError::Unavailable(format!("NetworkManager not reachable ({error})"));
        let connection = Connection::system().await.map_err(unavailable)?;
        let manager = ManagerProxy::builder(&connection)
            .cache_properties(CacheProperties::No)
            .build()
            .await
            .map_err(unavailable)?;
        manager.wireless_enabled().await.map_err(unavailable)?;
        Ok(Self { connection, manager })
    }

    pub async fn apply(&self, command: &NetworkCommand) -> Result<(), NetworkError> {
        match command {
            NetworkCommand::Refresh => Ok(()),
            NetworkCommand::Scan => self.scan().await,
            NetworkCommand::Connect { ssid, password, hidden } => {
                self.connect_wifi(ssid, password.as_deref(), *hidden).await
            }
            NetworkCommand::Disconnect => self.disconnect_wifi().await,
            NetworkCommand::Forget { ssid } => self.forget(ssid).await,
            NetworkCommand::SetWifiEnabled(enabled) => self.set_wifi_enabled(*enabled).await,
            NetworkCommand::SetWired { interface, connected } => self.set_wired(interface, *connected).await,
        }
    }

    pub async fn read(&self) -> Result<NetworkState, NetworkError> {
        let known = self.saved_wifi().await?;
        let known_names: Vec<String> = known.iter().map(|saved| saved.ssid.clone()).collect();
        let mut state = NetworkState {
            availability: Availability::Managed,
            wifi_enabled: self.manager.wireless_enabled().await?,
            ..NetworkState::default()
        };
        for path in self.manager.get_devices().await? {
            let device = at!(DeviceProxy, &self.connection, path);
            match device.device_type().await? {
                DEVICE_TYPE_WIFI if !state.wifi_present => {
                    state.wifi_present = true;
                    let wireless = at!(WirelessProxy, &self.connection, path);
                    let active = self.active_ssid(&wireless).await?;
                    let access_points = self.access_points(&wireless).await?;
                    state.networks = wifi_networks(&access_points, &known_names, active.as_deref());
                    if active.is_some() {
                        state.wifi_address = self.address(&device).await?;
                    }
                    state.active_ssid = active;
                }
                DEVICE_TYPE_ETHERNET => state.wired.push(self.wired_link(&device).await?),
                _ => {}
            }
        }
        Ok(state)
    }

    pub async fn scan(&self) -> Result<(), NetworkError> {
        let (_, wireless) = self.wifi_device().await?;
        // NetworkManager refuses scans in quick succession; the cached list is still fine.
        if let Err(error) = wireless.request_scan(HashMap::new()).await {
            log::debug!("Wi-Fi scan not started: {error}");
        }
        Ok(())
    }

    pub async fn connect_wifi(&self, ssid: &str, password: Option<&str>, hidden: bool) -> Result<(), NetworkError> {
        let (device, wireless) = self.wifi_device().await?;
        let saved = self.saved_wifi().await?;
        let existing: Vec<&SavedWifi> = saved.iter().filter(|saved| saved.ssid == ssid).collect();
        if password.is_none()
            && !hidden
            && let Some(saved) = existing.first()
        {
            let active = self.manager.activate_connection(&saved.path, &device, &no_object()).await?;
            return self.wait_for_activation(&active, ssid).await;
        }
        let (security, access_point) = if hidden {
            (Security::Open, no_object().into())
        } else {
            self.strongest_access_point(&wireless, ssid)
                .await?
                .ok_or_else(|| NetworkError::NotFound(ssid.to_owned()))?
        };
        let profile = wifi_profile(ssid, security, password, hidden)?;
        for saved in existing {
            self.delete(&saved.path).await?;
        }
        let (created, active) =
            self.manager.add_and_activate_connection(connection_settings(&profile), &device, &access_point).await?;
        let result = self.wait_for_activation(&active, ssid).await;
        if result.is_err() {
            // Like other front ends: do not keep a profile with a wrong password.
            self.delete(&created).await?;
        }
        result
    }

    /// NetworkManager does not auto-connect the device again until the user picks a network.
    pub async fn disconnect_wifi(&self) -> Result<(), NetworkError> {
        let (path, _) = self.wifi_device().await?;
        let device = at!(DeviceProxy, &self.connection, path);
        Ok(device.disconnect().await?)
    }

    pub async fn forget(&self, ssid: &str) -> Result<(), NetworkError> {
        let matching: Vec<SavedWifi> =
            self.saved_wifi().await?.into_iter().filter(|saved| saved.ssid == ssid).collect();
        if matching.is_empty() {
            return Err(NetworkError::NotFound(ssid.to_owned()));
        }
        for saved in matching {
            self.delete(&saved.path).await?;
        }
        Ok(())
    }

    pub async fn set_wifi_enabled(&self, enabled: bool) -> Result<(), NetworkError> {
        Ok(self.manager.set_wireless_enabled(enabled).await?)
    }

    pub async fn set_wired(&self, interface: &str, connected: bool) -> Result<(), NetworkError> {
        for path in self.manager.get_devices().await? {
            let device = at!(DeviceProxy, &self.connection, path);
            if device.device_type().await? == DEVICE_TYPE_ETHERNET && device.interface().await? == interface {
                if connected {
                    // Without a connection path NetworkManager picks the best saved profile for the device.
                    self.manager.activate_connection(&no_object(), &path, &no_object()).await?;
                } else {
                    device.disconnect().await?;
                }
                return Ok(());
            }
        }
        Err(NetworkError::NotFound(interface.to_owned()))
    }

    async fn wifi_device(&self) -> Result<(OwnedObjectPath, WirelessProxy<'static>), NetworkError> {
        for path in self.manager.get_devices().await? {
            let device = at!(DeviceProxy, &self.connection, path);
            if device.device_type().await? == DEVICE_TYPE_WIFI {
                let wireless = at!(WirelessProxy, &self.connection, path);
                return Ok((path, wireless));
            }
        }
        Err(NetworkError::NotFound("Wi-Fi adapter".to_owned()))
    }

    async fn access_points(&self, wireless: &WirelessProxy<'_>) -> Result<Vec<AccessPoint>, NetworkError> {
        let mut points = Vec::new();
        for path in wireless.get_all_access_points().await? {
            points.push(self.access_point(&path).await?);
        }
        Ok(points)
    }

    async fn access_point(&self, path: &OwnedObjectPath) -> Result<AccessPoint, NetworkError> {
        let point = at!(ApProxy, &self.connection, path);
        Ok(AccessPoint {
            ssid: String::from_utf8_lossy(&point.ssid().await?).into_owned(),
            signal: point.strength().await?,
            security: Security::from_flags(point.flags().await?, point.wpa_flags().await?, point.rsn_flags().await?),
        })
    }

    async fn strongest_access_point(
        &self,
        wireless: &WirelessProxy<'_>,
        ssid: &str,
    ) -> Result<Option<(Security, OwnedObjectPath)>, NetworkError> {
        let mut best: Option<(u8, Security, OwnedObjectPath)> = None;
        for path in wireless.get_all_access_points().await? {
            let point = self.access_point(&path).await?;
            if point.ssid == ssid && best.as_ref().is_none_or(|(signal, _, _)| point.signal > *signal) {
                best = Some((point.signal, point.security, path));
            }
        }
        Ok(best.map(|(_, security, path)| (security, path)))
    }

    async fn active_ssid(&self, wireless: &WirelessProxy<'_>) -> Result<Option<String>, NetworkError> {
        let path = wireless.active_access_point().await?;
        if path.as_str() == NO_OBJECT {
            return Ok(None);
        }
        Ok(Some(self.access_point(&path).await?.ssid))
    }

    async fn wired_link(&self, device: &DeviceProxy<'_>) -> Result<WiredLink, NetworkError> {
        let state = device.state().await?;
        let connected = state == DEVICE_STATE_ACTIVATED;
        Ok(WiredLink {
            interface: device.interface().await?,
            cable: state > DEVICE_STATE_UNAVAILABLE,
            connected,
            address: if connected { self.address(device).await? } else { None },
        })
    }

    async fn address(&self, device: &DeviceProxy<'_>) -> Result<Option<String>, NetworkError> {
        let path = device.ip4_config().await?;
        if path.as_str() == NO_OBJECT {
            return Ok(None);
        }
        let config = at!(Ip4ConfigProxy, &self.connection, path);
        Ok(config.address_data().await?.iter().find_map(|entry| entry.get("address").and_then(text)))
    }

    async fn saved_wifi(&self) -> Result<Vec<SavedWifi>, NetworkError> {
        let settings = SettingsProxy::builder(&self.connection).cache_properties(CacheProperties::No).build().await?;
        let mut saved = Vec::new();
        for path in settings.list_connections().await? {
            let connection = at!(SavedConnectionProxy, &self.connection, path);
            let values = connection.get_settings().await?;
            let is_wifi = values.get("connection").and_then(|section| section.get("type")).and_then(text)
                == Some(WIFI_TYPE.to_owned());
            let ssid = values.get(WIFI_TYPE).and_then(|section| section.get("ssid")).map(bytes);
            if let (true, Some(ssid)) = (is_wifi, ssid) {
                saved.push(SavedWifi { ssid: String::from_utf8_lossy(&ssid).into_owned(), path });
            }
        }
        Ok(saved)
    }

    async fn delete(&self, path: &OwnedObjectPath) -> Result<(), NetworkError> {
        let connection = at!(SavedConnectionProxy, &self.connection, path);
        Ok(connection.delete().await?)
    }

    async fn wait_for_activation(&self, path: &OwnedObjectPath, ssid: &str) -> Result<(), NetworkError> {
        let failed = || NetworkError::ConnectFailed(ssid.to_owned());
        let active = at!(ActiveConnectionProxy, &self.connection, path);
        let deadline = tokio::time::Instant::now() + ACTIVATION_TIMEOUT;
        while tokio::time::Instant::now() < deadline {
            // The object disappears when the activation is given up.
            match active.state().await {
                Ok(ACTIVE_CONNECTION_ACTIVATED) => return Ok(()),
                Ok(ACTIVE_CONNECTION_DEACTIVATED) | Err(_) => return Err(failed()),
                Ok(_) => tokio::time::sleep(ACTIVATION_POLL).await,
            }
        }
        Err(failed())
    }
}

fn no_object() -> ObjectPath<'static> {
    ObjectPath::from_static_str_unchecked(NO_OBJECT)
}

fn connection_settings(profile: &super::WifiProfile) -> HashMap<&str, HashMap<&str, Value<'_>>> {
    let mut wireless = HashMap::from([
        ("ssid", Value::new(profile.ssid.as_bytes().to_vec())),
        ("mode", Value::from("infrastructure")),
    ]);
    if profile.hidden {
        wireless.insert("hidden", Value::from(true));
    }
    let mut settings = HashMap::from([
        (
            "connection",
            HashMap::from([
                ("id", Value::from(profile.ssid.as_str())),
                ("type", Value::from(WIFI_TYPE)),
                ("autoconnect", Value::from(true)),
            ]),
        ),
        (WIFI_TYPE, wireless),
    ]);
    if let (Some(key_management), Some(passphrase)) = (profile.key_management, profile.passphrase.as_deref()) {
        settings.insert(
            "802-11-wireless-security",
            HashMap::from([("key-mgmt", Value::from(key_management)), ("psk", Value::from(passphrase))]),
        );
    }
    settings
}

fn text(value: &OwnedValue) -> Option<String> {
    match &**value {
        Value::Str(text) => Some(text.to_string()),
        _ => None,
    }
}

fn bytes(value: &OwnedValue) -> Vec<u8> {
    match &**value {
        Value::Array(array) => array
            .iter()
            .filter_map(|item| match item {
                Value::U8(byte) => Some(*byte),
                _ => None,
            })
            .collect(),
        _ => Vec::new(),
    }
}
