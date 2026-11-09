//! Settings > Network: list, connecting, confirmation, read-only fallback.

use super::ask;
use crate::session::Session;
use freakymate::{Confirm, Navigation, Page};
use freakymate_core::network::{Availability, NetworkCommand, NetworkState};
use freakymate_core::Query;
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
    Ok(())
}
