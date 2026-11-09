//! What Klipper is told: its methods behind Moonraker's `printer.*` and its own G-code. G-code of the plugins and
//! macros reaches Klipper through [`run_gcode`] as well.

use super::{instance, Axis, FanKind, Motion, TestZ, BED_HEATER, NOZZLE_HEATER};
use crate::moonraker::Call;
use serde_json::json;

/// G-code feed rates are in mm/min, the speeds here in mm/s.
const SECONDS_PER_MINUTE: f64 = 60.0;
/// Fan speed in G-code (`M106 S`) runs from 0 to 255.
const FAN_PWM_MAX: f64 = 255.0;
/// Fixed speeds for manual moves in mm/s.
const JOG_SPEED_XY: f64 = 100.0;
const JOG_SPEED_Z: f64 = 10.0;

pub const DISABLE_MOTORS: &str = "M84";
pub const QUAD_GANTRY_LEVEL: &str = "QUAD_GANTRY_LEVEL";
pub const Z_TILT_ADJUST: &str = "Z_TILT_ADJUST";
pub const RESET_Z_OFFSET: &str = "SET_GCODE_OFFSET Z=0 MOVE=1";
pub const APPLY_Z_OFFSET_TO_PROBE: &str = "Z_OFFSET_APPLY_PROBE";
pub const APPLY_Z_OFFSET_TO_ENDSTOP: &str = "Z_OFFSET_APPLY_ENDSTOP";
pub const SAVE_CONFIG: &str = "SAVE_CONFIG";
pub const BED_MESH_CALIBRATE: &str = "BED_MESH_CALIBRATE";
pub const BED_MESH_CLEAR: &str = "BED_MESH_CLEAR";
pub const SCREWS_TILT_CALCULATE: &str = "SCREWS_TILT_CALCULATE";
pub const BED_SCREWS_ADJUST: &str = "BED_SCREWS_ADJUST";
pub const PROBE_CALIBRATE: &str = "PROBE_CALIBRATE";
pub const Z_ENDSTOP_CALIBRATE: &str = "Z_ENDSTOP_CALIBRATE";
/// Klipper registers these only while `manual_probe` or `bed_screws` waits.
pub const ACCEPT: &str = "ACCEPT";
pub const ADJUSTED: &str = "ADJUSTED";
pub const ABORT: &str = "ABORT";

/// Runs a G-code script, Klipper's own or that of a plugin or macro.
pub fn run_gcode(script: impl Into<String>) -> Call {
    Call::with("printer.gcode.script", json!({ "script": script.into() }))
}

pub fn start_print(filename: &str) -> Call {
    Call::with("printer.print.start", json!({ "filename": filename }))
}

pub fn pause_print() -> Call {
    Call::new("printer.print.pause")
}

pub fn resume_print() -> Call {
    Call::new("printer.print.resume")
}

pub fn cancel_print() -> Call {
    Call::new("printer.print.cancel")
}

pub fn emergency_stop() -> Call {
    Call::new("printer.emergency_stop")
}

pub fn restart() -> Call {
    Call::new("printer.restart")
}

pub fn firmware_restart() -> Call {
    Call::new("printer.firmware_restart")
}

/// `G28` for all axes or none selected, else only the selected ones.
pub fn home(x: bool, y: bool, z: bool) -> String {
    if x && y && z || !(x || y || z) {
        return "G28".to_owned();
    }
    let axes: Vec<&str> = [(x, "X"), (y, "Y"), (z, "Z")]
        .into_iter()
        .filter_map(|(selected, letter)| selected.then_some(letter))
        .collect();
    format!("G28 {}", axes.join(" "))
}

/// A relative move as an absolute one, clamped to the axis range; `None` if the axis would not move.
pub fn jog(motion: &Motion, axis: Axis, distance: f64) -> Option<String> {
    let index = axis.index();
    let current = motion.position[index];
    let (minimum, maximum) = (motion.axis_minimum[index], motion.axis_maximum[index]);
    let target = if maximum > minimum { (current + distance).clamp(minimum, maximum) } else { current + distance };
    if (target - current).abs() < 1e-6 {
        return None;
    }
    let speed = if axis == Axis::Z { JOG_SPEED_Z } else { JOG_SPEED_XY };
    Some(format!("G90\nG1 {}{} F{}", axis.letter(), number(target), number(speed * SECONDS_PER_MINUTE)))
}

/// G-code for a target temperature, matching the kind of heater; below zero is off.
pub fn heater_target(heater: &str, target: f64) -> String {
    let target = number(target);
    match heater.split_once(' ') {
        None if heater == NOZZLE_HEATER => format!("M104 S{target}"),
        None if heater == BED_HEATER => format!("M140 S{target}"),
        Some(("temperature_fan", name)) => format!("SET_TEMPERATURE_FAN_TARGET TEMPERATURE_FAN={name} TARGET={target}"),
        Some((_, name)) => format!("SET_HEATER_TEMPERATURE HEATER={name} TARGET={target}"),
        None => format!("SET_HEATER_TEMPERATURE HEATER={heater} TARGET={target}"),
    }
}

/// `M106` for the part fan, `SET_FAN_SPEED` for a generic fan; the others cannot be set. `speed` from 0 to 1.
pub fn fan_speed(fan: &str, speed: f64) -> Option<String> {
    let speed = speed.clamp(0.0, 1.0);
    match FanKind::of(fan)? {
        FanKind::Part => Some(format!("M106 S{}", (speed * FAN_PWM_MAX).round())),
        FanKind::Generic => Some(format!("SET_FAN_SPEED FAN={} SPEED={:.2}", instance(fan), speed)),
        FanKind::Heater | FanKind::Controller => None,
    }
}

/// `SET_LED` with every channel from 0 to 1, in the order red, green, blue, white.
pub fn set_led(led: &str, [red, green, blue, white]: [f64; 4]) -> String {
    let channel = |value: f64| format!("{:.2}", value.clamp(0.0, 1.0));
    format!(
        "SET_LED LED={} RED={} GREEN={} BLUE={} WHITE={} SYNC=0",
        instance(led),
        channel(red),
        channel(green),
        channel(blue),
        channel(white)
    )
}

/// Babystepping: moves the nozzle at once by `delta`.
pub fn adjust_z_offset(delta: f64) -> String {
    format!("SET_GCODE_OFFSET Z_ADJUST={} MOVE=1", number(delta))
}

pub fn speed_factor(percent: f64) -> String {
    format!("M220 S{}", percent.max(1.0).round())
}

/// Relative extrusion; `length` below zero retracts, the sign of `speed` does not count.
pub fn extrude(length: f64, speed: f64) -> String {
    format!("M83\nG1 E{} F{}", number(length), number(speed.abs() * SECONDS_PER_MINUTE))
}

/// `EXCLUDE_OBJECT` reads the name up to the next space, so names with spaces cannot be excluded.
pub fn exclude_object(name: &str) -> Option<String> {
    (!name.is_empty() && !name.contains(char::is_whitespace)).then(|| format!("EXCLUDE_OBJECT NAME={name}"))
}

/// Why a name cannot be given to `BED_MESH_PROFILE SAVE=`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MeshNameProblem {
    Empty,
    /// Klipper splits parameters at whitespace.
    Whitespace,
    /// `bed_mesh.py` refuses to save to `default`, the profile `BED_MESH_CALIBRATE` writes.
    Reserved,
}

pub const DEFAULT_MESH_PROFILE: &str = "default";

fn usable_profile(profile: &str) -> bool {
    !profile.is_empty() && !profile.contains(char::is_whitespace)
}

pub fn mesh_name_problem(profile: &str) -> Option<MeshNameProblem> {
    if profile.is_empty() {
        Some(MeshNameProblem::Empty)
    } else if profile.contains(char::is_whitespace) {
        Some(MeshNameProblem::Whitespace)
    } else if profile == DEFAULT_MESH_PROFILE {
        Some(MeshNameProblem::Reserved)
    } else {
        None
    }
}

/// Klipper splits parameters at whitespace, so a profile name with a space cannot be loaded this way.
pub fn load_bed_mesh(profile: &str) -> Option<String> {
    usable_profile(profile).then(|| format!("BED_MESH_PROFILE LOAD={profile}"))
}

/// Saves the active mesh as `profile` for this session; `SAVE_CONFIG` keeps it.
pub fn save_bed_mesh(profile: &str) -> Option<String> {
    mesh_name_problem(profile).is_none().then(|| format!("BED_MESH_PROFILE SAVE={profile}"))
}

/// Removes `profile` for this session; `SAVE_CONFIG` removes it from the configuration.
pub fn remove_bed_mesh(profile: &str) -> Option<String> {
    usable_profile(profile).then(|| format!("BED_MESH_PROFILE REMOVE={profile}"))
}

pub fn test_z(step: TestZ) -> String {
    match step {
        TestZ::By(distance) => format!("TESTZ Z={}", number(distance)),
        TestZ::BisectUp => "TESTZ Z=+".to_owned(),
        TestZ::BisectDown => "TESTZ Z=-".to_owned(),
    }
}

/// Number without redundant decimal places, the way Klipper likes it.
fn number(value: f64) -> String {
    let rounded = (value * 10_000.0).round() / 10_000.0;
    let text = format!("{rounded:.4}");
    text.trim_end_matches('0').trim_end_matches('.').to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn motion_at(position: [f64; 4]) -> Motion {
        Motion {
            position,
            axis_minimum: [0.0, 0.0, -2.0, 0.0],
            axis_maximum: [300.0, 300.0, 250.0, 0.0],
            ..Motion::default()
        }
    }

    #[test]
    fn homing_selects_axes() {
        assert_eq!(home(true, true, false), "G28 X Y");
        assert_eq!(home(true, true, true), "G28");
        assert_eq!(home(false, false, true), "G28 Z");
        assert_eq!(home(false, false, false), "G28");
    }

    #[test]
    fn exclude_object_names_the_object() {
        assert_eq!(exclude_object("Benchy_0").as_deref(), Some("EXCLUDE_OBJECT NAME=Benchy_0"));
        assert_eq!(exclude_object("Part 2"), None);
        assert_eq!(exclude_object(""), None);
    }

    #[test]
    fn jog_is_clamped_to_axis_limits() {
        let motion = motion_at([10.0, 10.0, 5.0, 0.0]);
        assert_eq!(jog(&motion, Axis::X, -50.0).as_deref(), Some("G90\nG1 X0 F6000"));
        assert_eq!(jog(&motion, Axis::Z, 0.1).as_deref(), Some("G90\nG1 Z5.1 F600"));
        assert_eq!(jog(&motion_at([0.0; 4]), Axis::X, -1.0), None);
    }

    #[test]
    fn heater_targets_use_matching_gcode() {
        assert_eq!(heater_target("extruder", 210.0), "M104 S210");
        assert_eq!(heater_target("heater_bed", 0.0), "M140 S0");
        assert_eq!(heater_target("heater_generic chamber", 45.5), "SET_HEATER_TEMPERATURE HEATER=chamber TARGET=45.5");
        assert_eq!(
            heater_target("temperature_fan nevermore", 40.0),
            "SET_TEMPERATURE_FAN_TARGET TEMPERATURE_FAN=nevermore TARGET=40"
        );
        assert_eq!(heater_target("extruder1", 200.0), "SET_HEATER_TEMPERATURE HEATER=extruder1 TARGET=200");
    }

    #[test]
    fn only_part_and_generic_fans_can_be_set() {
        assert_eq!(fan_speed("fan", 0.5).as_deref(), Some("M106 S128"));
        assert_eq!(fan_speed("fan_generic nevermore", 0.4).as_deref(), Some("SET_FAN_SPEED FAN=nevermore SPEED=0.40"));
        assert_eq!(fan_speed("fan", 1.5).as_deref(), Some("M106 S255"));
        assert_eq!(fan_speed("heater_fan hotend", 1.0), None);
    }

    #[test]
    fn leds_get_every_channel_clamped() {
        assert_eq!(
            set_led("neopixel sb", [1.2, 0.5, 0.0, -1.0]),
            "SET_LED LED=sb RED=1.00 GREEN=0.50 BLUE=0.00 WHITE=0.00 SYNC=0"
        );
    }

    #[test]
    fn tuning_and_extrusion_gcode() {
        assert_eq!(adjust_z_offset(-0.005), "SET_GCODE_OFFSET Z_ADJUST=-0.005 MOVE=1");
        assert_eq!(test_z(TestZ::By(-0.05)), "TESTZ Z=-0.05");
        assert_eq!(test_z(TestZ::BisectUp), "TESTZ Z=+");
        assert_eq!(load_bed_mesh("default").as_deref(), Some("BED_MESH_PROFILE LOAD=default"));
        assert_eq!(load_bed_mesh("my mesh"), None);
        assert_eq!(save_bed_mesh("pei").as_deref(), Some("BED_MESH_PROFILE SAVE=pei"));
        assert_eq!(save_bed_mesh("default"), None);
        assert_eq!(mesh_name_problem("my mesh"), Some(MeshNameProblem::Whitespace));
        assert_eq!(remove_bed_mesh("default").as_deref(), Some("BED_MESH_PROFILE REMOVE=default"));
        assert_eq!(speed_factor(110.0), "M220 S110");
        assert_eq!(speed_factor(0.0), "M220 S1");
        assert_eq!(extrude(-10.0, 5.0), "M83\nG1 E-10 F300");
    }

    #[test]
    fn methods_carry_their_parameters() {
        assert_eq!(start_print("a/b.gcode"), Call::with("printer.print.start", json!({ "filename": "a/b.gcode" })));
        assert_eq!(run_gcode("G28"), Call::with("printer.gcode.script", json!({ "script": "G28" })));
        assert_eq!(emergency_stop().method, "printer.emergency_stop");
    }
}
