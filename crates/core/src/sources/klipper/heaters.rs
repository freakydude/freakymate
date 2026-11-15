//! Heaters with a target (`extruder`, `heater_bed`, `heater_generic`, `temperature_fan`, additional extruders) and
//! temperatures without one (`temperature_sensor`, drivers that measure their own).

use super::setup::is_additional_extruder;
use super::status::label;
use crate::sources::fields::number;
use crate::sources::objects::ObjectStore;
use serde_json::Value;

/// The Klipper object of the nozzle heater.
pub const NOZZLE_HEATER: &str = "extruder";
/// The Klipper object of the bed heater.
pub const BED_HEATER: &str = "heater_bed";

/// Heater with a target temperature; `object` is the Klipper object name, e.g. `heater_generic chamber`.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Heater {
    pub object: String,
    pub label: String,
    pub temperature: f64,
    pub target: f64,
    pub power: f64,
    /// `min_temp`/`max_temp` from the Klipper configuration; Klipper rejects targets outside (except 0 = off).
    pub min_temp: f64,
    pub max_temp: Option<f64>,
    /// Klipper's `verify_heater` tolerance around the target ([`DEFAULT_HYSTERESIS`] unless configured).
    pub hysteresis: f64,
    /// A nozzle extrudes only above this (`min_extrude_temp`, Klipper's default 170); `None` for other heaters.
    pub min_extrude_temp: Option<f64>,
    /// Only cools (`temperature_fan`): staying below the target is no deviation.
    pub cools_only: bool,
    /// Moonraker keeps a temperature history of it; see [`history_objects`].
    pub has_history: bool,
}

/// From here a `max_temp` is Klipper's placeholder for "none" (`temperature_sensor` defaults to 99999999.9).
const UNSET_MAX_TEMP: f64 = 1_000_000.0;
/// Klipper's default of `verify_heater` `hysteresis` (°C).
pub const DEFAULT_HYSTERESIS: f64 = 5.0;
/// Klipper's default of the extruder's `min_extrude_temp` (°C).
const DEFAULT_MIN_EXTRUDE_TEMP: f64 = 170.0;

impl Heater {
    /// The heater of a nozzle: Klipper's `extruder`, or `extruder1`, `extruder2`, … of a printer with several.
    pub fn is_nozzle(&self) -> bool {
        self.object == NOZZLE_HEATER || is_additional_extruder(&self.object)
    }

    /// Target after pressing + or − (`step` with sign): jumps over the forbidden range between off and
    /// `min_temp` and stops at `max_temp`.
    pub fn stepped_target(&self, step: f64) -> f64 {
        let maximum = self.max_temp.unwrap_or(f64::INFINITY);
        let next = (self.target + step).min(maximum);
        if next <= 0.0 || (step < 0.0 && next < self.min_temp) { 0.0 } else { next.max(self.min_temp) }
    }

    pub fn can_raise(&self) -> bool {
        self.max_temp.is_none_or(|maximum| self.target < maximum)
    }

    /// Klipper takes 0 (off) or a target between `min_temp` and `max_temp`.
    pub fn accepts_target(&self, target: f64) -> bool {
        target == 0.0 || (target >= self.min_temp && self.max_temp.is_none_or(|maximum| target <= maximum))
    }
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct TemperatureSensor {
    pub object: String,
    pub label: String,
    /// `None` if the sensor has no value right now, e.g. a stepper driver that is switched off.
    pub temperature: Option<f64>,
    /// `min_temp`/`max_temp` from the configuration, where it has them; Klipper shuts down outside. For a stepper
    /// driver the operating range of its datasheet (see [`DRIVER_RANGE`]).
    pub min_temp: Option<f64>,
    pub max_temp: Option<f64>,
    /// Lowest and highest reading since Klipper started (`measured_min_temp`/`measured_max_temp`, only
    /// `temperature_sensor`); none before the first reading.
    pub measured: Option<(f64, f64)>,
    /// Moonraker keeps a temperature history of it; see [`history_objects`].
    pub has_history: bool,
}

/// The objects in Moonraker's temperature store (`server.temperature_store` without monitors): Moonraker builds it
/// from Klipper's `heaters.available_sensors`, which therefore tells it as soon as Klipper is ready. The store itself
/// answers `{}` until Moonraker has set it up a moment later. Stepper drivers are only `available_monitors`.
fn history_objects(store: &ObjectStore) -> Vec<&str> {
    store
        .object("heaters")
        .and_then(|heaters| heaters.get("available_sensors"))
        .and_then(Value::as_array)
        .map(|names| names.iter().filter_map(Value::as_str).collect())
        .unwrap_or_default()
}

/// The configuration section of an object: Klipper writes the section names of `configfile.settings` in lower case,
/// while the objects keep the case of the config (`temperature_sensor M8P` is the section `temperature_sensor m8p`).
fn section_of<'a>(settings: &'a Value, object: &str) -> Option<&'a Value> {
    settings.get(object).or_else(|| settings.get(object.to_lowercase()))
}

/// Extruder first, then bed, then additional heaters in object order.
pub(super) fn read_heaters(store: &ObjectStore) -> Vec<Heater> {
    let settings = store.object("configfile").and_then(|configfile| configfile.get("settings"));
    let with_history = history_objects(store);
    let heater = |name: &str, value: &Value| {
        let section = |name: &str| settings.and_then(|settings| section_of(settings, name));
        let verify = section(&format!("verify_heater {}", name.rsplit(' ').next().unwrap_or(name)));
        heater(name, value, section(name), verify, with_history.contains(&name))
    };
    let mut heaters: Vec<Heater> = [NOZZLE_HEATER, BED_HEATER]
        .into_iter()
        .filter_map(|name| store.object(name).map(|value| heater(name, value)))
        .collect();
    heaters.extend(
        store
            .names()
            .filter(|name| is_extra_heater(name))
            .filter_map(|name| store.object(name).map(|value| heater(name, value))),
    );
    heaters
}

fn is_extra_heater(name: &str) -> bool {
    name.starts_with("heater_generic ")
        || name.starts_with("temperature_fan ")
        || (name.starts_with("extruder") && name != "extruder" && !name.contains(' '))
}

fn heater(
    object: &str,
    value: &Value,
    settings: Option<&Value>,
    verify_heater: Option<&Value>,
    has_history: bool,
) -> Heater {
    let settings = settings.unwrap_or(&Value::Null);
    let is_nozzle = object == NOZZLE_HEATER || is_additional_extruder(object);
    Heater {
        object: object.to_owned(),
        label: label(object),
        temperature: number(value, "temperature"),
        target: number(value, "target"),
        power: number(value, "power"),
        min_temp: number(settings, "min_temp").max(0.0),
        max_temp: settings.get("max_temp").and_then(Value::as_f64),
        hysteresis: verify_heater
            .and_then(|section| section.get("hysteresis"))
            .and_then(Value::as_f64)
            .unwrap_or(DEFAULT_HYSTERESIS),
        min_extrude_temp: is_nozzle
            .then(|| settings.get("min_extrude_temp").and_then(Value::as_f64).unwrap_or(DEFAULT_MIN_EXTRUDE_TEMP)),
        cools_only: object.starts_with("temperature_fan "),
        has_history,
    }
}

/// Object kinds that are only a temperature; `tmc2240` and `tmc5262` report the temperature of the driver (the other
/// TMC drivers have no temperature register, their `temperature` is always null). Not
/// `temperature_host`/`temperature_combined`: Klipper adds them beside the section that uses the sensor type, so they
/// would show the same sensor twice (see `Setup::temperature_sensors`).
const SENSOR_KINDS: [&str; 3] = ["temperature_sensor", "tmc2240", "tmc5262"];

/// Range of the stepper drivers in [`SENSOR_KINDS`], which have no `min_temp`/`max_temp`: the operating temperature
/// range of their datasheets (TMC2240 19-101548 Rev 2 11/23, TMC5262 19-102041 Rev 0 09/25: -40 to 125 °C). They shut
/// down at 165 °C (typical) at the power stages, but measure in the middle of the die; the TMC5262 sheet puts that
/// 20 to 40 °C cooler ("Thermal Protection and Shutdown"), so from 125 °C on the reading a shutdown can come.
const DRIVER_RANGE: (f64, f64) = (-40.0, 125.0);

/// In the order of [`SENSOR_KINDS`], each kind in name order.
pub(super) fn read_sensors(store: &ObjectStore) -> Vec<TemperatureSensor> {
    let with_history = history_objects(store);
    let settings = store.object("configfile").and_then(|configfile| configfile.get("settings"));
    let setting = |name: &str, key: &str| {
        settings
            .and_then(|settings| section_of(settings, name))
            .and_then(|section| section.get(key))
            .and_then(Value::as_f64)
    };
    // Klipper fills a missing `max_temp` of a sensor with 99999999.9: that is no limit.
    let maximum = |name: &str| setting(name, "max_temp").filter(|maximum| *maximum < UNSET_MAX_TEMP);
    SENSOR_KINDS
        .iter()
        .flat_map(|kind| {
            store
                .names()
                .filter(move |name| name.split_once(' ').is_some_and(|(prefix, _)| prefix == *kind))
                .map(move |name| (*kind, name))
        })
        .filter_map(|(kind, name)| {
            let datasheet = (kind != "temperature_sensor").then_some(DRIVER_RANGE);
            store.object(name).map(|value| TemperatureSensor {
                min_temp: setting(name, "min_temp").or(datasheet.map(|range| range.0)),
                max_temp: maximum(name).or(datasheet.map(|range| range.1)),
                object: name.to_owned(),
                label: label(name),
                temperature: value.get("temperature").and_then(Value::as_f64),
                measured: measured_range(value),
                has_history: with_history.contains(&name),
            })
        })
        .collect()
}

/// Klipper starts `measured_min_temp` at 99999999 and `measured_max_temp` at 0 (`extras/temperature_sensor.py`), so
/// until the first reading the minimum lies above the maximum.
fn measured_range(value: &Value) -> Option<(f64, f64)> {
    let lowest = value.get("measured_min_temp").and_then(Value::as_f64)?;
    let highest = value.get("measured_max_temp").and_then(Value::as_f64)?;
    (lowest <= highest).then_some((lowest, highest))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_settings_of_an_object_with_capitals_are_found_in_lower_case() {
        let mut store = ObjectStore::default();
        store.replace(&json!({
            "temperature_sensor M8P": { "temperature": 40.0 },
            "heater_generic Dryer": { "temperature": 30.0, "target": 0.0, "power": 0.0 },
            "configfile": { "settings": {
                "temperature_sensor m8p": { "min_temp": 10.0, "max_temp": 80.0 },
                "heater_generic dryer": { "min_temp": 0.0, "max_temp": 70.0 },
                "verify_heater dryer": { "hysteresis": 3.0 },
            } }
        }));
        assert_eq!(read_sensors(&store)[0].max_temp, Some(80.0));
        let dryer = &read_heaters(&store)[0];
        assert_eq!((dryer.max_temp, dryer.hysteresis), (Some(70.0), 3.0));
    }

    #[test]
    fn the_measured_range_counts_from_the_first_reading() {
        let mut store = ObjectStore::default();
        store.replace(&json!({
            // Values of a Voron (2026-11-15); a fresh sensor still has Klipper's start values.
            "temperature_sensor EBB": { "temperature": 87.41, "measured_min_temp": 60.51, "measured_max_temp": 87.66 },
            "temperature_sensor fresh": { "temperature": 0.0, "measured_min_temp": 99999999.0, "measured_max_temp": 0.0 },
        }));
        let measured: Vec<_> = read_sensors(&store).iter().map(|sensor| sensor.measured).collect();
        assert_eq!(measured, [Some((60.51, 87.66)), None]);
    }

    #[test]
    fn a_sensor_with_klippers_placeholder_maximum_has_no_limit() {
        let mut store = ObjectStore::default();
        store.replace(&json!({
            "temperature_sensor chamber": { "temperature": 30.0 },
            "temperature_sensor mcu": { "temperature": 40.0 },
            "configfile": { "settings": {
                "temperature_sensor chamber": { "min_temp": 10.0, "max_temp": 99999999.9 },
                "temperature_sensor mcu": { "min_temp": 10.0, "max_temp": 80.0 },
            } }
        }));
        let sensors = read_sensors(&store);
        let limits: Vec<_> = sensors.iter().map(|sensor| (sensor.object.as_str(), sensor.max_temp)).collect();
        assert_eq!(limits, [("temperature_sensor chamber", None), ("temperature_sensor mcu", Some(80.0))]);
    }

    #[test]
    fn stepper_drivers_take_the_operating_range_of_their_datasheet() {
        let mut store = ObjectStore::default();
        store.replace(&json!({
            "tmc2240 stepper_x": { "temperature": 45.0 },
            "tmc5262 stepper_y": { "temperature": null },
        }));
        let ranges: Vec<_> = read_sensors(&store).iter().map(|sensor| (sensor.min_temp, sensor.max_temp)).collect();
        assert_eq!(ranges, [(Some(-40.0), Some(125.0)); 2]);
    }

    #[test]
    fn every_extruder_heats_a_nozzle() {
        let heater = |object: &str| Heater { object: object.into(), ..Heater::default() };

        assert!(heater("extruder").is_nozzle() && heater("extruder2").is_nozzle());
        assert!(!heater("heater_bed").is_nozzle() && !heater("heater_generic extruder_box").is_nozzle());
    }
    use crate::sources::PrinterStatus;
    use serde_json::json;

    #[test]
    fn reads_heater_limits_from_the_configuration() {
        let mut store = ObjectStore::default();
        store.replace(&json!({
            "extruder": { "temperature": 20.0, "target": 0.0 },
            "heater_bed": { "temperature": 20.0, "target": 0.0 },
            "configfile": { "settings": { "extruder": { "min_temp": 10.0, "max_temp": 280.0 } } }
        }));
        let status = PrinterStatus::from_store(&store);

        let nozzle = status.heater("extruder").unwrap();
        assert_eq!((nozzle.min_temp, nozzle.max_temp), (10.0, Some(280.0)));
        let bed = status.heater("heater_bed").unwrap();
        assert_eq!((bed.min_temp, bed.max_temp), (0.0, None));
    }

    #[test]
    fn only_klippers_available_sensors_have_a_history() {
        let mut store = ObjectStore::default();
        store.replace(&json!({
            "extruder": { "temperature": 20.0, "target": 0.0 },
            "heater_bed": { "temperature": 20.0, "target": 0.0 },
            "temperature_sensor mcu": { "temperature": 40.0 },
            "tmc2240 stepper_x": { "temperature": 30.0 },
            "heaters": {
                "available_sensors": ["extruder", "heater_bed", "temperature_sensor mcu"],
                "available_monitors": ["tmc2240 stepper_x"]
            }
        }));
        let status = PrinterStatus::from_store(&store);

        assert!(status.heaters.iter().all(|heater| heater.has_history));
        let sensors: Vec<(&str, bool)> =
            status.sensors.iter().map(|sensor| (sensor.object.as_str(), sensor.has_history)).collect();
        assert_eq!(sensors, [("temperature_sensor mcu", true), ("tmc2240 stepper_x", false)]);
    }

    #[test]
    fn targets_are_accepted_off_or_between_the_limits() {
        let nozzle = Heater { min_temp: 10.0, max_temp: Some(280.0), ..Heater::default() };
        let accepted: Vec<bool> = [0.0, 5.0, 10.0, 280.0, 281.0].map(|target| nozzle.accepts_target(target)).into();
        assert_eq!(accepted, [true, false, true, true, false]);

        let without_maximum = Heater { min_temp: 0.0, max_temp: None, ..Heater::default() };
        assert!(without_maximum.accepts_target(500.0));
    }

    #[test]
    fn steps_skip_the_range_below_min_temp_and_stop_at_max_temp() {
        let heater = |target| Heater { target, min_temp: 40.0, max_temp: Some(280.0), ..Heater::default() };

        assert_eq!(heater(0.0).stepped_target(5.0), 40.0);
        assert_eq!(heater(40.0).stepped_target(5.0), 45.0);
        assert_eq!(heater(45.0).stepped_target(-10.0), 0.0);
        assert_eq!(heater(60.0).stepped_target(-10.0), 50.0);
        assert_eq!(heater(0.0).stepped_target(-5.0), 0.0);
        assert_eq!(heater(275.0).stepped_target(10.0), 280.0);
        assert_eq!(heater(300.0).stepped_target(-5.0), 280.0);
        assert!(!heater(280.0).can_raise());

        let unlimited = Heater { target: 0.0, ..Heater::default() };
        assert_eq!(unlimited.stepped_target(5.0), 5.0);
        assert!(unlimited.can_raise());
    }
}
