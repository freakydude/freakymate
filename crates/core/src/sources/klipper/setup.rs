//! What this Klipper has: read from `printer.objects.list` and, once per subscription, from its configuration (it
//! only changes with a restart, which subscribes again).

use super::calibration::{self, CalibrationSetup};
use super::devices::{self, FanKind, WhiteChannel};
use super::filament_sensors::is_filament_sensor;
use super::leveling::Leveling;
use super::macros::{self, MacroDetails};
use super::tuning::TuningConfig;
use crate::sources::subscription::Subscription;
use serde_json::Value;
use std::collections::BTreeMap;
use std::sync::Arc;

const HEATER_FIELDS: &[&str] = &["temperature", "target", "power"];

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Setup {
    pub leveling: Option<Leveling>,
    /// A probe (`probe`, `bltouch`, `smart_effector`, `probe_eddy_current`): the Z offset goes to the probe, else
    /// to the endstop.
    pub probe: bool,
    /// Bed mesh, bed screws and the Z endstop, see [`CalibrationSetup`].
    pub calibration: CalibrationSetup,
    /// Additional heaters with a target temperature (`heater_generic`, `temperature_fan`, additional extruders).
    pub extra_heaters: Vec<String>,
    /// Temperatures without a target: `temperature_sensor` and the drivers that measure their own (`tmc2240`, `tmc5262`).
    /// Klipper adds a `temperature_host` or `temperature_combined` object next to the section that uses that sensor
    /// type (`temperature_sensor cb1` brings `temperature_host cb1`); those are the same sensor and left out.
    pub temperature_sensors: Vec<String>,
    pub filament_sensors: Vec<String>,
    /// `fan`, `fan_generic`, `heater_fan` and `controller_fan` objects.
    pub fans: Vec<String>,
    /// LED objects (`led`, `neopixel`, `dotstar`, `pca9533`, `pca9632`), each its own config section; one object
    /// can be a chain of several chips.
    pub leds: Vec<String>,
    /// Which chips of each LED have a white channel, from the configuration.
    pub led_white: Vec<(String, WhiteChannel)>,
    /// The configured motion and extrusion values, for the expert tab.
    pub tuning: TuningConfig,
    /// Callable macros (excluding helper macros starting with `_`), in upper case.
    pub macros: Vec<String>,
    /// Parameters of each macro by upper-case name, read once from the configuration. Shared, because every
    /// snapshot carries the setup.
    pub macro_details: Arc<BTreeMap<String, MacroDetails>>,
    /// The tool numbers Klipper has a `T<n>` command for (macros, Happy Hare, toolchanger plugins alike), from
    /// `printer.gcode.help`; `None` until asked or when the answer failed.
    pub tool_commands: Option<Vec<usize>>,
    /// Klipper has the command `UNSELECT_TOOL` (a tool changer without `require_tool_present`); from the same answer.
    pub has_unselect_tool: bool,
    /// `[save_variables]` is configured, so `SAVE_VARIABLE` keeps values across restarts.
    pub save_variables: bool,
}

impl Setup {
    pub fn from_objects<'a>(names: impl IntoIterator<Item = &'a str>) -> Self {
        let mut setup = Self::default();
        let mut leveling_objects = Vec::new();
        for name in names {
            setup.calibration.note_object(name);
            let (kind, instance) = name.split_once(' ').map_or((name, None), |(kind, instance)| (kind, Some(instance)));
            match (kind, instance) {
                (_, None) if Leveling::objects().contains(&kind) => leveling_objects.push(kind),
                ("probe" | "bltouch" | "smart_effector" | "probe_eddy_current", _) => setup.probe = true,
                ("save_variables", None) => setup.save_variables = true,
                // What is only shown leaves out helpers; what changes behaviour (probe, leveling) counts above.
                _ if devices::is_hidden(name) => {}
                ("heater_generic" | "temperature_fan", Some(_)) => setup.extra_heaters.push(name.to_owned()),
                ("temperature_sensor" | "tmc2240" | "tmc5262", Some(_)) => {
                    setup.temperature_sensors.push(name.to_owned())
                }
                _ if FanKind::of(name).is_some() => setup.fans.push(name.to_owned()),
                _ if devices::is_led(name) => setup.leds.push(name.to_owned()),
                (_, Some(_)) if is_filament_sensor(kind) => setup.filament_sensors.push(name.to_owned()),
                ("gcode_macro", Some(macro_name)) => {
                    setup.macros.push(macro_name.to_ascii_uppercase());
                }
                (extruder, None) if is_additional_extruder(extruder) => setup.extra_heaters.push(name.to_owned()),
                _ => {}
            }
        }
        setup.leveling = Leveling::detect(&leveling_objects);
        setup.macros.sort();
        setup
    }

    /// What only the configuration tells: macro parameters, LEDs with white, the configured tuning values.
    pub fn apply_settings(&mut self, settings: Option<&Value>) {
        self.macro_details = Arc::new(macros::details_from_settings(settings));
        let section = |object: &str| settings.and_then(|settings| settings.get(object));
        self.led_white = self.leds.iter().map(|led| (led.clone(), devices::white_channel(section(led)))).collect();
        self.tuning = TuningConfig::from_settings(settings);
        self.calibration.apply_settings(settings);
    }

    pub fn white_channel(&self, led: &str) -> WhiteChannel {
        self.led_white.iter().find(|(candidate, _)| candidate == led).map(|(_, white)| *white).unwrap_or_default()
    }

    /// Channel levels `[red, green, blue, white]` for white light at `level` (0 to 1) on `led`, see
    /// [`WhiteChannel::white_light`].
    pub fn white_light(&self, led: &str, level: f64) -> [f64; 4] {
        self.white_channel(led).white_light(level)
    }

    pub fn has_macro(&self, name: &str) -> bool {
        self.macros.iter().any(|candidate| candidate.eq_ignore_ascii_case(name))
    }

    /// Whether Klipper knows the command `T<tool>`; `None` without its command list.
    pub fn has_tool_command(&self, tool: usize) -> Option<bool> {
        self.tool_commands.as_ref().map(|tools| tools.contains(&tool))
    }

    /// A Klipper with these `T<n>` commands, for tests of the tools of a file.
    #[cfg(test)]
    pub(crate) fn with_tool_commands(tools: &[usize]) -> Self {
        Self { tool_commands: Some(tools.to_vec()), ..Self::default() }
    }

    /// A Klipper with nothing but these macros, for tests of the macro conventions.
    #[cfg(test)]
    pub(crate) fn with_macros(macros: &[&str]) -> Self {
        Self { macros: macros.iter().map(|name| name.to_ascii_uppercase()).collect(), ..Self::default() }
    }

    /// Klipper's own objects and fields; `configfile.settings` holds the heater limits and is sent once, it does not
    /// change at runtime.
    pub(in crate::sources) fn subscribe(&self, subscription: &mut Subscription) {
        subscription.add("webhooks", None);
        subscription.add("print_stats", None);
        subscription.add("display_status", Some(&["progress", "message"]));
        subscription.add("virtual_sdcard", Some(&["progress", "is_active"]));
        subscription.add("idle_timeout", Some(&["state"]));
        subscription.add(
            "toolhead",
            Some(&[
                "position",
                "homed_axes",
                "axis_minimum",
                "axis_maximum",
                "max_velocity",
                "max_accel",
                "square_corner_velocity",
                "minimum_cruise_ratio",
            ]),
        );
        subscription.add("motion_report", Some(&["live_position", "live_velocity", "live_extruder_velocity"]));
        subscription.add("gcode_move", Some(&["gcode_position", "homing_origin", "speed_factor", "extrude_factor"]));
        subscription.add(
            "extruder",
            Some(&["temperature", "target", "power", "pressure_advance", "smooth_time", "can_extrude"]),
        );
        subscription.add("configfile", Some(&["save_config_pending", "save_config_pending_items", "settings"]));
        subscription.add("heater_bed", Some(HEATER_FIELDS));
        for leveling in Leveling::objects() {
            subscription.add(leveling, Some(&["applied"]));
        }
        for object in calibration::OBJECTS {
            // Only the probed points are shown; the interpolated `mesh_matrix` can be much larger.
            let mesh_fields = &["profile_name", "profiles", "mesh_min", "mesh_max", "probed_matrix"][..];
            subscription.add(object, (object == "bed_mesh").then_some(mesh_fields));
        }
        subscription.add(
            "firmware_retraction",
            Some(&["retract_length", "retract_speed", "unretract_extra_length", "unretract_speed"]),
        );
        subscription.add("exclude_object", Some(&["objects", "excluded_objects", "current_object"]));
        for heater in &self.extra_heaters {
            subscription.add(heater, None);
        }
        for sensor in &self.temperature_sensors {
            subscription.add(sensor, Some(&["temperature"]));
        }
        for fan in &self.fans {
            subscription.add(fan, Some(&["speed", "rpm"]));
        }
        for led in &self.leds {
            subscription.add(led, Some(&["color_data"]));
        }
        for sensor in &self.filament_sensors {
            subscription.add(sensor, Some(&["enabled", "filament_detected"]));
        }
    }
}

pub(super) fn is_additional_extruder(name: &str) -> bool {
    name.strip_prefix("extruder")
        .is_some_and(|number| !number.is_empty() && number.bytes().all(|byte| byte.is_ascii_digit()))
}

/// Whether the answer of `printer.gcode.help` lists `command`.
pub fn lists_command(help: &Value, command: &str) -> bool {
    help.as_object().is_some_and(|commands| commands.keys().any(|key| key.eq_ignore_ascii_case(command)))
}

/// The tool numbers of the `T<n>` commands in the answer of `printer.gcode.help` (command → help text), sorted.
pub fn tool_commands(help: &Value) -> Vec<usize> {
    let mut tools: Vec<usize> = help
        .as_object()
        .into_iter()
        .flat_map(|commands| commands.keys())
        .filter_map(|command| command.strip_prefix(['T', 't']))
        .filter(|number| !number.is_empty() && number.bytes().all(|byte| byte.is_ascii_digit()))
        .filter_map(|number| number.parse().ok())
        .collect();
    tools.sort_unstable();
    tools
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_optional_hardware_and_macros() {
        let setup = Setup::from_objects([
            "webhooks",
            "extruder",
            "extruder1",
            "heater_bed",
            "fan",
            "quad_gantry_level",
            "bed_mesh",
            "bltouch",
            "heater_generic chamber",
            "temperature_sensor raspberry_pi",
            "temperature_host raspberry_pi",
            "filament_switch_sensor runout",
            "gcode_macro PRINT_START",
            "gcode_macro load_filament",
            "gcode_macro _HELPER",
        ]);

        assert_eq!(setup.leveling, Some(Leveling::QuadGantry));
        assert!(setup.probe);
        assert_eq!(setup.fans, ["fan"]);
        assert_eq!(setup.extra_heaters, ["extruder1", "heater_generic chamber"]);
        assert_eq!(setup.temperature_sensors, ["temperature_sensor raspberry_pi"]);
        assert_eq!(setup.filament_sensors, ["filament_switch_sensor runout"]);
        assert_eq!(setup.macros, ["LOAD_FILAMENT", "PRINT_START"]);
        assert!(setup.has_macro("load_filament"));
    }

    #[test]
    fn helper_sections_are_not_shown_but_still_count() {
        let setup = Setup::from_objects([
            "fan",
            "fan_generic _exhaust",
            "heater_fan _hotend",
            "controller_fan _mcu",
            "heater_generic _chamber",
            "temperature_fan _electronics",
            "temperature_sensor _mcu",
            "neopixel _status",
            "filament_switch_sensor _runout",
            "gcode_macro _HELPER",
            "probe_eddy_current _scanner",
        ]);

        assert_eq!(setup.fans, ["fan"]);
        assert!(setup.extra_heaters.is_empty() && setup.temperature_sensors.is_empty());
        assert!(setup.leds.is_empty() && setup.filament_sensors.is_empty() && setup.macros.is_empty());
        assert!(setup.probe);
    }

    #[test]
    fn quad_gantry_level_wins_over_z_tilt() {
        let leveling = |objects: &[&str]| Setup::from_objects(objects.iter().copied()).leveling;

        assert_eq!(leveling(&["z_tilt"]), Some(Leveling::ZTilt));
        assert_eq!(leveling(&["z_tilt", "quad_gantry_level"]), Some(Leveling::QuadGantry));
        assert_eq!(leveling(&["extruder"]), None);
    }

    #[test]
    fn tool_commands_are_the_t_commands_of_the_help() {
        let help =
            serde_json::json!({"T0": "G-Code macro", "T2": "Change tool", "TURN_OFF_HEATERS": "Off", "G28": "Home"});

        assert_eq!(tool_commands(&help), [0, 2]);
        assert_eq!(tool_commands(&Value::Null), Vec::<usize>::new());
        assert!(lists_command(&help, "T0"));
        assert!(!lists_command(&help, "UNSELECT_TOOL"));
    }
}
