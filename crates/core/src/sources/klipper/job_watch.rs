//! What Sailhand saw of the job over time, which `print_stats` does not keep: when the current pause began, and the
//! last job Klipper showed with a file, which a Klipper restart wipes out (`print_stats` starts anew in `standby`).

use super::job::{PrintJob, PrintState};

#[derive(Debug, Default)]
pub struct JobWatch {
    state: Option<PrintState>,
    paused_since: Option<f64>,
    last_job: Option<PrintJob>,
}

impl JobWatch {
    /// Sets when the pause of `job` began, if this watch saw it begin, and returns the last job with a file once
    /// Klipper shows none. `now` is in Unix seconds.
    pub fn apply(&mut self, job: &mut PrintJob, now: f64) -> Option<PrintJob> {
        self.paused_since = match job.state {
            PrintState::Paused if self.state == Some(PrintState::Printing) => Some(now),
            PrintState::Paused => self.paused_since,
            _ => None,
        };
        self.state = Some(job.state);
        job.paused_since = self.paused_since;
        if job.filename.is_empty() {
            self.last_job.clone()
        } else {
            self.last_job = Some(job.clone());
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn job(state: PrintState, filename: &str) -> PrintJob {
        PrintJob { state, filename: filename.into(), ..PrintJob::default() }
    }

    fn paused_since(watch: &mut JobWatch, state: PrintState, now: f64) -> Option<f64> {
        let mut job = job(state, "a.gcode");
        watch.apply(&mut job, now);
        job.paused_since
    }

    #[test]
    fn a_pause_seen_beginning_tells_since_when() {
        let mut watch = JobWatch::default();
        paused_since(&mut watch, PrintState::Printing, 100.0);

        assert_eq!(paused_since(&mut watch, PrintState::Paused, 160.0), Some(160.0));
        assert_eq!(paused_since(&mut watch, PrintState::Paused, 200.0), Some(160.0));
        assert_eq!(paused_since(&mut watch, PrintState::Printing, 300.0), None);
    }

    #[test]
    fn a_pause_found_already_running_has_no_beginning() {
        let mut watch = JobWatch::default();

        assert_eq!(paused_since(&mut watch, PrintState::Paused, 160.0), None);
        assert_eq!(paused_since(&mut watch, PrintState::Paused, 200.0), None);
    }

    #[test]
    fn the_last_job_stays_once_klipper_shows_none() {
        let mut watch = JobWatch::default();
        let mut printing = PrintJob { progress: 0.4, ..job(PrintState::Printing, "a.gcode") };
        assert_eq!(watch.apply(&mut printing, 1.0), None);

        let last = watch.apply(&mut PrintJob::default(), 2.0);

        assert_eq!(last.map(|job| (job.filename, job.progress)), Some(("a.gcode".into(), 0.4)));
    }
}
