//! How a temperature stands to Klipper's limits: the scale from `min_temp` to `max_temp` (where Klipper shuts the
//! printer down), the zones near the top, and for a heater with a target whether it is still heating up, holds
//! the target or has left it. The UI only draws what is decided here.

use crate::sources::klipper::{Heater, TemperatureSensor};
use std::collections::HashMap;

/// A sensor or a switched-off heater turns yellow in the last quarter of its range, red in the last 8 %.
const WARNING_SHARE: f64 = 0.25;
const DANGER_SHARE: f64 = 0.08;

/// The scale starts at 0 °C at the lowest: Klipper's default `min_temp` of a `temperature_sensor` is -273.15 °C,
/// which would put room temperature into the yellow zone. Real rooms are above freezing.
const SCALE_FLOOR: f64 = 0.0;

/// What the number of a temperature shows by its colour.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Level {
    #[default]
    Normal,
    Warning,
    Danger,
}

/// The scale under a temperature and the marks on it; all values in °C.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Limits {
    pub min: f64,
    pub max: f64,
    /// Start of the yellow zone; none for a heater with a target (its scale has only the red zone).
    pub warning_from: Option<f64>,
    pub danger_from: f64,
    /// Target ± the tolerance of `verify_heater`, while a target is set.
    pub window: Option<(f64, f64)>,
    pub target: Option<f64>,
    /// Below this the nozzle does not extrude.
    pub min_extrude: Option<f64>,
    /// Lowest and highest reading since Klipper started; Klipper counts it only for a `temperature_sensor`.
    pub measured: Option<(f64, f64)>,
    pub level: Level,
}

impl Limits {
    /// The zones of a sensor or a switched-off heater; none without a usable range.
    fn zones(min: f64, max: f64) -> Option<Self> {
        let min = min.max(SCALE_FLOOR);
        let range = max - min;
        (range > 0.0).then_some(Self {
            min,
            max,
            warning_from: Some(max - WARNING_SHARE * range),
            danger_from: max - DANGER_SHARE * range,
            ..Self::default()
        })
    }

    fn level_of(&self, temperature: f64) -> Level {
        if temperature >= self.danger_from {
            Level::Danger
        } else if self.warning_from.is_some_and(|from| temperature >= from) {
            Level::Warning
        } else {
            Level::Normal
        }
    }
}

/// The limits of a sensor; none without `min_temp`/`max_temp` (a `temperature_sensor` without `max_temp`).
pub fn sensor_limits(sensor: &TemperatureSensor) -> Option<Limits> {
    let (min, max) = (sensor.min_temp?, sensor.max_temp?);
    let mut limits = Limits::zones(min, max)?;
    limits.level = sensor.temperature.map_or(Level::Normal, |temperature| limits.level_of(temperature));
    limits.measured = sensor.measured;
    Some(limits)
}

/// Where a heater with a target stands: after each change of the target it approaches it until the temperature is
/// inside the tolerance; leaving it again later is a deviation.
#[derive(Debug, Clone, Copy, PartialEq)]
struct Phase {
    target: f64,
    reached: bool,
}

/// Remembers per heater whether it has reached its target; one per UI, fed with every status.
#[derive(Debug, Default)]
pub struct HeaterWatch {
    phases: HashMap<String, Phase>,
}

impl HeaterWatch {
    /// The limits of a heater, updating what is remembered about it; none without `max_temp`.
    pub fn limits(&mut self, heater: &Heater) -> Option<Limits> {
        let max = heater.max_temp?;
        let mut limits = Limits::zones(heater.min_temp, max)?;
        if heater.target <= 0.0 {
            self.phases.remove(&heater.object);
            limits.level = limits.level_of(heater.temperature);
            return Some(limits);
        }
        let tolerance = heater.hysteresis;
        let window = (heater.target - tolerance, heater.target + tolerance);
        let inside = heater.temperature <= window.1 && (heater.cools_only || heater.temperature >= window.0);
        let phase = self.phases.entry(heater.object.clone()).or_insert(Phase { target: heater.target, reached: false });
        if phase.target != heater.target {
            *phase = Phase { target: heater.target, reached: false };
        }
        phase.reached |= inside;
        let deviating = phase.reached && !inside;
        limits.warning_from = None;
        limits.window = Some(window);
        limits.target = Some(heater.target);
        limits.min_extrude = heater.min_extrude_temp;
        limits.level = match (deviating, heater.temperature >= limits.danger_from) {
            (false, _) => Level::Normal,
            (true, false) => Level::Warning,
            (true, true) => Level::Danger,
        };
        Some(limits)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn heater(temperature: f64, target: f64) -> Heater {
        Heater {
            object: "extruder".into(),
            temperature,
            target,
            max_temp: Some(300.0),
            hysteresis: 5.0,
            ..Heater::default()
        }
    }

    fn sensor(temperature: f64) -> TemperatureSensor {
        TemperatureSensor {
            temperature: Some(temperature),
            min_temp: Some(0.0),
            max_temp: Some(100.0),
            ..Default::default()
        }
    }

    #[test]
    fn sensors_turn_yellow_in_the_last_quarter_and_red_in_the_last_eighth_percent() {
        let level = |temperature| sensor_limits(&sensor(temperature)).unwrap().level;
        assert_eq!(level(74.0), Level::Normal);
        assert_eq!(level(75.0), Level::Warning);
        assert_eq!(level(91.9), Level::Warning);
        assert_eq!(level(92.0), Level::Danger);
    }

    #[test]
    fn klippers_default_minimum_of_absolute_zero_starts_the_scale_at_freezing() {
        let mcu = TemperatureSensor { min_temp: Some(-273.15), ..sensor(22.0) };
        let limits = sensor_limits(&mcu).unwrap();
        assert_eq!((limits.min, limits.warning_from, limits.level), (0.0, Some(75.0), Level::Normal));
    }

    #[test]
    fn a_sensor_without_a_maximum_has_no_scale() {
        let chamber = TemperatureSensor { temperature: Some(40.0), ..Default::default() };
        assert_eq!(sensor_limits(&chamber), None);
    }

    #[test]
    fn heating_up_is_neutral_however_far_the_target_is() {
        let mut watch = HeaterWatch::default();
        assert_eq!(watch.limits(&heater(25.0, 280.0)).unwrap().level, Level::Normal);
        assert_eq!(watch.limits(&heater(200.0, 280.0)).unwrap().level, Level::Normal);
    }

    #[test]
    fn a_target_inside_the_red_zone_stays_neutral_while_it_holds() {
        let mut watch = HeaterWatch::default();
        watch.limits(&heater(280.0, 280.0));
        let held = watch.limits(&heater(282.0, 280.0)).unwrap();
        assert_eq!(held.level, Level::Normal);
        assert_eq!(held.window, Some((275.0, 285.0)));
    }

    #[test]
    fn leaving_the_window_after_reaching_it_is_a_deviation_and_red_in_the_red_zone() {
        let mut watch = HeaterWatch::default();
        watch.limits(&heater(200.0, 200.0));
        assert_eq!(watch.limits(&heater(150.0, 200.0)).unwrap().level, Level::Warning);
        assert_eq!(watch.limits(&heater(230.0, 200.0)).unwrap().level, Level::Warning);
        watch.limits(&heater(280.0, 280.0));
        assert_eq!(watch.limits(&heater(290.0, 280.0)).unwrap().level, Level::Danger);
    }

    #[test]
    fn a_new_target_starts_a_new_approach_and_cooling_down_is_no_deviation() {
        let mut watch = HeaterWatch::default();
        watch.limits(&heater(250.0, 250.0));
        assert_eq!(watch.limits(&heater(250.0, 200.0)).unwrap().level, Level::Normal);
        assert_eq!(watch.limits(&heater(205.0, 200.0)).unwrap().level, Level::Normal);
    }

    #[test]
    fn a_switched_off_heater_uses_the_zones_of_a_sensor() {
        let mut watch = HeaterWatch::default();
        let off = watch.limits(&heater(280.0, 0.0)).unwrap();
        assert_eq!((off.level, off.target), (Level::Danger, None));
        assert_eq!(watch.limits(&heater(100.0, 0.0)).unwrap().level, Level::Normal);
    }

    #[test]
    fn a_fan_that_cools_only_deviates_only_above_the_window() {
        let fan = |temperature| Heater { cools_only: true, ..heater(temperature, 40.0) };
        let mut watch = HeaterWatch::default();
        assert_eq!(watch.limits(&fan(30.0)).unwrap().level, Level::Normal);
        assert_eq!(watch.limits(&fan(25.0)).unwrap().level, Level::Normal);
        assert_eq!(watch.limits(&fan(50.0)).unwrap().level, Level::Warning);
    }

    #[test]
    fn a_heater_without_maximum_has_no_scale() {
        assert_eq!(HeaterWatch::default().limits(&Heater { max_temp: None, ..heater(20.0, 0.0) }), None);
    }
}
