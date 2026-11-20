//! Emergency stop, firmware restart and the idle timeout.

use super::ask;
use crate::session::Session;
use sailhand::{Confirm, Navigation, Page, Printer};
use sailhand_core::Command;
use sailhand_core::sources::klipper::{Activity, KlippyState, PrintState};
use slint::ComponentHandle;
use std::time::Duration;

/// A confirmation that is open when Klipper shuts down closes like a cancel. The print it stops is gone from Klipper
/// after the restart; the print page shows it from the history as failed.
/// The fake's idle timeout passes after about 30 s at time scale 20.
const IDLE_WAIT_LIMIT: Duration = Duration::from_secs(60);

pub(super) fn shutdown(session: &mut Session) -> Result<(), String> {
    session.service.send(Command::StartPrint { filename: "benchy.gcode".into() });
    session.wait_until("print running", |snapshot| snapshot.status.job.state == PrintState::Printing)?;
    session.pump(Duration::from_secs(2));
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
    session.wait_until("ready again", |snapshot| snapshot.status.klippy_state == KlippyState::Ready)?;
    lost_job_shows(session)?;
    session.capture("print-lost")
}

/// After the restart Klipper shows no job; the page shows the lost one once the history has arrived.
fn lost_job_shows(session: &mut Session) -> Result<(), String> {
    for _ in 0..40 {
        let printer = session.window.global::<Printer>();
        if printer.get_job_state() == "error" && printer.get_filename() == "benchy.gcode" {
            return Ok(());
        }
        session.pump(Duration::from_millis(100));
    }
    Err("the job lost in the shutdown does not show on the print page".into())
}

pub(super) fn idle(session: &mut Session) -> Result<(), String> {
    session.wait_until_for("idle timeout", IDLE_WAIT_LIMIT, |snapshot| snapshot.status.activity == Activity::Idle)?;
    session.show_page(Page::Print, "print-idle")
}
