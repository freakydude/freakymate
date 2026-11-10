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
}

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
        if next <= 0.0 || (step < 0.0 && next < self.min_temp) {
            0.0
        } else {
            next.max(self.min_temp)
        }
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
}

/// Extruder first, then bed, then additional heaters in object order.
pub(super) fn read_heaters(store: &ObjectStore) -> Vec<Heater> {
    let settings = store.object("configfile").and_then(|configfile| configfile.get("settings"));
    let heater = |name: &str, value: &Value| heater(name, value, settings.and_then(|settings| settings.get(name)));
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

fn heater(object: &str, value: &Value, settings: Option<&Value>) -> Heater {
    let settings = settings.unwrap_or(&Value::Null);
    Heater {
        object: object.to_owned(),
        label: label(object),
        temperature: number(value, "temperature"),
        target: number(value, "target"),
        power: number(value, "power"),
        min_temp: number(settings, "min_temp").max(0.0),
        max_temp: settings.get("max_temp").and_then(Value::as_f64),
    }
}

/// Object kinds that are only a temperature; `tmc2240` reports the temperature of the driver. Not
/// `temperature_host`/`temperature_combined`: Klipper adds them beside the section that uses the sensor type, so they
/// would show the same sensor twice (see `Setup::temperature_sensors`).
const SENSOR_KINDS: [&str; 2] = ["temperature_sensor", "tmc2240"];

/// In the order of [`SENSOR_KINDS`], each kind in name order.
pub(super) fn read_sensors(store: &ObjectStore) -> Vec<TemperatureSensor> {
    SENSOR_KINDS
        .iter()
        .flat_map(|kind| {
            store.names().filter(move |name| name.split_once(' ').is_some_and(|(prefix, _)| prefix == *kind))
        })
        .filter_map(|name| {
            store.object(name).map(|value| TemperatureSensor {
                object: name.to_owned(),
                label: label(name),
                temperature: value.get("temperature").and_then(Value::as_f64),
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

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
