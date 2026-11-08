//! Typed view of what Klipper reports, read from the [`ObjectStore`]: Klippy's own state and whether the machine is
//! busy here, each kind of object in its own module.

use super::calibration::CalibrationState;
use super::devices::{self, Fan, Led};
use super::exclude_object::{read_print_objects, PrintObject};
use super::extruder::{read_extruder, ExtruderState};
use super::filament_sensors::{read_filament_sensors, FilamentSensor};
use super::firmware_retraction::{read_retraction, Retraction};
use super::heaters::{read_heaters, read_sensors, Heater, TemperatureSensor};
use super::job::{read_job, PrintJob};
use super::leveling;
use super::motion::{read_motion, Motion};
use crate::sources::fields::{flag, text};
use crate::sources::objects::ObjectStore;
use serde_json::Value;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum KlippyState {
    #[default]
    Disconnected,
    Startup,
    Ready,
    Shutdown,
    Error,
}

impl KlippyState {
    pub fn parse(text: &str) -> Self {
        match text {
            "startup" => Self::Startup,
            "ready" => Self::Ready,
            "shutdown" => Self::Shutdown,
            "error" => Self::Error,
            _ => Self::Disconnected,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Disconnected => "disconnected",
            Self::Startup => "startup",
            Self::Ready => "ready",
            Self::Shutdown => "shutdown",
            Self::Error => "error",
        }
    }
}

/// `idle_timeout.state`: whether the machine is working, independent of a print job.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Activity {
    #[default]
    Ready,
    /// Klipper calls this "Printing": any command is being executed, e.g. homing or heating.
    Busy,
    /// Inactive for longer than the idle timeout; Klipper has usually switched off heaters and motors.
    Idle,
}

impl Activity {
    pub fn parse(text: &str) -> Self {
        match text {
            "Printing" => Self::Busy,
            "Idle" => Self::Idle,
            _ => Self::Ready,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Ready => "ready",
            Self::Busy => "busy",
            Self::Idle => "idle",
        }
    }
}

/// The parts of the state that Klipper itself reports, read from its objects.
pub(in crate::sources) struct KlipperParts {
    pub klippy_state: KlippyState,
    pub klippy_message: String,
    pub activity: Activity,
    pub job: PrintJob,
    pub motion: Motion,
    pub extruder: ExtruderState,
    pub heaters: Vec<Heater>,
    pub sensors: Vec<TemperatureSensor>,
    pub fans: Vec<Fan>,
    pub leds: Vec<Led>,
    pub retraction: Option<Retraction>,
    pub save_config_pending: bool,
    pub leveling_applied: Option<bool>,
    pub calibration: CalibrationState,
    pub filament_sensors: Vec<FilamentSensor>,
    pub print_objects: Vec<PrintObject>,
}

impl KlipperParts {
    pub(in crate::sources) fn read(store: &ObjectStore) -> Self {
        let object = |name: &str| store.object(name).unwrap_or(&Value::Null);
        let webhooks = object("webhooks");
        Self {
            klippy_state: KlippyState::parse(text(webhooks, "state")),
            klippy_message: text(webhooks, "state_message").trim().to_owned(),
            activity: Activity::parse(text(object("idle_timeout"), "state")),
            job: read_job(object("print_stats"), object("display_status"), object("virtual_sdcard")),
            motion: read_motion(object("toolhead"), object("motion_report"), object("gcode_move")),
            extruder: read_extruder(object("extruder"), object("configfile").pointer("/settings/extruder")),
            heaters: read_heaters(store),
            sensors: read_sensors(store),
            fans: devices::read_fans(store),
            leds: devices::read_leds(store),
            retraction: store.object("firmware_retraction").map(read_retraction),
            save_config_pending: flag(object("configfile"), "save_config_pending"),
            leveling_applied: leveling::read_applied(store),
            calibration: CalibrationState::read(store),
            filament_sensors: read_filament_sensors(store),
            print_objects: read_print_objects(object("exclude_object")),
        }
    }
}

/// Display name from the object name: `heater_generic chamber` → `Chamber`, `heater_bed` → `Bed`.
pub fn label(object: &str) -> String {
    let base = match object {
        "extruder" => "nozzle",
        "heater_bed" => "bed",
        other => other.split_once(' ').map_or(other, |(_, instance)| instance),
    };
    let words: Vec<String> = base
        .split(['_', ' '])
        .filter(|word| !word.is_empty())
        .map(|word| {
            let mut characters = word.chars();
            characters.next().map_or_else(String::new, |first| first.to_uppercase().chain(characters).collect())
        })
        .collect();
    words.join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sources::klipper::PrintState;
    use crate::sources::PrinterStatus;
    use serde_json::json;

    fn store(status: Value) -> ObjectStore {
        let mut store = ObjectStore::default();
        store.replace(&status);
        store
    }

    #[test]
    fn reads_job_motion_and_heaters() {
        let status = PrinterStatus::from_store(&store(json!({
            "webhooks": { "state": "ready", "state_message": "Printer is ready\n" },
            "idle_timeout": { "state": "Printing" },
            "print_stats": {
                "state": "printing", "filename": "cube.gcode", "print_duration": 60.0,
                "info": { "current_layer": 3, "total_layer": 100 }
            },
            "display_status": { "progress": 0.25 },
            "toolhead": { "position": [1.0, 2.0, 3.0, 4.0], "homed_axes": "xyz", "axis_maximum": [300.0, 300.0, 250.0, 0.0] },
            "gcode_move": { "homing_origin": [0.0, 0.0, 0.05, 0.0], "speed_factor": 1.2 },
            "extruder": { "temperature": 200.5, "target": 210.0, "pressure_advance": 0.04, "can_extrude": true },
            "heater_bed": { "temperature": 60.0, "target": 60.0 },
            "heater_generic chamber": { "temperature": 35.0, "target": 0.0 },
            "temperature_sensor mcu_temp": { "temperature": 41.0 },
            "tmc2240 stepper_x": { "temperature": null },
            "temperature_sensor pi": { "temperature": 48.5 },
            "temperature_host pi": { "temperature": 48.5 },
            "fan": { "speed": 0.5 },
            "quad_gantry_level": { "applied": true }
        })));

        assert_eq!(status.klippy_state, KlippyState::Ready);
        assert_eq!(status.klippy_message, "Printer is ready");
        assert_eq!(status.activity, Activity::Busy);
        assert_eq!(status.job.state, PrintState::Printing);
        assert_eq!(status.job.progress, 0.25);
        assert_eq!((status.job.current_layer, status.job.total_layer), (Some(3), Some(100)));
        assert!(status.motion.homed.all());
        assert_eq!(status.motion.homing_origin[2], 0.05);
        assert_eq!(status.motion.extrude_factor, 1.0);
        assert!(status.extruder.can_extrude);
        let heaters: Vec<&str> = status.heaters.iter().map(|heater| heater.label.as_str()).collect();
        assert_eq!(heaters, ["Nozzle", "Bed", "Chamber"]);
        let sensors: Vec<(&str, Option<f64>)> =
            status.sensors.iter().map(|sensor| (sensor.label.as_str(), sensor.temperature)).collect();
        assert_eq!(sensors, [("Mcu Temp", Some(41.0)), ("Pi", Some(48.5)), ("Stepper X", None)]);
        assert_eq!(status.fans[0].speed, 0.5);
        assert_eq!(status.leveling_applied, Some(true));
    }

    #[test]
    fn missing_objects_give_safe_defaults() {
        let status = PrinterStatus::from_store(&ObjectStore::default());

        assert_eq!(status.klippy_state, KlippyState::Disconnected);
        assert_eq!(status.activity, Activity::Ready);
        assert_eq!(Activity::parse("Idle"), Activity::Idle);
        assert_eq!(status.job.state, PrintState::Standby);
        assert!(status.heaters.is_empty());
        assert!(status.fans.is_empty() && status.leds.is_empty() && status.retraction.is_none());
        assert_eq!(status.leveling_applied, None);
    }

    #[test]
    fn labels_are_human_readable() {
        assert_eq!(label("extruder"), "Nozzle");
        assert_eq!(label("heater_bed"), "Bed");
        assert_eq!(label("temperature_fan electronics_fan"), "Electronics Fan");
        assert_eq!(label("extruder1"), "Extruder1");
    }
}
