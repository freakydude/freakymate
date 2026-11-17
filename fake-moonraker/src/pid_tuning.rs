//! `PID_CALIBRATE`, simplified. Source: Klipper master `29d4ec2d`, `klippy/extras/pid_calibrate.py` (pc) and
//! `heaters.py` (h). Klipper heats to `TARGET`, then lets the target swing between `TARGET` and `TARGET - 2.5`
//! (pc l.58, 89, 93) until it has seen 12 peaks (pc l.105-107), switches the heater off (h l.127-132), and keeps the
//! found values for `SAVE_CONFIG` (pc l.51-56, `"%.3f"`). Klipper blocks every other command meanwhile; the fake only
//! delays the answer of the script, others are still served. Durations and values are made up.

use serde_json::{Map, Value, json};
use std::collections::BTreeMap;

/// The target falls this far below `TARGET` when the temperature has crossed it (pc l.58).
pub const SWING_DEGREES: f64 = 2.5;
/// Simulated seconds between two switches of the target; the real run takes minutes (made up).
const HALF_PERIOD_SECONDS: f64 = 4.0;
/// Switches of the target before the run ends; Klipper counts 12 peaks (pc l.106), here fewer (made up).
const SWITCHES: u32 = 10;
/// The values a run finds (made up; those of the design mockup).
const FOUND: [f64; 3] = [23.104, 1.152, 115.840];
/// The values of printer.cfg at the start for the nozzle and the bed (made up).
const NOZZLE_CONFIGURED: [f64; 3] = [21.527, 1.063, 108.982];
const BED_CONFIGURED: [f64; 3] = [54.027, 0.770, 948.182];

/// Where a running tuning stands, and what the heater has to do about it.
#[derive(Debug, Clone, PartialEq)]
pub enum Step {
    /// Leave the target as it is.
    Hold,
    /// Set the heater's target.
    Target(f64),
    /// The tuning ends: the heater is switched off and the section gets the found values.
    Done,
}

#[derive(Debug, Clone, PartialEq)]
pub struct PidRun {
    /// The name `HEATER=` takes (short name).
    pub heater: String,
    target: f64,
    switches: u32,
    seconds_since_switch: f64,
    heating: bool,
}

impl PidRun {
    pub fn new(heater: &str, target: f64) -> Self {
        Self { heater: heater.to_owned(), target, switches: 0, seconds_since_switch: 0.0, heating: true }
    }

    /// Simulated seconds the run takes from `temperature`: heating up at `rate` degrees per second, then the swings.
    pub fn duration_seconds(&self, temperature: f64, rate: f64) -> f64 {
        (self.target - temperature).max(0.0) / rate + f64::from(SWITCHES) * HALF_PERIOD_SECONDS
    }

    pub fn advance(&mut self, reading: f64, seconds: f64) -> Step {
        if self.switches == 0 && self.heating && reading < self.target {
            return Step::Target(self.target);
        }
        self.seconds_since_switch += seconds;
        if self.seconds_since_switch < HALF_PERIOD_SECONDS {
            return Step::Hold;
        }
        self.seconds_since_switch = 0.0;
        self.switches += 1;
        self.heating = !self.heating;
        if self.switches >= SWITCHES {
            return Step::Done;
        }
        Step::Target(if self.heating { self.target } else { self.target - SWING_DEGREES })
    }
}

/// The PID values of printer.cfg: those at the start, then what `SAVE_CONFIG` wrote.
#[derive(Debug, Clone, Default)]
pub struct PidConfig {
    saved: BTreeMap<String, [f64; 3]>,
}

impl PidConfig {
    pub fn new() -> Self {
        let saved = [("extruder", NOZZLE_CONFIGURED), ("heater_bed", BED_CONFIGURED)]
            .into_iter()
            .map(|(section, values)| (section.to_owned(), values))
            .collect();
        Self { saved }
    }

    /// The options `configfile.settings` lists (lower-case keys) for a section; none for a heater that is not tuned.
    pub fn settings(&self, section: &str) -> Option<Value> {
        let [kp, ki, kd] = self.saved.get(section)?;
        Some(json!({ "control": "pid", "pid_kp": kp, "pid_ki": ki, "pid_kd": kd }))
    }

    /// `SAVE_CONFIG` writes the pending PID options of every section into the file.
    pub fn save(&mut self, pending: &Value) {
        for (section, options) in pending.as_object().into_iter().flatten() {
            let value = |key: &str| options.get(key).and_then(Value::as_str).and_then(|text| text.parse::<f64>().ok());
            if let (Some(kp), Some(ki), Some(kd)) = (value("pid_Kp"), value("pid_Ki"), value("pid_Kd")) {
                self.saved.insert(section.clone(), [kp, ki, kd]);
            }
        }
    }
}

/// The pending items of a finished run: `control` and the three values, spelled as `PID_CALIBRATE` sets them.
pub fn found_options() -> Map<String, Value> {
    let [kp, ki, kd] = FOUND;
    let mut options = Map::new();
    options.insert("control".to_owned(), json!("pid"));
    options.insert("pid_Kp".to_owned(), json!(format!("{kp:.3}")));
    options.insert("pid_Ki".to_owned(), json!(format!("{ki:.3}")));
    options.insert("pid_Kd".to_owned(), json!(format!("{kd:.3}")));
    options
}

/// The section a heater has in the configuration: the extruders and the bed carry their name, any other heater is a
/// `heater_generic` (`HEATER=` takes the short name).
pub fn section_of(heater: &str) -> String {
    if heater == "heater_bed" || heater.starts_with("extruder") {
        heater.to_owned()
    } else {
        format!("heater_generic {heater}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_run_heats_swings_between_target_and_target_minus_two_and_a_half_and_ends() {
        let mut run = PidRun::new("extruder", 200.0);
        assert_eq!(run.advance(150.0, 1.0), Step::Target(200.0), "heating up first");
        assert_eq!(run.advance(200.0, 1.0), Step::Hold);
        let mut targets = Vec::new();
        let mut finished = false;
        for _ in 0..100 {
            match run.advance(200.0, HALF_PERIOD_SECONDS) {
                Step::Target(target) => targets.push(target),
                Step::Done => finished = true,
                Step::Hold => {}
            }
            if finished {
                break;
            }
        }
        assert!(finished);
        assert_eq!(&targets[..3], [197.5, 200.0, 197.5]);
        assert_eq!(targets.len(), (SWITCHES - 1) as usize);
    }

    #[test]
    fn the_duration_covers_the_heating_and_the_swings() {
        let run = PidRun::new("extruder", 200.0);
        assert_eq!(run.duration_seconds(200.0, 3.0), f64::from(SWITCHES) * HALF_PERIOD_SECONDS);
        assert_eq!(run.duration_seconds(140.0, 3.0), 20.0 + f64::from(SWITCHES) * HALF_PERIOD_SECONDS);
    }

    #[test]
    fn found_values_are_pending_with_klippers_spelling_and_saving_makes_them_the_settings() {
        let mut config = PidConfig::new();
        assert_eq!(config.settings("extruder").unwrap()["pid_kp"], 21.527);
        assert_eq!(config.settings("heater_generic chamber"), None);

        let pending = json!({ "extruder": found_options() });
        assert_eq!(pending["extruder"]["pid_Kp"], "23.104");
        config.save(&pending);
        assert_eq!(config.settings("extruder").unwrap()["pid_kd"], 115.84);
    }

    #[test]
    fn a_section_is_named_after_its_heater() {
        assert_eq!(section_of("extruder1"), "extruder1");
        assert_eq!(section_of("heater_bed"), "heater_bed");
        assert_eq!(section_of("chamber"), "heater_generic chamber");
    }
}
