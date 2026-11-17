//! PID tuning of a heater (`PID_CALIBRATE`): the target to start with, the values printer.cfg has now and the
//! values Klipper has found and keeps for `SAVE_CONFIG`, whoever ran the tuning (this display, the console,
//! Mainsail).

use crate::sources::PrinterStatus;
use crate::sources::klipper::{BED_HEATER, Heater, PidValues};

/// The target offered for a nozzle (Frank's choice, 2026-11-17).
pub const NOZZLE_TARGET_C: f64 = 230.0;
/// The target offered for the heated bed.
pub const BED_TARGET_C: f64 = 70.0;

/// What the dialog of a heater shows about PID tuning.
#[derive(Debug, Clone, PartialEq)]
pub struct PidTuning {
    /// The target to start with; none for a heater without a usual value, which the person enters.
    pub default_target: Option<f64>,
    /// The values in printer.cfg now; only a heater with `control: pid` has them.
    pub current: Option<PidValues>,
    /// The values `PID_CALIBRATE` found, waiting for `SAVE_CONFIG`.
    pub pending: Option<PidValues>,
}

impl PidTuning {
    /// `None` for what cannot be tuned: an unknown object or a `temperature_fan`, which Klipper's heater manager does
    /// not know (`PID_CALIBRATE` looks the name up there).
    pub fn of(status: &PrinterStatus, object: &str) -> Option<Self> {
        let heater = status.heater(object).filter(|heater| !heater.cools_only)?;
        Some(Self {
            default_target: default_target(heater),
            current: heater.pid,
            pending: pending_values(status, object),
        })
    }
}

/// 230 °C for a nozzle, 70 °C for the bed, nothing for another heater; kept inside what the heater takes.
pub fn default_target(heater: &Heater) -> Option<f64> {
    let usual = if heater.is_nozzle() {
        NOZZLE_TARGET_C
    } else if heater.object == BED_HEATER {
        BED_TARGET_C
    } else {
        return None;
    };
    let capped = heater.max_temp.map_or(usual, |maximum| usual.min(maximum));
    Some(capped.max(heater.min_temp))
}

/// The pending `pid_Kp`, `pid_Ki` and `pid_Kd` in the section of the heater (`save_config_pending_items`, spelled as
/// `PID_CALIBRATE` sets them: `pid_calibrate.py` l.51-56); `None` while one is missing or the control is another one.
pub fn pending_values(status: &PrinterStatus, object: &str) -> Option<PidValues> {
    let section = status.calibration.pending.iter().find(|pending| pending.section == object)?;
    let options = section.options.as_ref()?;
    let option =
        |key: &str| options.iter().find(|(name, _)| name.eq_ignore_ascii_case(key)).map(|(_, value)| value.trim());
    if option("control").is_some_and(|control| control != "pid") {
        return None;
    }
    PidValues::read(|name| option(&format!("pid_{name}"))?.parse().ok())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sources::klipper::PendingSection;

    fn heater(object: &str) -> Heater {
        Heater { object: object.into(), max_temp: Some(300.0), min_temp: 0.0, ..Heater::default() }
    }

    fn pending(section: &str, options: &[(&str, &str)]) -> PendingSection {
        let options = options.iter().map(|(name, value)| ((*name).to_owned(), (*value).to_owned())).collect();
        PendingSection { section: section.into(), options: Some(options) }
    }

    fn status(heaters: Vec<Heater>, pending: Vec<PendingSection>) -> PrinterStatus {
        let mut status = PrinterStatus { heaters, ..PrinterStatus::default() };
        status.calibration.pending = pending;
        status
    }

    #[test]
    fn the_start_target_follows_the_kind_of_heater() {
        assert_eq!(default_target(&heater("extruder")), Some(230.0));
        assert_eq!(default_target(&heater("extruder1")), Some(230.0));
        assert_eq!(default_target(&heater("heater_bed")), Some(70.0));
        assert_eq!(default_target(&heater("heater_generic chamber")), None);
    }

    #[test]
    fn the_start_target_stays_inside_what_the_heater_takes() {
        let low = Heater { max_temp: Some(120.0), ..heater("extruder") };
        assert_eq!(default_target(&low), Some(120.0));
        let high = Heater { min_temp: 250.0, ..heater("extruder") };
        assert_eq!(default_target(&high), Some(250.0));
    }

    #[test]
    fn a_pending_result_is_read_whoever_started_the_tuning() {
        let options = [("control", "pid"), ("pid_Kp", "23.104"), ("pid_Ki", "1.152"), ("pid_Kd", "115.840")];
        let current = PidValues { kp: 21.527, ki: 1.063, kd: 108.982 };
        let status = status(
            vec![Heater { pid: Some(current), ..heater("extruder") }, heater("heater_bed")],
            vec![pending("extruder", &options), pending("stepper_x", &[("position_endstop", "1")])],
        );

        let tuning = PidTuning::of(&status, "extruder").unwrap();
        assert_eq!(tuning.current, Some(current));
        assert_eq!(tuning.pending, Some(PidValues { kp: 23.104, ki: 1.152, kd: 115.84 }));
        assert_eq!(PidTuning::of(&status, "heater_bed").unwrap().pending, None, "another heater's section");
    }

    #[test]
    fn an_incomplete_or_foreign_pending_item_is_no_result() {
        let missing = pending("extruder", &[("control", "pid"), ("pid_Kp", "23.1"), ("pid_Ki", "1.1")]);
        assert_eq!(pending_values(&status(vec![], vec![missing]), "extruder"), None);
        let other = pending("extruder", &[("control", "watermark"), ("pid_Kp", "1"), ("pid_Ki", "1"), ("pid_Kd", "1")]);
        assert_eq!(pending_values(&status(vec![], vec![other]), "extruder"), None);
        let removed = PendingSection { section: "extruder".into(), options: None };
        assert_eq!(pending_values(&status(vec![], vec![removed]), "extruder"), None);
    }

    #[test]
    fn a_fan_that_only_cools_has_no_tuning() {
        let fan = Heater { cools_only: true, ..heater("temperature_fan case") };
        assert_eq!(PidTuning::of(&status(vec![fan], vec![]), "temperature_fan case"), None);
        assert_eq!(PidTuning::of(&PrinterStatus::default(), "extruder"), None);
    }
}
