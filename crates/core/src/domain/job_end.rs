//! The end of a job: when the job Klipper shows ended, and the job Klipper lost in a shutdown or restart. Klipper's
//! `print_stats` joined with the newest job of Moonraker's history.

use crate::sources::klipper::{PrintJob, PrintState};
use crate::sources::moonraker_server::history::FinishedJob;

/// How Klipper lost a job, from Moonraker's history: afterwards `print_stats` starts anew without it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JobLoss {
    /// `klippy_shutdown`: Klipper shut down during the print.
    Shutdown,
    /// `klippy_disconnect`: Klipper stopped or restarted during the print.
    Disconnect,
}

impl JobLoss {
    fn of(status: &str) -> Option<Self> {
        match status {
            "klippy_shutdown" => Some(Self::Shutdown),
            "klippy_disconnect" => Some(Self::Disconnect),
            _ => None,
        }
    }
}

/// The job the print page shows.
#[derive(Debug, Clone, PartialEq)]
pub struct ShownJob {
    pub job: PrintJob,
    /// `Some` for a job Klipper lost; the job then is the history's record of it, failed.
    pub loss: Option<JobLoss>,
    /// Whether the progress is known: of a lost job only if Sailhand saw it printing.
    pub progress_known: bool,
}

/// The job Klipper shows; if it shows none and the newest job of the history was lost by Klipper, that one, with
/// the progress, layers and height Sailhand saw last of it (`previous`). It stays until the next job, as an ended job
/// does in Klipper. Moonraker restarts (`interrupted`, `server_exit`) are left out: Klipper may print on through them.
pub fn shown_job(job: &PrintJob, previous: Option<&PrintJob>, newest: Option<&FinishedJob>) -> ShownJob {
    let live = || ShownJob { job: job.clone(), loss: None, progress_known: true };
    if job.state != PrintState::Standby || !job.filename.is_empty() {
        return live();
    }
    let Some((newest, loss)) = newest.and_then(|newest| Some((newest, JobLoss::of(&newest.status)?))) else {
        return live();
    };
    let seen = previous.filter(|previous| previous.filename == newest.filename);
    let lost = PrintJob {
        state: PrintState::Error,
        filename: newest.filename.clone(),
        print_duration: newest.print_duration,
        total_duration: newest.total_duration,
        filament_used: newest.filament_used,
        progress: seen.map_or(0.0, |seen| seen.progress),
        current_layer: seen.and_then(|seen| seen.current_layer),
        total_layer: seen.and_then(|seen| seen.total_layer),
        height: seen.and_then(|seen| seen.height),
        ..PrintJob::default()
    };
    ShownJob { job: lost, loss: Some(loss), progress_known: seen.is_some() }
}

/// When `job` ended, if the newest history entry is that job: the same file and an outcome that matches its state.
/// Every outcome besides completed and cancelled is a failure (shutdown, disconnect, server exit).
pub fn ended_at(job: &PrintJob, newest: &FinishedJob) -> Option<f64> {
    let matches = match job.state {
        PrintState::Complete => newest.status == "completed",
        PrintState::Cancelled => newest.status == "cancelled",
        PrintState::Error => !matches!(newest.status.as_str(), "completed" | "cancelled" | "in_progress"),
        _ => false,
    };
    (matches && newest.filename == job.filename).then_some(newest.end_time)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sources::moonraker_server::history::parse_print_history;
    use serde_json::json;

    fn job(state: PrintState, filename: &str) -> PrintJob {
        PrintJob { state, filename: filename.into(), ..PrintJob::default() }
    }

    #[test]
    fn the_newest_job_tells_when_the_shown_job_ended() {
        let history = parse_print_history(&json!({ "jobs": [
            { "filename": "a.gcode", "status": "cancelled", "end_time": 1_700_000_600.0 },
            { "filename": "a.gcode", "status": "completed", "end_time": 1_600_000_000.0 }
        ] }));
        let newest = history.newest.unwrap();

        assert_eq!(ended_at(&job(PrintState::Cancelled, "a.gcode"), &newest), Some(1_700_000_600.0));
        assert_eq!(ended_at(&job(PrintState::Complete, "a.gcode"), &newest), None);
        assert_eq!(ended_at(&job(PrintState::Cancelled, "b.gcode"), &newest), None);
    }

    #[test]
    fn every_other_outcome_is_the_end_of_a_failed_job() {
        let newest = finished("a.gcode", "klippy_shutdown");

        assert_eq!(ended_at(&job(PrintState::Error, "a.gcode"), &newest), Some(5.0));
    }

    #[test]
    fn a_running_job_has_not_ended() {
        let newest = finished("a.gcode", "in_progress");

        assert_eq!(ended_at(&job(PrintState::Printing, "a.gcode"), &newest), None);
    }

    fn finished(filename: &str, status: &str) -> FinishedJob {
        FinishedJob {
            filename: filename.into(),
            status: status.into(),
            end_time: 5.0,
            print_duration: 600.0,
            total_duration: 900.0,
            filament_used: 2000.0,
            spool_ids: Vec::new(),
        }
    }

    #[test]
    fn a_job_lost_in_a_shutdown_shows_as_failed_with_what_was_seen_of_it() {
        let seen = PrintJob {
            progress: 0.4,
            current_layer: Some(30),
            height: Some(6.2),
            ..job(PrintState::Printing, "a.gcode")
        };

        let shown = shown_job(&PrintJob::default(), Some(&seen), Some(&finished("a.gcode", "klippy_shutdown")));

        assert_eq!(shown.loss, Some(JobLoss::Shutdown));
        assert!(shown.progress_known);
        assert_eq!(shown.job.state, PrintState::Error);
        assert_eq!(shown.job.filename, "a.gcode");
        assert_eq!((shown.job.print_duration, shown.job.total_duration), (600.0, 900.0));
        assert_eq!((shown.job.progress, shown.job.current_layer, shown.job.height), (0.4, Some(30), Some(6.2)));
    }

    #[test]
    fn a_lost_job_not_seen_printing_has_no_progress() {
        let other = job(PrintState::Printing, "b.gcode");

        let shown = shown_job(&PrintJob::default(), Some(&other), Some(&finished("a.gcode", "klippy_disconnect")));

        assert_eq!(shown.loss, Some(JobLoss::Disconnect));
        assert!(!shown.progress_known);
        assert_eq!(shown.job.height, None);
    }

    #[test]
    fn klippers_own_job_and_other_outcomes_show_as_they_are() {
        let lost = finished("a.gcode", "klippy_shutdown");
        let cancelled = job(PrintState::Cancelled, "b.gcode");

        assert_eq!(shown_job(&cancelled, None, Some(&lost)).job, cancelled);
        assert_eq!(shown_job(&PrintJob::default(), None, Some(&finished("a.gcode", "error"))).loss, None);
        assert_eq!(shown_job(&PrintJob::default(), None, Some(&finished("a.gcode", "interrupted"))).loss, None);
        assert_eq!(shown_job(&PrintJob::default(), None, None).loss, None);
    }
}
