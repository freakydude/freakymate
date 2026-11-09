//! Emergency stop, firmware restart and the idle timeout.

use super::ask;
use crate::session::Session;
use freakymate::{Confirm, Navigation, Page};
use freakymate_core::sources::klipper::{Activity, KlippyState};
use freakymate_core::Command;
use slint::ComponentHandle;
use std::time::Duration;

/// A confirmation that is open when Klipper shuts down closes like a cancel.
/// The fake's idle timeout passes after about 30 s at time scale 20.
const IDLE_WAIT_LIMIT: Duration = Duration::from_secs(60);

pub(super) fn shutdown(session: &mut Session) -> Result<(), String> {
    session.window.global::<Navigation>().set_page(Page::Print);
    ask(session, "Cancel print", "The current print will be stopped and cannot be resumed.", "cancel-print", "");
    session.service.send(Command::EmergencyStop);
    session.wait_until("shutdown", |snapshot| snapshot.status.klippy_state == KlippyState::Shutdown)?;
    session.pump(Duration::from_secs(1));
    if session.window.global::<Confirm>().get_open() {
        return Err("the confirmation stayed open after Klipper shut down".into());
    }
    session.capture("klippy-shutdown")?;
    session.service.send(Command::FirmwareRestart);
    session.wait_until("ready again", |snapshot| snapshot.status.klippy_state == KlippyState::Ready)
}

pub(super) fn idle(session: &mut Session) -> Result<(), String> {
    session.wait_until_for("idle timeout", IDLE_WAIT_LIMIT, |snapshot| snapshot.status.activity == Activity::Idle)?;
    session.show_page(Page::Print, "print-idle")
}
