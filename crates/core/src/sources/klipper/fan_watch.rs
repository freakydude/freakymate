//! Fans whose tachometer has stopped measuring at a speed where it measured before.
//!
//! Many tachometers report nothing at a low PWM although the fan turns, so 0 rpm alone says nothing. A fan only counts
//! as stalled at a speed at or above the lowest one where its tachometer has measured since Klipper started.

use super::devices::Fan;
use std::collections::HashMap;
use std::time::{Duration, Instant};

/// After a change of speed a reading only counts this much later (plus `kick_start_time`): the kick start runs the fan
/// at full power, and Klipper's tachometer averages over a second.
const SETTLE_TIME: Duration = Duration::from_secs(3);
/// How long 0 rpm must last where the tachometer measured before.
const STALL_TIME: Duration = Duration::from_secs(5);

/// What each fan's tachometer has measured; forgotten with every Klipper start (a new `FanWatch`).
#[derive(Debug, Default)]
pub struct FanWatch {
    fans: HashMap<String, Measured>,
}

#[derive(Debug)]
struct Measured {
    /// The lowest speed at which the tachometer reported a speed after settling.
    measures_from: Option<f64>,
    speed: f64,
    speed_since: Instant,
    /// Since when it has reported 0 rpm at a speed at or above `measures_from`.
    silent_since: Option<Instant>,
}

impl FanWatch {
    pub fn apply(&mut self, fans: &mut [Fan], now: Instant) {
        for fan in fans.iter_mut() {
            let Some(rpm) = fan.rpm else { continue };
            let measured = self.fans.entry(fan.object.clone()).or_insert(Measured {
                measures_from: None,
                speed: fan.speed,
                speed_since: now,
                silent_since: None,
            });
            measured.observe(fan, rpm, now);
            fan.stalled = measured.silent_since.is_some_and(|since| now.duration_since(since) >= STALL_TIME);
        }
    }
}

impl Measured {
    fn observe(&mut self, fan: &Fan, rpm: f64, now: Instant) {
        if fan.speed != self.speed {
            self.speed = fan.speed;
            self.speed_since = now;
        }
        let settle = SETTLE_TIME + Duration::from_secs_f64(fan.kick_start_time.max(0.0));
        let settled = now.duration_since(self.speed_since) >= settle;
        let teaches = fan.should_turn() && rpm > 0.0 && self.measures_from.is_none_or(|from| fan.speed < from);
        if teaches && settled {
            self.measures_from = Some(fan.speed);
        }
        let silent = fan.should_turn() && rpm <= 0.0 && self.measures_from.is_some_and(|from| fan.speed >= from);
        self.silent_since = if silent { Some(self.silent_since.unwrap_or(now)) } else { None };
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sources::klipper::FanKind;

    struct Run {
        watch: FanWatch,
        start: Instant,
    }

    impl Run {
        fn new() -> Self {
            Self { watch: FanWatch::default(), start: Instant::now() }
        }

        /// Whether the fan counts as stalled at `seconds` with this speed and reading.
        fn at(&mut self, seconds: f64, speed: f64, rpm: f64) -> bool {
            let mut fans = vec![Fan {
                object: "fan_generic exhaust".into(),
                label: "Exhaust".into(),
                kind: FanKind::Generic,
                speed,
                rpm: Some(rpm),
                off_below: 0.0,
                kick_start_time: 0.5,
                heaters: Vec::new(),
                stalled: false,
            }];
            self.watch.apply(&mut fans, self.start + Duration::from_secs_f64(seconds));
            fans[0].stalled
        }
    }

    #[test]
    fn a_fan_that_never_measured_is_never_stalled() {
        let mut run = Run::new();
        run.at(0.0, 0.3, 0.0);

        assert!(!run.at(60.0, 0.3, 0.0));
    }

    #[test]
    fn silence_where_the_tachometer_measured_before_is_a_stall_after_a_while() {
        let mut run = Run::new();
        run.at(0.0, 0.5, 1400.0);
        run.at(4.0, 0.5, 1400.0);

        assert!(!run.at(10.0, 0.5, 0.0));
        assert!(!run.at(14.0, 0.5, 0.0));
        assert!(run.at(15.0, 0.5, 0.0));
        assert!(!run.at(16.0, 0.5, 1400.0), "turning again");
    }

    #[test]
    fn below_the_lowest_measured_speed_silence_is_normal() {
        let mut run = Run::new();
        run.at(0.0, 0.5, 1400.0);
        run.at(4.0, 0.5, 1400.0);
        run.at(5.0, 0.3, 0.0);

        assert!(!run.at(30.0, 0.3, 0.0));
    }

    #[test]
    fn the_kick_start_teaches_nothing() {
        let mut run = Run::new();
        // Switched on at 30 %: the kick start runs it at full power, the tachometer reports it, then nothing.
        run.at(0.0, 0.3, 2800.0);
        run.at(1.0, 0.3, 900.0);
        run.at(4.0, 0.3, 0.0);

        assert!(!run.at(30.0, 0.3, 0.0));
    }

    #[test]
    fn learned_speeds_belong_to_one_klipper_start() {
        let mut run = Run::new();
        run.at(0.0, 0.5, 1400.0);
        run.at(4.0, 0.5, 1400.0);
        run.watch = FanWatch::default();
        run.at(10.0, 0.5, 0.0);

        assert!(!run.at(30.0, 0.5, 0.0));
    }
}
