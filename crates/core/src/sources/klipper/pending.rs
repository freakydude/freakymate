//! Heater targets and moves sent but not yet reported by Klipper.

use super::heaters::Heater;
use super::motion::Motion;
use super::moves::{AxisTarget, KLIPPER_START_DELAY, PlannedMove};
use std::collections::HashMap;
use std::time::{Duration, Instant};

/// A sent target or move that Klipper has not reported back by then is given up (e.g. rejected command).
const PENDING_TARGET_TIMEOUT: Duration = Duration::from_secs(3);
/// A G-code position closer than this to a sent one counts as reported (the script rounds to 0.0001 mm).
const REPORTED_MM: f64 = 0.001;

/// Shows a sent target at once and lets quick taps on + and − build on each other, until Klipper reports it back.
#[derive(Debug, Default)]
pub struct PendingTargets {
    targets: HashMap<String, (f64, Instant)>,
}

impl PendingTargets {
    pub fn request(&mut self, object: &str, target: f64, now: Instant) {
        self.targets.insert(object.to_owned(), (target, now));
    }

    /// Shows pending targets on the reported heaters; forgets confirmed and outdated ones.
    pub fn apply(&mut self, heaters: &mut [Heater], now: Instant) {
        self.targets.retain(|object, (target, sent)| {
            now.duration_since(*sent) < PENDING_TARGET_TIMEOUT
                && heaters.iter().any(|heater| &heater.object == object && heater.target != *target)
        });
        for heater in heaters {
            if let Some((target, _)) = self.targets.get(&heater.object) {
                heater.target = *target;
            }
        }
    }
}

/// Lets quick steps and the segments of the live pad build on each other: Klipper reports the position only a few
/// times a second, so the next move starts from where the last one was sent to. Also tracks until when the sent moves
/// keep Klipper busy, which the live pad needs to keep the queue filled.
#[derive(Debug, Default)]
pub struct PendingMoves {
    axes: [Option<(AxisTarget, Instant)>; 3],
    busy_until: Option<Instant>,
}

impl PendingMoves {
    pub fn request(&mut self, planned: &PlannedMove, now: Instant) {
        for target in &planned.targets {
            self.axes[target.axis.index()] = Some((*target, now));
        }
        // Klipper starts a move for an idle toolhead only after its start delay.
        let earliest = now + Duration::from_secs_f64(KLIPPER_START_DELAY);
        let start = self.busy_until.filter(|until| *until > earliest).unwrap_or(earliest);
        self.busy_until = Some(start + Duration::try_from_secs_f64(planned.seconds).unwrap_or_default());
    }

    /// Shows pending moves in the reported positions; forgets reported and outdated ones.
    pub fn apply(&mut self, motion: &mut Motion, now: Instant) {
        motion.queued_seconds = self.busy_until.map_or(0.0, |until| until.saturating_duration_since(now).as_secs_f64());
        for (index, pending) in self.axes.iter_mut().enumerate() {
            let Some((target, sent)) = *pending else { continue };
            let reported = (motion.gcode_position[index] - target.gcode).abs() < REPORTED_MM;
            if reported || now.duration_since(sent) >= PENDING_TARGET_TIMEOUT {
                *pending = None;
                continue;
            }
            motion.position[index] = target.toolhead;
            motion.gcode_position[index] = target.gcode;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sources::klipper::Axis;

    #[test]
    fn pending_moves_cover_the_delay_until_klipper_reports_back() {
        let start = Instant::now();
        let reported = |z: f64| Motion {
            position: [0.0, 0.0, z + 0.07, 0.0],
            gcode_position: [0.0, 0.0, z, 0.0],
            ..Motion::default()
        };
        let to = |z: f64| PlannedMove {
            targets: vec![AxisTarget { axis: Axis::Z, gcode: z, toolhead: z + 0.07 }],
            speed: 10.0,
            seconds: 0.0,
        };
        let mut pending = PendingMoves::default();
        pending.request(&to(5.1), start);
        pending.request(&to(5.2), start);

        let mut motion = reported(5.1);
        pending.apply(&mut motion, start + Duration::from_millis(200));
        assert_eq!((motion.gcode_position[2], motion.position[2]), (5.2, 5.2 + 0.07));

        let mut motion = reported(5.2);
        pending.apply(&mut motion, start + Duration::from_millis(400));
        pending.request(&to(5.3), start + Duration::from_millis(500));
        let mut motion = reported(5.2);
        pending.apply(&mut motion, start + PENDING_TARGET_TIMEOUT + Duration::from_secs(1));
        assert_eq!(motion.gcode_position[2], 5.2);
    }

    #[test]
    fn sent_moves_keep_klipper_busy_from_its_start_delay_on() {
        let start = Instant::now();
        let lasting = |seconds: f64| PlannedMove {
            targets: vec![AxisTarget { axis: Axis::X, gcode: 1.0, toolhead: 1.0 }],
            speed: 10.0,
            seconds,
        };
        let queued = |pending: &mut PendingMoves, at: Duration| {
            let mut motion = Motion::default();
            pending.apply(&mut motion, start + at);
            motion.queued_seconds
        };
        let mut pending = PendingMoves::default();
        assert_eq!(queued(&mut pending, Duration::ZERO), 0.0);
        pending.request(&lasting(0.2), start);
        assert!((queued(&mut pending, Duration::ZERO) - (KLIPPER_START_DELAY + 0.2)).abs() < 1e-6);
        // A move sent while the queue is still running follows on.
        pending.request(&lasting(0.15), start + Duration::from_millis(150));
        assert!((queued(&mut pending, Duration::from_millis(150)) - (KLIPPER_START_DELAY + 0.2)).abs() < 1e-6);
        assert_eq!(queued(&mut pending, Duration::from_secs(2)), 0.0);
        // After a pause Klipper's start delay counts again.
        pending.request(&lasting(0.1), start + Duration::from_secs(2));
        assert!((queued(&mut pending, Duration::from_secs(2)) - (KLIPPER_START_DELAY + 0.1)).abs() < 1e-6);
    }

    #[test]
    fn pending_targets_cover_the_delay_until_klipper_reports_back() {
        let start = Instant::now();
        let reported = |target| vec![Heater { object: "extruder".into(), target, ..Heater::default() }];
        let mut pending = PendingTargets::default();
        pending.request("extruder", 45.0, start);
        pending.request("extruder", 50.0, start);

        let mut heaters = reported(45.0);
        pending.apply(&mut heaters, start + Duration::from_millis(200));
        assert_eq!(heaters[0].target, 50.0);

        let mut heaters = reported(50.0);
        pending.apply(&mut heaters, start + Duration::from_millis(400));
        pending.request("extruder", 55.0, start + Duration::from_millis(500));
        let mut heaters = reported(50.0);
        pending.apply(&mut heaters, start + PENDING_TARGET_TIMEOUT + Duration::from_secs(1));
        assert_eq!(heaters[0].target, 50.0);
    }
}
