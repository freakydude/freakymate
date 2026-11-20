//! The height of the layer being printed, from the Z of the G-code file (`gcode_move.gcode_position`).
//!
//! A Z hop lifts the nozzle for a moment on travel moves, so the height is the lowest Z of the last seconds. A pause
//! lifts it too (the `PAUSE` macro parks the head): the height stays as it was before, which is where the print goes
//! on. When the job ends, the height it reached stays until Klipper shows no job any more: no Klipper object keeps it,
//! and the end macros lift the head.

use super::job::{PrintJob, PrintState};
use std::collections::VecDeque;
use std::time::{Duration, Instant};

/// Longer than a Z hop on a travel move; a new layer shows this much later.
const WINDOW: Duration = Duration::from_secs(2);

/// The height a job reached is the lowest Z of this span before it ended, so the park move of a cancel macro, which
/// runs while the job still prints, does not count. If a layer was started in this span, the height is the one of the
/// layer before.
const END_WINDOW: Duration = Duration::from_secs(10);

/// The Z values of the last [`END_WINDOW`] while printing, and the height a job reached once it has ended.
#[derive(Debug, Default)]
pub struct HeightWatch {
    samples: VecDeque<(Instant, f64)>,
    /// `Some` once the job has ended: the height it reached, if it was seen printing.
    reached: Option<Option<f64>>,
}

impl HeightWatch {
    pub fn apply(&mut self, job: &mut PrintJob, z: f64, now: Instant) {
        job.height = match job.state {
            PrintState::Printing => {
                self.reached = None;
                self.samples.push_back((now, z));
                while self.samples.front().is_some_and(|(time, _)| now.duration_since(*time) > END_WINDOW) {
                    self.samples.pop_front();
                }
                self.current()
            }
            // The height before the pause stays and counts as fresh, so the parked head does not show on resume.
            PrintState::Paused => {
                self.reached = None;
                if let Some(height) = self.current() {
                    self.samples = VecDeque::from([(now, height)]);
                }
                self.current()
            }
            PrintState::Complete | PrintState::Cancelled | PrintState::Error => {
                *self.reached.get_or_insert_with(|| {
                    let reached = self.samples.iter().map(|(_, z)| *z).reduce(f64::min);
                    self.samples.clear();
                    reached
                })
            }
            PrintState::Standby => {
                self.samples.clear();
                self.reached = None;
                None
            }
        };
    }

    /// The lowest Z of the last [`WINDOW`] the job was printing.
    fn current(&self) -> Option<f64> {
        let (newest, _) = self.samples.back()?;
        self.samples.iter().filter(|(time, _)| newest.duration_since(*time) <= WINDOW).map(|(_, z)| *z).reduce(f64::min)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Run {
        watch: HeightWatch,
        start: Instant,
    }

    impl Run {
        fn new() -> Self {
            Self { watch: HeightWatch::default(), start: Instant::now() }
        }

        fn at(&mut self, millis: u64, state: PrintState, z: f64) -> Option<f64> {
            let mut job = PrintJob { state, ..PrintJob::default() };
            self.watch.apply(&mut job, z, self.start + Duration::from_millis(millis));
            job.height
        }
    }

    #[test]
    fn a_z_hop_does_not_lift_the_height() {
        let mut run = Run::new();
        run.at(0, PrintState::Printing, 2.4);

        assert_eq!(run.at(300, PrintState::Printing, 2.8), Some(2.4));
        assert_eq!(run.at(600, PrintState::Printing, 2.4), Some(2.4));
    }

    #[test]
    fn a_new_layer_shows_once_the_window_has_passed() {
        let mut run = Run::new();
        run.at(0, PrintState::Printing, 2.4);
        run.at(500, PrintState::Printing, 2.6);

        assert_eq!(run.at(1500, PrintState::Printing, 2.6), Some(2.4));
        assert_eq!(run.at(2600, PrintState::Printing, 2.6), Some(2.6));
    }

    #[test]
    fn the_height_before_a_pause_stays_while_parked_and_on_resume() {
        let mut run = Run::new();
        run.at(0, PrintState::Printing, 10.0);

        assert_eq!(run.at(60_000, PrintState::Paused, 30.0), Some(10.0));
        assert_eq!(run.at(60_500, PrintState::Printing, 30.0), Some(10.0));
        assert_eq!(run.at(61_000, PrintState::Printing, 10.0), Some(10.0));
    }

    #[test]
    fn an_ended_job_keeps_the_height_it_reached_before_the_end_macro_lifted_the_head() {
        let mut run = Run::new();
        run.at(0, PrintState::Printing, 12.4);
        run.at(1000, PrintState::Printing, 22.4);
        run.at(4000, PrintState::Printing, 22.4);

        assert_eq!(run.at(4500, PrintState::Cancelled, 22.4), Some(12.4));
        assert_eq!(run.at(60_000, PrintState::Cancelled, 22.4), Some(12.4));
    }

    #[test]
    fn a_job_cancelled_in_a_pause_keeps_the_height_of_the_pause() {
        let mut run = Run::new();
        run.at(0, PrintState::Printing, 10.0);
        run.at(60_000, PrintState::Paused, 30.0);

        assert_eq!(run.at(120_000, PrintState::Cancelled, 30.0), Some(10.0));
    }

    #[test]
    fn there_is_no_height_without_a_job_or_if_its_print_was_not_seen() {
        let mut run = Run::new();
        assert_eq!(run.at(0, PrintState::Complete, 50.0), None);

        run.at(100, PrintState::Printing, 10.0);
        assert_eq!(run.at(200, PrintState::Standby, 0.0), None);
        assert_eq!(run.at(300, PrintState::Complete, 50.0), None);
    }
}
