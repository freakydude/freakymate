//! Motion and extrusion values that can be changed at runtime, until the next restart: the current value, the
//! configured one and the G-code that sets it. Commands from Klipper's G-Codes reference (`M221`,
//! `SET_VELOCITY_LIMIT`, `SET_PRESSURE_ADVANCE`, `SET_RETRACTION`), status fields from its Status Reference.

use crate::sources::PrinterStatus;
use serde_json::Value;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TuningKey {
    /// `M221`; not in the configuration, 100 % is the normal value.
    FlowFactor,
    MaxVelocity,
    MaxAccel,
    SquareCornerVelocity,
    MinimumCruiseRatio,
    PressureAdvance,
    SmoothTime,
    RetractLength,
    RetractSpeed,
    UnretractExtraLength,
    UnretractSpeed,
}

/// How a value moves on the screen: one tap changes it by `step`; it stays within `minimum` and `maximum`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TuningRange {
    pub step: f64,
    pub minimum: f64,
    pub maximum: f64,
    pub decimals: usize,
    pub unit: &'static str,
}

impl TuningKey {
    /// In the order of the Tune page: extrusion first (the flow factor sits in its Basics), then motion, then
    /// retraction.
    pub const ALL: [Self; 11] = [
        Self::FlowFactor,
        Self::PressureAdvance,
        Self::SmoothTime,
        Self::MaxVelocity,
        Self::MaxAccel,
        Self::SquareCornerVelocity,
        Self::MinimumCruiseRatio,
        Self::RetractLength,
        Self::RetractSpeed,
        Self::UnretractExtraLength,
        Self::UnretractSpeed,
    ];

    /// Stable identifier for the UI, e.g. `max_velocity`.
    pub fn id(self) -> &'static str {
        self.config_option().map_or("flow_factor", |(_, option)| option)
    }

    pub fn parse(id: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|key| key.id() == id)
    }

    pub fn range(self) -> TuningRange {
        let range = |step, minimum, maximum, decimals, unit| TuningRange { step, minimum, maximum, decimals, unit };
        match self {
            Self::FlowFactor => range(1.0, 50.0, 200.0, 0, " %"),
            Self::MaxVelocity => range(10.0, 1.0, 5000.0, 0, " mm/s"),
            Self::MaxAccel => range(500.0, 1.0, 100_000.0, 0, " mm/s²"),
            Self::SquareCornerVelocity => range(1.0, 0.0, 100.0, 1, " mm/s"),
            // Klipper needs it below 1.
            Self::MinimumCruiseRatio => range(0.05, 0.0, 0.95, 2, ""),
            Self::PressureAdvance => range(0.005, 0.0, 2.0, 3, ""),
            Self::SmoothTime => range(0.005, 0.0, 0.2, 3, " s"),
            Self::RetractLength | Self::UnretractExtraLength => range(0.1, 0.0, 20.0, 2, " mm"),
            Self::RetractSpeed | Self::UnretractSpeed => range(5.0, 1.0, 200.0, 0, " mm/s"),
        }
    }

    /// Section and option in Klipper's configuration that hold the configured value; `None` for the flow factor.
    fn config_option(self) -> Option<(&'static str, &'static str)> {
        let option = match self {
            Self::FlowFactor => return None,
            Self::MaxVelocity => ("printer", "max_velocity"),
            Self::MaxAccel => ("printer", "max_accel"),
            Self::SquareCornerVelocity => ("printer", "square_corner_velocity"),
            Self::MinimumCruiseRatio => ("printer", "minimum_cruise_ratio"),
            Self::PressureAdvance => ("extruder", "pressure_advance"),
            Self::SmoothTime => ("extruder", "pressure_advance_smooth_time"),
            Self::RetractLength => ("firmware_retraction", "retract_length"),
            Self::RetractSpeed => ("firmware_retraction", "retract_speed"),
            Self::UnretractExtraLength => ("firmware_retraction", "unretract_extra_length"),
            Self::UnretractSpeed => ("firmware_retraction", "unretract_speed"),
        };
        Some(option)
    }

    /// The value Klipper reports now; `None` if the printer does not have it (no firmware retraction).
    pub fn current(self, status: &PrinterStatus) -> Option<f64> {
        let limits = &status.motion;
        let retraction = status.retraction.as_ref();
        match self {
            Self::FlowFactor => Some(limits.extrude_factor * 100.0),
            Self::MaxVelocity => Some(limits.max_velocity),
            Self::MaxAccel => Some(limits.max_accel),
            Self::SquareCornerVelocity => Some(limits.square_corner_velocity),
            Self::MinimumCruiseRatio => Some(limits.minimum_cruise_ratio),
            Self::PressureAdvance => Some(status.extruder.pressure_advance),
            Self::SmoothTime => Some(status.extruder.smooth_time),
            Self::RetractLength => retraction.map(|retraction| retraction.retract_length),
            Self::RetractSpeed => retraction.map(|retraction| retraction.retract_speed),
            Self::UnretractExtraLength => retraction.map(|retraction| retraction.unretract_extra_length),
            Self::UnretractSpeed => retraction.map(|retraction| retraction.unretract_speed),
        }
    }

    /// The G-code that sets the value until the next restart.
    pub fn gcode(self, value: f64) -> String {
        let range = self.range();
        let value = format!("{:.*}", range.decimals.max(3), value.clamp(range.minimum, range.maximum));
        let value = value.trim_end_matches('0').trim_end_matches('.');
        match self {
            Self::FlowFactor => format!("M221 S{value}"),
            Self::MaxVelocity => format!("SET_VELOCITY_LIMIT VELOCITY={value}"),
            Self::MaxAccel => format!("SET_VELOCITY_LIMIT ACCEL={value}"),
            Self::SquareCornerVelocity => format!("SET_VELOCITY_LIMIT SQUARE_CORNER_VELOCITY={value}"),
            Self::MinimumCruiseRatio => format!("SET_VELOCITY_LIMIT MINIMUM_CRUISE_RATIO={value}"),
            Self::PressureAdvance => format!("SET_PRESSURE_ADVANCE ADVANCE={value}"),
            Self::SmoothTime => format!("SET_PRESSURE_ADVANCE SMOOTH_TIME={value}"),
            Self::RetractLength => format!("SET_RETRACTION RETRACT_LENGTH={value}"),
            Self::RetractSpeed => format!("SET_RETRACTION RETRACT_SPEED={value}"),
            Self::UnretractExtraLength => format!("SET_RETRACTION UNRETRACT_EXTRA_LENGTH={value}"),
            Self::UnretractSpeed => format!("SET_RETRACTION UNRETRACT_SPEED={value}"),
        }
    }

    /// One step up (`direction` 1) or down (-1) from the current value, within the range.
    pub fn stepped(self, current: f64, direction: f64) -> f64 {
        let range = self.range();
        let stepped = current + direction.signum() * range.step;
        // Rounded to the shown decimals, so values like 0.30000000000000004 do not creep in.
        let factor = 10f64.powi(range.decimals as i32);
        ((stepped * factor).round() / factor).clamp(range.minimum, range.maximum)
    }
}

/// The flow factor has no configuration; it is normal at 100 %.
const NORMAL_FLOW_FACTOR: f64 = 100.0;

/// The configured values, read once from `configfile.settings` (they change only with a restart).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct TuningConfig {
    values: Vec<(TuningKey, f64)>,
}

impl TuningConfig {
    pub fn from_settings(settings: Option<&Value>) -> Self {
        let values = TuningKey::ALL
            .into_iter()
            .filter_map(|key| {
                let value = match key.config_option() {
                    Some((section, option)) => settings?.get(section)?.get(option)?.as_f64()?,
                    None => NORMAL_FLOW_FACTOR,
                };
                Some((key, value))
            })
            .collect();
        Self { values }
    }

    pub fn get(&self, key: TuningKey) -> Option<f64> {
        self.values.iter().find(|(candidate, _)| *candidate == key).map(|(_, value)| *value)
    }
}

/// One value of the Tune page.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TuningValue {
    pub key: TuningKey,
    pub current: f64,
    pub configured: Option<f64>,
}

impl TuningValue {
    /// The value differs from the configuration by more than rounding.
    pub fn is_changed(&self) -> bool {
        let tolerance = 10f64.powi(-(self.key.range().decimals as i32)) / 2.0;
        self.configured.is_some_and(|configured| (configured - self.current).abs() > tolerance)
    }
}

/// The values this printer has, in the order of [`TuningKey::ALL`].
pub fn values(status: &PrinterStatus, config: &TuningConfig) -> Vec<TuningValue> {
    TuningKey::ALL
        .into_iter()
        .filter_map(|key| Some(TuningValue { key, current: key.current(status)?, configured: config.get(key) }))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sources::klipper::Retraction;
    use serde_json::json;

    #[test]
    fn gcode_uses_the_documented_commands_and_drops_trailing_zeros() {
        assert_eq!(TuningKey::MaxAccel.gcode(5000.0), "SET_VELOCITY_LIMIT ACCEL=5000");
        assert_eq!(TuningKey::MinimumCruiseRatio.gcode(0.45), "SET_VELOCITY_LIMIT MINIMUM_CRUISE_RATIO=0.45");
        assert_eq!(TuningKey::MinimumCruiseRatio.gcode(1.0), "SET_VELOCITY_LIMIT MINIMUM_CRUISE_RATIO=0.95");
        assert_eq!(TuningKey::PressureAdvance.gcode(0.035), "SET_PRESSURE_ADVANCE ADVANCE=0.035");
        assert_eq!(TuningKey::SmoothTime.gcode(0.04), "SET_PRESSURE_ADVANCE SMOOTH_TIME=0.04");
        assert_eq!(TuningKey::RetractLength.gcode(0.4), "SET_RETRACTION RETRACT_LENGTH=0.4");
        assert_eq!(TuningKey::UnretractSpeed.gcode(-3.0), "SET_RETRACTION UNRETRACT_SPEED=1");
    }

    #[test]
    fn steps_stay_on_the_grid_and_in_range() {
        assert_eq!(TuningKey::RetractLength.stepped(0.2, 1.0), 0.3);
        assert_eq!(TuningKey::PressureAdvance.stepped(0.0, -1.0), 0.0);
        assert_eq!(TuningKey::MaxAccel.stepped(5000.0, 1.0), 5500.0);
    }

    #[test]
    fn values_compare_with_the_configuration() {
        let settings = json!({
            "printer": { "max_velocity": 600.0, "max_accel": 5000.0, "square_corner_velocity": 7.0, "minimum_cruise_ratio": 0.5 },
            "extruder": { "pressure_advance": 0.03, "pressure_advance_smooth_time": 0.01 }
        });
        let config = TuningConfig::from_settings(Some(&settings));
        let mut status = PrinterStatus::default();
        status.motion.max_velocity = 600.0;
        status.motion.max_accel = 3000.0;
        status.extruder.pressure_advance = 0.0301;

        let values = values(&status, &config);

        assert_eq!(values.len(), 7, "no firmware retraction, so four values are missing");
        let of = |key| values.iter().find(|value| value.key == key).unwrap();
        assert!(!of(TuningKey::MaxVelocity).is_changed());
        assert!(of(TuningKey::MaxAccel).is_changed());
        assert!(!of(TuningKey::PressureAdvance).is_changed(), "within rounding of three decimals");

        status.retraction = Some(Retraction { retract_length: 0.4, ..Retraction::default() });
        assert_eq!(super::values(&status, &config).len(), 11);
        assert_eq!(super::values(&status, &config)[7].configured, None, "not in this configuration");
        assert_eq!(of(TuningKey::FlowFactor).configured, Some(100.0), "the flow factor is normal at 100 %");
    }

    #[test]
    fn ids_round_trip() {
        for key in TuningKey::ALL {
            assert_eq!(TuningKey::parse(key.id()), Some(key));
        }
        assert_eq!(TuningKey::SmoothTime.id(), "pressure_advance_smooth_time");
        assert_eq!(TuningKey::FlowFactor.id(), "flow_factor");
        assert_eq!(TuningKey::FlowFactor.gcode(95.0), "M221 S95");
    }
}
