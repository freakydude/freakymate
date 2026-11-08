//! When the printer wakes a dark screen: something needs a look (pause, error, shutdown, lost connection), or a
//! print starts or ends. The standby itself lives in [`crate::backlight`]; this only reads the printer.

use super::pause::{interruption, Interruption};
use crate::moonraker::ConnectionState;
use crate::sources::klipper::{KlippyState, PrintState};
use crate::sources::moonraker_server::gcode_store::ConsoleLine;
use crate::sources::PrinterStatus;

/// The parts of the printer state whose change can wake the screen.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct WakeFacts {
    connection: ConnectionState,
    klippy: KlippyState,
    print: PrintState,
    interruption: Option<Interruption>,
}

impl WakeFacts {
    pub fn read(connection: ConnectionState, status: &PrinterStatus) -> Self {
        Self { connection, klippy: status.klippy_state, print: status.job.state, interruption: interruption(status) }
    }

    /// A print runs or is paused: the standby uses its print timeout.
    pub fn printing(&self) -> bool {
        matches!(self.print, PrintState::Printing | PrintState::Paused)
    }

    /// Why going from `before` to these facts needs a look at the screen, for the log; `None` when nothing does.
    /// Start and end of a print wake through [`printing`](Self::printing) already; this covers the rest.
    pub fn wake_after(&self, before: &Self) -> Option<String> {
        if self.connection != before.connection && self.connection != ConnectionState::Connected {
            return Some(format!("connection {:?}", self.connection));
        }
        if self.klippy != before.klippy && matches!(self.klippy, KlippyState::Shutdown | KlippyState::Error) {
            return Some(format!("Klipper {:?}", self.klippy));
        }
        let print_needs_look =
            matches!(self.print, PrintState::Paused | PrintState::Error | PrintState::Complete | PrintState::Cancelled);
        if self.print != before.print && print_needs_look {
            return Some(format!("print {:?}", self.print));
        }
        // A new reason during the same pause (for example Happy Hare after a runout).
        if self.interruption.is_some() && self.interruption != before.interruption {
            return Some("new reason for the pause".to_owned());
        }
        None
    }
}

/// An error answer of Klipper (`!!`) on the console wakes the screen; replies to commands sent from here included.
/// Returns the reason for the log.
pub fn console_line_wake(line: &ConsoleLine) -> Option<String> {
    line.is_error().then(|| format!("console error: {}", line.text()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sources::happy_hare::MmuStatus;
    use crate::sources::klipper::PrintJob;

    fn status(klippy: KlippyState, print: PrintState) -> PrinterStatus {
        PrinterStatus {
            klippy_state: klippy,
            job: PrintJob { state: print, ..PrintJob::default() },
            ..PrinterStatus::default()
        }
    }

    fn facts(klippy: KlippyState, print: PrintState) -> WakeFacts {
        WakeFacts::read(ConnectionState::Connected, &status(klippy, print))
    }

    #[test]
    fn printing_includes_the_pause() {
        assert!(facts(KlippyState::Ready, PrintState::Printing).printing());
        assert!(facts(KlippyState::Ready, PrintState::Paused).printing());
        assert!(!facts(KlippyState::Ready, PrintState::Complete).printing());
        assert!(!facts(KlippyState::Ready, PrintState::Standby).printing());
    }

    #[test]
    fn pause_error_and_end_of_a_print_wake() {
        let printing = facts(KlippyState::Ready, PrintState::Printing);
        for state in [PrintState::Paused, PrintState::Error, PrintState::Complete, PrintState::Cancelled] {
            assert!(facts(KlippyState::Ready, state).wake_after(&printing).is_some(), "{state:?}");
        }
        assert!(facts(KlippyState::Ready, PrintState::Printing).wake_after(&printing).is_none());
        assert!(printing.wake_after(&facts(KlippyState::Ready, PrintState::Paused)).is_none(), "resuming is no reason");
    }

    #[test]
    fn klipper_shutdown_and_lost_connection_wake() {
        let ready = facts(KlippyState::Ready, PrintState::Standby);
        assert!(facts(KlippyState::Shutdown, PrintState::Standby).wake_after(&ready).is_some());
        assert!(facts(KlippyState::Error, PrintState::Standby).wake_after(&ready).is_some());
        assert!(
            ready.wake_after(&facts(KlippyState::Startup, PrintState::Standby)).is_none(),
            "coming back is no reason"
        );
        let offline = WakeFacts::read(ConnectionState::Disconnected, &status(KlippyState::Ready, PrintState::Standby));
        assert!(offline.wake_after(&ready).is_some());
        assert!(offline.wake_after(&offline).is_none(), "only the change wakes, not every snapshot");
    }

    #[test]
    fn new_reason_during_a_pause_wakes() {
        let mut paused = status(KlippyState::Ready, PrintState::Paused);
        let before = WakeFacts::read(ConnectionState::Connected, &paused);
        paused.mmu = Some(MmuStatus {
            version: "4.1.0".into(),
            enabled: true,
            reason_for_pause: "Runout on gate 1".into(),
            ..MmuStatus::default()
        });
        let after = WakeFacts::read(ConnectionState::Connected, &paused);
        assert!(after.wake_after(&before).is_some());
        assert!(after.wake_after(&after).is_none());
    }
}
