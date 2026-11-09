//! Simulated Moonraker host features: power devices, update manager and G-code history.

use serde_json::{json, Value};
use std::collections::VecDeque;

const GCODE_STORE_SIZE: usize = 200;

#[derive(Debug, Clone)]
struct PowerDevice {
    name: &'static str,
    on: bool,
    locked_while_printing: bool,
}

#[derive(Debug, Clone)]
pub struct Host {
    power_devices: Vec<PowerDevice>,
    gcode_store: VecDeque<Value>,
    klipper_updated: bool,
}

impl Default for Host {
    fn default() -> Self {
        Self {
            power_devices: vec![
                PowerDevice { name: "printer", on: true, locked_while_printing: true },
                PowerDevice { name: "light", on: false, locked_while_printing: false },
            ],
            gcode_store: VecDeque::new(),
            klipper_updated: false,
        }
    }
}

impl Host {
    pub fn power_devices(&self) -> Value {
        let devices: Vec<Value> = self.power_devices.iter().map(device_json).collect();
        json!({ "devices": devices })
    }

    /// Switches a device; `printing` locks devices with `locked_while_printing`.
    pub fn set_power(&mut self, name: &str, action: &str, printing: bool) -> Result<Value, String> {
        let device = self
            .power_devices
            .iter_mut()
            .find(|device| device.name == name)
            .ok_or_else(|| format!("No valid device named {name}"))?;
        if printing && device.locked_while_printing {
            return Err(format!("Unable to change power for {name} while printing"));
        }
        device.on = match action {
            "on" => true,
            "off" => false,
            "toggle" => !device.on,
            other => return Err(format!("Invalid power action '{other}'")),
        };
        Ok(json!({ name: if device.on { "on" } else { "off" } }))
    }

    pub fn device_notification(&self, name: &str) -> Option<Value> {
        self.power_devices.iter().find(|device| device.name == name).map(device_json)
    }

    pub fn update_status(&self) -> Value {
        let klipper_version = if self.klipper_updated { "v0.13.0-120" } else { "v0.13.0-100" };
        json!({
            "busy": false,
            "github_rate_limit": 60,
            "github_requests_remaining": 55,
            "github_limit_reset_time": 1_700_000_000,
            "version_info": {
                "system": { "name": "system", "configured_type": "system", "package_count": 2, "package_list": ["libc6", "openssl"] },
                "moonraker": { "name": "moonraker", "configured_type": "git_repo", "version": "v0.9.3-50", "remote_version": "v0.9.3-50", "is_valid": true, "commits_behind_count": 0 },
                "klipper": { "name": "klipper", "configured_type": "git_repo", "version": klipper_version, "remote_version": "v0.13.0-120", "is_valid": true, "commits_behind_count": if self.klipper_updated { 0 } else { 20 } },
                "mainsail": { "name": "mainsail", "configured_type": "web", "version": "v2.14.0", "remote_version": "v2.14.0" },
                "crowsnest": { "name": "crowsnest", "configured_type": "git_repo", "version": "v4.1.12", "remote_version": "v4.1.12", "is_valid": true, "commits_behind_count": 0 },
                "moonraker-timelapse": { "name": "moonraker-timelapse", "configured_type": "git_repo", "version": "v0.0.1-150", "remote_version": "v0.0.1-150", "is_valid": true, "commits_behind_count": 0 },
                "freakymate": { "name": "freakymate", "configured_type": "zip", "version": "v0.2.3", "remote_version": "v0.2.3" },
            }
        })
    }

    /// `machine.update.upgrade`: like Moonraker, refuses a name that is not exactly a section of the update manager
    /// (`Freakymate` for `freakymate` fails); without a name everything is updated.
    pub fn upgrade(&mut self, name: Option<&str>) -> Result<(), String> {
        if let Some(name) = name {
            if self.update_status()["version_info"].get(name).is_none() {
                return Err(format!("Updater {name} not available"));
            }
        }
        if matches!(name, None | Some("klipper")) {
            self.klipper_updated = true;
        }
        Ok(())
    }

    /// Subset of `machine.system_info`: distribution and network interfaces.
    pub fn system_info(&self) -> Value {
        json!({ "system_info": {
            "provider": "systemd_dbus",
            "distribution": { "name": "Debian GNU/Linux 12 (bookworm)", "id": "debian", "version": "12" },
            "network": {
                "wlan0": { "mac_address": "b8:27:eb:00:00:01", "ip_addresses": [
                    { "family": "ipv4", "address": "192.168.1.57", "is_link_local": false },
                    { "family": "ipv6", "address": "fe80::ba27:ebff:fe00:1", "is_link_local": true }
                ]},
                "eth0": { "mac_address": "b8:27:eb:00:00:02", "ip_addresses": [
                    { "family": "ipv4", "address": "192.168.1.42", "is_link_local": false }
                ]}
            }
        }})
    }

    pub fn record(&mut self, kind: &str, message: &str, time: f64) {
        if self.gcode_store.len() == GCODE_STORE_SIZE {
            self.gcode_store.pop_front();
        }
        self.gcode_store.push_back(json!({ "message": message, "time": time, "type": kind }));
    }

    pub fn gcode_store(&self, count: Option<usize>) -> Value {
        let count = count.unwrap_or(self.gcode_store.len()).min(self.gcode_store.len());
        let items: Vec<&Value> = self.gcode_store.iter().skip(self.gcode_store.len() - count).collect();
        json!({ "gcode_store": items })
    }
}

fn device_json(device: &PowerDevice) -> Value {
    json!({
        "device": device.name,
        "status": if device.on { "on" } else { "off" },
        "locked_while_printing": device.locked_while_printing,
        "type": "gpio",
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn switching_power_respects_print_lock() {
        let mut host = Host::default();

        assert_eq!(host.set_power("light", "toggle", true).unwrap(), json!({ "light": "on" }));
        assert!(host.set_power("printer", "off", true).is_err());
        assert_eq!(host.set_power("printer", "off", false).unwrap(), json!({ "printer": "off" }));
        assert!(host.set_power("nope", "on", false).is_err());
    }

    #[test]
    fn upgrade_brings_klipper_up_to_date() {
        let mut host = Host::default();
        assert_eq!(host.update_status()["version_info"]["klipper"]["commits_behind_count"], 20);

        host.upgrade(Some("klipper")).unwrap();

        assert_eq!(host.update_status()["version_info"]["klipper"]["version"], "v0.13.0-120");
    }

    #[test]
    fn upgrade_refuses_a_name_that_is_not_exactly_a_section() {
        let mut host = Host::default();

        assert_eq!(host.upgrade(Some("Freakymate")), Err("Updater Freakymate not available".to_owned()));
        assert_eq!(host.upgrade(Some("freakymate")), Ok(()));
    }

    #[test]
    fn gcode_store_returns_latest_entries() {
        let mut host = Host::default();
        host.record("command", "G28", 1.0);
        host.record("response", "ok", 2.0);

        let latest = host.gcode_store(Some(1));

        assert_eq!(latest["gcode_store"], json!([{ "message": "ok", "time": 2.0, "type": "response" }]));
    }
}
