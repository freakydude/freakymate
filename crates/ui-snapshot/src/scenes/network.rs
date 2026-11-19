//! System > Network: list, connecting, confirmation, read-only fallback; System > Printers: list and removal.

use super::ask;
use crate::session::Session;
use sailhand::{Confirm, Navigation, Page};
use sailhand_core::Query;
use sailhand_core::network::{Availability, NetworkCommand, NetworkState};
use slint::ComponentHandle;
use std::time::Duration;

pub(super) fn network(session: &mut Session) -> Result<(), String> {
    session.window.global::<Navigation>().set_page(Page::Network);
    session.network_service.send(NetworkCommand::Refresh);
    session.pump(Duration::from_millis(500));
    session.capture("network")?;

    session.network_service.send(NetworkCommand::Connect { ssid: "Cafe Guest".into(), password: None, hidden: false });
    session.pump(Duration::from_millis(200));
    session.capture("network-connecting")?;
    session.settle_replies(Duration::from_secs(2));

    ask(session, "Remove network", "The saved password of \"Garage\" is deleted.", "forget-network", "Garage");
    session.capture("network-forget")?;
    session.window.global::<Confirm>().set_open(false);

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
