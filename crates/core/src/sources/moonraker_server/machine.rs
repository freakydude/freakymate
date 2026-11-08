//! Moonraker's `machine` component: what it reports about the printer host, and restarting its services or the host.

use crate::moonraker::Call;
use serde_json::{json, Value};

/// Network interface of the printer host as reported by Moonraker (read-only).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct HostInterface {
    pub name: String,
    pub mac_address: String,
    /// IPv4 and global IPv6 addresses.
    pub addresses: Vec<String>,
}

impl HostInterface {
    /// Linux names wireless interfaces `wlan0`, `wlp2s0`, ...
    pub fn is_wireless(&self) -> bool {
        self.name.starts_with("wl")
    }
}

/// `machine.system_info`.
pub fn system_info_call() -> Call {
    Call::new("machine.system_info")
}

pub fn parse_host_network(result: &Value) -> Vec<HostInterface> {
    let Some(network) = result.pointer("/system_info/network").and_then(Value::as_object) else {
        return Vec::new();
    };
    let mut interfaces: Vec<HostInterface> = network
        .iter()
        .map(|(name, interface)| HostInterface {
            name: name.clone(),
            mac_address: interface.get("mac_address").and_then(Value::as_str).unwrap_or_default().to_owned(),
            addresses: interface
                .get("ip_addresses")
                .and_then(Value::as_array)
                .map(|addresses| {
                    addresses
                        .iter()
                        .filter(|address| !address.get("is_link_local").and_then(Value::as_bool).unwrap_or(false))
                        .filter_map(|address| address.get("address").and_then(Value::as_str).map(str::to_owned))
                        .collect()
                })
                .unwrap_or_default(),
        })
        .collect();
    interfaces.sort_by(|left, right| left.name.cmp(&right.name));
    interfaces
}

pub fn restart_service(service: &str) -> Call {
    Call::with("machine.services.restart", json!({ "service": service }))
}

pub fn reboot() -> Call {
    Call::new("machine.reboot")
}

pub fn shutdown() -> Call {
    Call::new("machine.shutdown")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_host_network_without_link_local_addresses() {
        let interfaces = parse_host_network(&json!({ "system_info": { "network": {
            "wlan0": { "mac_address": "aa:bb", "ip_addresses": [
                { "family": "ipv4", "address": "192.168.1.127", "is_link_local": false },
                { "family": "ipv6", "address": "fe80::1", "is_link_local": true }
            ]},
            "eth0": { "mac_address": "cc:dd", "ip_addresses": [] }
        }}}));

        assert_eq!(interfaces.len(), 2);
        assert_eq!(interfaces[0].name, "eth0");
        assert_eq!(interfaces[1].addresses, ["192.168.1.127"]);
        assert_eq!(interfaces[1].mac_address, "aa:bb");
    }

    #[test]
    fn wireless_interfaces_are_recognized_by_their_linux_name() {
        let interface = |name: &str| HostInterface { name: name.to_owned(), ..HostInterface::default() };

        assert!(interface("wlan0").is_wireless() && interface("wlp3s0").is_wireless());
        assert!(!interface("eth0").is_wireless() && !interface("enp1s0").is_wireless());
    }
}
