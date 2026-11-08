//! Moonraker's `power` component: switchable devices such as the printer's mains relay.

use crate::moonraker::Call;
use serde::Deserialize;
use serde_json::{json, Value};

#[derive(Debug, Clone, PartialEq, Eq, Default, Deserialize)]
pub struct PowerDevice {
    pub device: String,
    #[serde(default)]
    pub status: String,
    #[serde(default)]
    pub locked_while_printing: bool,
}

impl PowerDevice {
    pub fn is_on(&self) -> bool {
        self.status == "on"
    }
}

/// `machine.device_power.devices`.
pub fn devices_call() -> Call {
    Call::new("machine.device_power.devices")
}

pub fn parse_power_devices(result: &Value) -> Vec<PowerDevice> {
    result.get("devices").cloned().and_then(|devices| serde_json::from_value(devices).ok()).unwrap_or_default()
}

pub fn switch_device(device: &str, on: bool) -> Call {
    Call::with("machine.device_power.post_device", json!({ "device": device, "action": if on { "on" } else { "off" } }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_device_is_switched_by_name() {
        assert_eq!(switch_device("printer", false).params.unwrap(), json!({ "device": "printer", "action": "off" }));
    }

    #[test]
    fn reads_power_devices() {
        let devices = parse_power_devices(&json!({ "devices": [
            { "device": "printer", "status": "on", "locked_while_printing": true, "type": "gpio" },
            { "device": "light", "status": "off", "locked_while_printing": false, "type": "gpio" }
        ]}));

        assert_eq!(devices.len(), 2);
        assert!(devices[0].is_on() && devices[0].locked_while_printing);
        assert!(!devices[1].is_on());
    }
}
