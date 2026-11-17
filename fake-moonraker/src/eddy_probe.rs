//! The setup of an Eddy current probe (`[probe_eddy_current eddy]`), simplified: what the file holds, the three setup
//! steps and what they leave for `SAVE_CONFIG`. Source: Klipper master 29d4ec2d, `klippy/extras/probe_eddy_current.py`
//! (pe), `ldc1612.py` (ldc) and `docs/Eddy_Probe.md`. Measured values are made up, and each step takes no time here
//! (Klipper's Z heights take minutes).

use crate::gcode::CommandError;
use serde_json::{Map, Value, json};

/// The section as Klipper names the object: `probe_eddy_current <CHIP>`.
pub const CHIP: &str = "eddy";
const SECTION: &str = "probe_eddy_current eddy";
/// `x_offset` and `y_offset` of the sensor to the nozzle in mm: made up (the options exist in pe; the values are
/// Frank's-Voron-like guesses).
const OFFSET: [f64; 2] = [-21.0, 0.0];
/// What the configuration holds at the start: made-up measurement results (the options are those named in
/// Eddy_Probe.md).
const START_DRIVE_CURRENT: &str = "15";
const MEASURED_DRIVE_CURRENT: &str = "16";
const MEASURED_TAP_THRESHOLD: &str = "0.582";
/// Points of the `calibrate` table: three per line, each `z:frequency` (pe l.118-132, `apply_calibration`).
const TABLE_POINTS: usize = 48;
const TABLE_START_Z: f64 = 0.05;
const TABLE_STEP_Z: f64 = 0.15;
const TABLE_START_FREQUENCY: f64 = 3_300_000.0;
const TABLE_STEP_FREQUENCY: f64 = 4_200.0;
/// Klipper's answer to a `TAP=refine` or `verify` without the step before (spec of pe l.402-439; the exact
/// conditions are simplified to the order guess, refine, verify).
const TAP_ORDER_ERROR: &str = "Must complete valid 'tap' probe first";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TapStep {
    Guess,
    Refine,
    Verify,
}

impl TapStep {
    pub fn parse(value: &str) -> Result<Self, CommandError> {
        match value.to_ascii_lowercase().as_str() {
            "guess" => Ok(Self::Guess),
            "refine" => Ok(Self::Refine),
            "verify" => Ok(Self::Verify),
            other => Err(CommandError::new(format!("Invalid TAP value '{other}'"))),
        }
    }
}

/// The options of the section in the file (the raw text) and how far the tap calibration got; like Klipper
/// the latter lives only in memory (pe l.338).
#[derive(Debug, Clone, PartialEq)]
pub struct EddyProbe {
    drive_current: String,
    calibrate: String,
    tap_threshold: Option<String>,
    tap_step: Option<TapStep>,
}

impl Default for EddyProbe {
    fn default() -> Self {
        Self {
            drive_current: START_DRIVE_CURRENT.to_owned(),
            calibrate: calibration_table(),
            tap_threshold: None,
            tap_step: None,
        }
    }
}

impl EddyProbe {
    /// The mux commands name the probe with `CHIP` (pe l.112; ldc l.43-74); Klipper rejects another value.
    pub fn check_chip(chip: &str) -> Result<(), CommandError> {
        if chip == CHIP { Ok(()) } else { Err(CommandError::new(format!("The value '{chip}' is not valid for CHIP"))) }
    }

    /// `LDC_CALIBRATE_DRIVE_CURRENT`: what it measured waits for `SAVE_CONFIG` (`reg_drive_current`).
    pub fn measure_drive_current() -> (&'static str, String) {
        ("reg_drive_current", MEASURED_DRIVE_CURRENT.to_owned())
    }

    /// The `calibrate` table a finished `PROBE_EDDY_CURRENT_CALIBRATE` leaves for `SAVE_CONFIG`.
    pub fn measured_table() -> (&'static str, String) {
        ("calibrate", calibration_table())
    }

    /// One step of `PROBE_EDDY_CURRENT_TAP_CALIBRATE`; only `verify` leaves `tap_threshold` for `SAVE_CONFIG`
    /// (pe l.393-400).
    pub fn tap(&mut self, step: TapStep) -> Result<Option<(&'static str, String)>, CommandError> {
        let done = match step {
            TapStep::Guess => None,
            TapStep::Refine => Some(TapStep::Guess),
            TapStep::Verify => Some(TapStep::Refine),
        };
        if self.tap_step != done {
            return Err(CommandError::new(TAP_ORDER_ERROR));
        }
        self.tap_step = (step != TapStep::Verify).then_some(step);
        Ok((step == TapStep::Verify).then(|| ("tap_threshold", MEASURED_TAP_THRESHOLD.to_owned())))
    }

    /// `SAVE_CONFIG` writes the pending options into the file.
    pub fn write(&mut self, options: &Map<String, Value>) {
        let text = |option: &str| options.get(option).and_then(Value::as_str).map(str::to_owned);
        if let Some(value) = text("reg_drive_current") {
            self.drive_current = value;
        }
        if let Some(value) = text("calibrate") {
            self.calibrate = value;
        }
        if let Some(value) = text("tap_threshold") {
            self.tap_threshold = Some(value);
        }
    }

    pub fn section_name() -> &'static str {
        SECTION
    }

    /// `configfile.settings`: what the file holds and the defaults of the options it lacks (the defaults are an
    /// assumption: `tap_threshold` 0 means no tap).
    pub fn settings(&self) -> Value {
        json!({
            "x_offset": OFFSET[0],
            "y_offset": OFFSET[1],
            "reg_drive_current": self.drive_current.parse::<i64>().unwrap_or_default(),
            "calibrate": self.calibrate,
            "tap_threshold": self.tap_threshold.as_deref().and_then(|text| text.parse::<f64>().ok()).unwrap_or(0.0),
        })
    }

    /// `configfile.config`: the raw text of the file, so only the options written there.
    pub fn raw(&self) -> Value {
        let mut options = Map::new();
        options.insert("x_offset".to_owned(), json!(OFFSET[0].to_string()));
        options.insert("y_offset".to_owned(), json!(OFFSET[1].to_string()));
        options.insert("reg_drive_current".to_owned(), json!(self.drive_current));
        options.insert("calibrate".to_owned(), json!(self.calibrate));
        if let Some(threshold) = &self.tap_threshold {
            options.insert("tap_threshold".to_owned(), json!(threshold));
        }
        Value::Object(options)
    }
}

fn calibration_table() -> String {
    let points: Vec<String> = (0..TABLE_POINTS)
        .map(|index| {
            let z = TABLE_START_Z + index as f64 * TABLE_STEP_Z;
            let frequency = TABLE_START_FREQUENCY + index as f64 * TABLE_STEP_FREQUENCY;
            format!("{z:.6}:{frequency:.3}")
        })
        .collect();
    let lines: Vec<String> = points.chunks(3).map(|line| line.join(",")).collect();
    format!("\n{}", lines.join(",\n"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_tap_steps_must_come_in_order_and_only_verify_sets_the_threshold() {
        let mut probe = EddyProbe::default();
        assert!(probe.tap(TapStep::Refine).is_err());
        assert_eq!(probe.tap(TapStep::Guess), Ok(None));
        assert!(probe.tap(TapStep::Verify).is_err());
        assert_eq!(probe.tap(TapStep::Refine), Ok(None));
        assert_eq!(probe.tap(TapStep::Verify), Ok(Some(("tap_threshold", "0.582".to_owned()))));
        assert!(probe.tap(TapStep::Refine).is_err());
    }

    #[test]
    fn the_file_text_lacks_the_tap_threshold_until_it_is_saved_while_the_settings_list_the_default() {
        let mut probe = EddyProbe::default();
        assert!(probe.raw().get("tap_threshold").is_none());
        assert_eq!(probe.settings()["tap_threshold"], 0.0);
        let mut options = Map::new();
        options.insert("tap_threshold".to_owned(), json!("0.582"));
        probe.write(&options);
        assert_eq!(probe.raw()["tap_threshold"], "0.582");
        assert_eq!(probe.raw()["reg_drive_current"], "15");
    }

    #[test]
    fn the_table_has_forty_eight_points_three_to_a_line() {
        let table = calibration_table();
        assert_eq!(table.split([',', '\n']).filter(|point| point.contains(':')).count(), TABLE_POINTS);
        assert!(table.starts_with('\n'));
    }

    #[test]
    fn another_chip_is_rejected() {
        assert!(EddyProbe::check_chip("eddy").is_ok());
        assert!(EddyProbe::check_chip("other").is_err());
    }
}
