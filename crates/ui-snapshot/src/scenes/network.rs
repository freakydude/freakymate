//! System > Network: list, connecting, confirmation, read-only fallback; System > Printers: list and removal.

use super::ask;
use crate::session::Session;
use sailhand::{Confirm, Navigation, Page};
use sailhand_core::Query;
use sailhand_core::network::{
    Adapter, AdapterKind, AdapterState, Availability, NetworkCommand, NetworkState, WifiNetwork,
};
use slint::ComponentHandle;
use std::time::Duration;

pub(super) fn network(session: &mut Session) -> Result<(), String> {
    session.window.global::<Navigation>().set_page(Page::Network);
    session.network_service.send(NetworkCommand::Refresh);
    session.pump(Duration::from_millis(500));
    session.capture("network")?;

    session.network_service.send(NetworkCommand::Connect {
        interface: "wlan0".into(),
        ssid: "Cafe Guest".into(),
        password: None,
        hidden: false,
    });
    session.pump(Duration::from_millis(200));
    session.capture("network-connecting")?;
    session.settle_replies(Duration::from_secs(2));

    ask(session, "Remove network", "The saved password of \"Garage\" is deleted.", "forget-network", "Garage");
    session.capture("network-forget")?;
    session.window.global::<Confirm>().set_open(false);

    session.network.apply_state(several_adapters("wlan0"));
    session.pump(Duration::from_millis(200));
    session.capture("network-adapters")?;
    session.network.apply_state(several_adapters("end0"));
    session.pump(Duration::from_millis(200));
    session.capture("network-ethernet")?;

    session.network.apply_state(NetworkState {
        availability: Availability::Unavailable("NetworkManager not reachable".into()),
        ..NetworkState::default()
    });
    session.service.query(Query::HostNetwork);
    session.pump(Duration::from_secs(1));
    session.capture("network-unavailable")?;
    session.network_service.send(NetworkCommand::Refresh);
    printers(session)
}

/// Two Wi-Fi and two Ethernet adapters (assumed names: a USB Wi-Fi stick and a USB Ethernet adapter next to the
/// built-in ones), as NetworkManager would report them; `selected` is shown on the right.
fn several_adapters(selected: &str) -> NetworkState {
    let adapter = |interface: &str, kind, state, ssid: Option<&str>, address: Option<&str>| Adapter {
        interface: interface.into(),
        kind,
        state,
        ssid: ssid.map(Into::into),
        address: address.map(Into::into),
        prefix: address.map(|_| 24),
        gateway: address.map(|_| "192.168.1.1".into()),
        ..Adapter::default()
    };
    let network = |ssid: &str, signal, active, active_on_other: Option<&str>| WifiNetwork {
        ssid: ssid.into(),
        signal,
        known: true,
        active,
        in_range: true,
        active_on_other: active_on_other.map(Into::into),
        ..WifiNetwork::default()
    };
    NetworkState {
        availability: Availability::Managed,
        wifi_enabled: true,
        adapters: vec![
            adapter("wlan0", AdapterKind::Wifi, AdapterState::Connected, Some("Workshop"), Some("192.168.1.57")),
            adapter(
                "wlx00c0ca3f12ab",
                AdapterKind::Wifi,
                AdapterState::Connected,
                Some("Garage"),
                Some("192.168.1.61"),
            ),
            Adapter {
                speed_mbit: Some(1000),
                mac: Some("02:81:3c:5e:21:7a".into()),
                ..adapter("end0", AdapterKind::Ethernet, AdapterState::Connected, None, Some("192.168.1.42"))
            },
            adapter("enx00e04c680a1b", AdapterKind::Ethernet, AdapterState::Unavailable, None, None),
        ],
        selected: Some(selected.into()),
        networks: vec![
            network("Workshop", 82, true, None),
            network("Garage", 48, false, Some("wlx00c0ca3f12ab")),
            network("Cafe Guest", 64, false, None),
        ],
        busy: None,
    }
}

fn printers(session: &mut Session) -> Result<(), String> {
    session.window.global::<Navigation>().set_page(Page::Printers);
    session.pump(Duration::from_millis(500));
    session.capture("printers")?;

    ask(
        session,
        "Remove printer",
        "The login and trusted certificate of \"ender\" are deleted.",
        "forget-printer",
        "http://192.0.2.48:7125",
    );
    session.capture("printers-forget")?;
    session.window.global::<Confirm>().set_open(false);
    Ok(())
}
