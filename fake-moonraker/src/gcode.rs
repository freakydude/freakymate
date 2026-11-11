//! Parser for the G-code lines the UI sends via `printer.gcode.script`.

use crate::calibration::{CalibrationCommand, TestZ};
use crate::layer_pause::LayerPauseCommand;
use std::fmt;

mod afc;
mod calibration;
mod happy_hare;
mod heating;
mod macros;
mod motion;
mod toolchanger;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Axis {
    X,
    Y,
    Z,
}

impl Axis {
    pub fn index(self) -> usize {
        self as usize
    }

    pub fn letter(self) -> char {
        match self {
            Axis::X => 'x',
            Axis::Y => 'y',
            Axis::Z => 'z',
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct Move {
    pub x: Option<f64>,
    pub y: Option<f64>,
    pub z: Option<f64>,
    pub e: Option<f64>,
}

impl Move {
    pub fn linear_axes(&self) -> impl Iterator<Item = (Axis, f64)> {
        [(Axis::X, self.x), (Axis::Y, self.y), (Axis::Z, self.z)]
            .into_iter()
            .filter_map(|(axis, value)| value.map(|value| (axis, value)))
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum Command {
    Home(Vec<Axis>),
    /// `M104`: the extruder of tool `T<n>` (`extruder`, `extruder1`, …), or the active one without `T`.
    SetExtruderTarget {
        tool: Option<usize>,
        target: f64,
    },
    SetBedTarget(f64),
    SetFanSpeed(f64),
    AbsolutePositioning,
    RelativePositioning,
    Move(Move),
    DisableSteppers,
    SetGcodeOffset {
        z: Option<f64>,
        z_adjust: Option<f64>,
    },
    AbsoluteExtrusion,
    RelativeExtrusion,
    SetSpeedFactor(f64),
    SetExtrudeFactor(f64),
    SetPressureAdvance {
        advance: Option<f64>,
        smooth_time: Option<f64>,
    },
    SetVelocityLimit(VelocityLimit),
    SetRetraction(RetractionChange),
    /// Fans, LEDs and effects (`SET_FAN_SPEED`, `SET_LED`, `SET_LED_EFFECT`, `STOP_LED_EFFECTS`, `FAKE_FAN_STALL`).
    Device(DeviceCommand),
    SetHeaterTemperature {
        heater: String,
        target: f64,
    },
    /// Empty name: Klipper only lists the excluded objects.
    ExcludeObject {
        name: String,
    },
    SaveConfig,
    QuadGantryLevel,
    Help,
    /// `G4 P<milliseconds>`: waits; the fake answers the script only afterwards, like Klipper.
    Dwell(std::time::Duration),
    Mmu(MmuCommand),
    /// Tool changer and the macro variable commands.
    Toolchanger(ToolchangerCommand),
    /// AFC's lane commands; a printer without AFC ignores them, as Klipper would not know them.
    Afc(AfcCommand),
    /// Mainsail's layer pause macros.
    LayerPause(LayerPauseCommand),
    /// Probe, bed screws and bed mesh, and the commands within their helpers.
    Calibration(CalibrationCommand),
    /// Ends the running print with an error, like Klipper after a move out of range (`FAKE_PRINT_ERROR`; not in
    /// Klipper).
    FakePrintError,
    Ignored,
}

/// Which tool `SELECT_TOOL` means: `T=<number>` or `TOOL=<name>`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ToolSelector {
    Number(usize),
    Name(String),
}

/// `toolchanger.status` of klipper-toolchanger (viesturz `klipper/extras/toolchanger.py`, `STATUS_*`). The plugin also
/// knows `initializing` and `changing`; the fake never rests in them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToolchangerState {
    Uninitialized,
    Ready,
    Error,
}

impl ToolchangerState {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Uninitialized => "uninitialized",
            Self::Ready => "ready",
            Self::Error => "error",
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum ToolchangerCommand {
    SelectTool(ToolSelector),
    UnselectTool,
    /// `INITIALIZE_TOOLCHANGER [RECOVER=1]`; the fake treats both alike.
    Initialize {
        recover: bool,
    },
    /// `FAKE_TOOLCHANGER_STATE STATE=...`: test hook of the fake, not in the plugin.
    FakeState(ToolchangerState),
    ActivateExtruder {
        extruder: String,
    },
    /// `SET_GCODE_VARIABLE`: a macro variable; `None` is `null`.
    SetGcodeVariable {
        macro_name: String,
        variable: String,
        value: serde_json::Value,
    },
    /// `SAVE_VARIABLE`: Klipper's `save_variables`.
    SaveVariable {
        variable: String,
        value: serde_json::Value,
    },
}

/// The commands of AFC (AFC-Klipper-Add-On at 5ea84ea) with their parameters as written; AFC checks them, not the
/// parser. `T<n>` is a [`MmuCommand::ChangeTool`] as for Happy Hare.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AfcCommand {
    /// `CHANGE_TOOL LANE=` (`AFC.py` L2293).
    ChangeTool { lane: Option<String> },
    /// `TOOL_LOAD LANE=` (L1427).
    ToolLoad { lane: Option<String> },
    /// `TOOL_UNLOAD [LANE=]` (L1828).
    ToolUnload { lane: Option<String> },
    /// `LANE_UNLOAD LANE=` (L1321).
    LaneUnload { lane: Option<String> },
    /// `SET_SPOOL_ID LANE= SPOOL_ID=` (`AFC_spool.py` L325).
    SetSpoolId { lane: Option<String>, spool_id: Option<String> },
    /// `SET_COLOR LANE= COLOR=` (L197).
    SetColor { lane: Option<String>, color: Option<String> },
    /// `SET_MATERIAL LANE= MATERIAL=` (L261).
    SetMaterial { lane: Option<String>, material: Option<String> },
    /// `SET_MAP LANE= MAP=` (L148).
    SetMap { lane: Option<String>, map: Option<String> },
}

/// `SET_VELOCITY_LIMIT`; every value is optional, as in Klipper.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct VelocityLimit {
    pub velocity: Option<f64>,
    pub accel: Option<f64>,
    pub square_corner_velocity: Option<f64>,
    pub minimum_cruise_ratio: Option<f64>,
}

/// `SET_RETRACTION`; every value is optional, as in Klipper.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct RetractionChange {
    pub retract_length: Option<f64>,
    pub retract_speed: Option<f64>,
    pub unretract_extra_length: Option<f64>,
    pub unretract_speed: Option<f64>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum DeviceCommand {
    SetFanSpeed {
        fan: String,
        speed: f64,
    },
    /// Colours from 0 to 1 in the order red, green, blue, white; a missing one is 0, as in Klipper.
    SetLed {
        led: String,
        color: [f64; 4],
    },
    SetLedEffect {
        effect: String,
        stop: bool,
    },
    StopLedEffects,
    /// The tachometer of a fan reports 0 rpm, as if the fan were blocked (`FAKE_FAN_STALL FAN= [STALL=0|1]`; not in
    /// Klipper).
    FakeFanStall {
        fan: String,
        stall: bool,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MmuCommand {
    Home,
    Select {
        gate: usize,
    },
    ChangeTool {
        tool: usize,
    },
    /// Loads the selected gate; like Happy Hare's `MMU_LOAD` it has no gate parameter.
    Load,
    Unload,
    /// Pulls the filament of a gate out of the MMU; without `gate` the selected one.
    Eject {
        gate: Option<usize>,
        force: bool,
    },
    /// Turns off the motors; the selector loses its home.
    MotorsOff,
    /// `MMU_CHECK_GATE`: one gate, every gate (`ALL=1`) or, without either, the selected one.
    CheckGate {
        gate: Option<usize>,
        all: bool,
    },
    /// `MMU_PRELOAD`: parks filament pushed into a gate; without `gate` the selected one.
    Preload {
        gate: Option<usize>,
    },
    /// `MMU_SERVO POS=`; without a position Happy Hare only reports it.
    Servo {
        position: Option<ServoPosition>,
    },
    /// `MMU_SELECT BYPASS=1` or `MMU_SELECT_BYPASS`: filament fed by hand past the gates.
    SelectBypass,
    /// `MMU_TTG_MAP TOOL= GATE=`: the tool takes its filament from the gate.
    MapTool {
        tool: usize,
        gate: usize,
    },
    /// `MMU_TTG_MAP RESET=1`: every tool back to the gate of its number.
    ResetToolMap,
    /// `MMU_ENDLESS_SPOOL ENABLE= GROUPS=`; a missing parameter keeps the value.
    EndlessSpool {
        enabled: Option<bool>,
        groups: Option<Vec<u32>>,
    },
    /// `MMU_GATE_MAP GATE= SPOOLID= MATERIAL= COLOR= TEMP=`: the Spoolman spool of a gate (a negative id for none)
    /// and what Happy Hare keeps of its filament; a missing parameter keeps the value.
    GateMap {
        gate: usize,
        spool_id: Option<i64>,
        material: Option<String>,
        color: Option<String>,
        temperature: Option<i32>,
    },
    /// Not a Happy Hare command: lets a test or the snapshot tool raise the tension that makes tangle prevention
    /// boost the gear current (`FAKE_MMU_TANGLE BOOST=0|1`).
    FakeTangle {
        boosting: bool,
    },
    /// Not a Happy Hare command: holds the filament at a point of its way (`filament_pos`, -1 to 10) and moving in a
    /// direction (-1, 0, 1), for pictures of a load in progress (`FAKE_MMU_FILAMENT POS=3 DIRECTION=1`).
    FakeFilament {
        position: i32,
        direction: i32,
    },
}

/// Positions of `MMU_SERVO POS=` (`mmu_linear_servo_selector.py`); `off` only cuts the power of the servo.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ServoPosition {
    Up,
    Down,
    Move,
    Off,
}

impl ServoPosition {
    fn parse(text: &str) -> Result<Self, CommandError> {
        match text.to_ascii_lowercase().as_str() {
            "up" => Ok(Self::Up),
            "down" => Ok(Self::Down),
            "move" => Ok(Self::Move),
            "off" => Ok(Self::Off),
            _ => Err(CommandError::new(format!("Invalid POS {text}"))),
        }
    }
}

impl MmuCommand {
    pub fn gcode_name(&self) -> &'static str {
        match self {
            MmuCommand::Home => "MMU_HOME",
            MmuCommand::Select { .. } => "MMU_SELECT",
            MmuCommand::ChangeTool { .. } => "MMU_CHANGE_TOOL",
            MmuCommand::Load => "MMU_LOAD",
            MmuCommand::Unload => "MMU_UNLOAD",
            MmuCommand::Eject { .. } => "MMU_EJECT",
            MmuCommand::MotorsOff => "MMU_MOTORS_OFF",
            MmuCommand::CheckGate { .. } => "MMU_CHECK_GATE",
            MmuCommand::Preload { .. } => "MMU_PRELOAD",
            MmuCommand::Servo { .. } => "MMU_SERVO",
            MmuCommand::GateMap { .. } => "MMU_GATE_MAP",
            MmuCommand::SelectBypass => "MMU_SELECT_BYPASS",
            MmuCommand::MapTool { .. } | MmuCommand::ResetToolMap => "MMU_TTG_MAP",
            MmuCommand::EndlessSpool { .. } => "MMU_ENDLESS_SPOOL",
            MmuCommand::FakeTangle { .. } => "FAKE_MMU_TANGLE",
            MmuCommand::FakeFilament { .. } => "FAKE_MMU_FILAMENT",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommandError(String);

impl CommandError {
    pub fn new(message: impl Into<String>) -> Self {
        Self(message.into())
    }

    pub fn message(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for CommandError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl std::error::Error for CommandError {}

pub fn parse_script(script: &str) -> Result<Vec<Command>, CommandError> {
    script.lines().filter_map(significant_part).map(parse_line).collect()
}

fn significant_part(line: &str) -> Option<&str> {
    let code = line.split(';').next().unwrap_or_default().trim();
    (!code.is_empty()).then_some(code)
}

fn parse_line(line: &str) -> Result<Command, CommandError> {
    let mut words = line.split_whitespace();
    let name = words.next().unwrap_or_default().to_ascii_uppercase();
    let args: Vec<&str> = words.collect();

    let groups = [
        motion::parse,
        heating::parse,
        macros::parse,
        calibration::parse,
        happy_hare::parse,
        toolchanger::parse,
        afc::parse,
    ];
    for parse in groups {
        if let Some(command) = parse(&name, &args)? {
            return Ok(command);
        }
    }
    Ok(Command::Ignored)
}

fn letter_value(args: &[&str], letter: char) -> Result<Option<f64>, CommandError> {
    args.iter()
        .find(|arg| arg.chars().next().map(|first| first.to_ascii_uppercase()) == Some(letter))
        // The first character is an ASCII letter and therefore exactly one byte long.
        .map(|arg| parse_number(&arg[1..]))
        .transpose()
}

fn named_value(args: &[&str], name: &str) -> Result<Option<f64>, CommandError> {
    args.iter()
        .filter_map(|arg| arg.split_once('='))
        .find(|(key, _)| key.eq_ignore_ascii_case(name))
        .map(|(_, value)| parse_number(value))
        .transpose()
}

fn required_text(args: &[&str], name: &str) -> Result<String, CommandError> {
    named_text(args, name).ok_or_else(|| CommandError::new(format!("Missing parameter {name}")))
}

fn named_text(args: &[&str], name: &str) -> Option<String> {
    args.iter()
        .filter_map(|arg| arg.split_once('='))
        .find(|(key, _)| key.eq_ignore_ascii_case(name))
        .map(|(_, value)| value.to_owned())
}

fn required_index(args: &[&str], name: &str) -> Result<usize, CommandError> {
    named_index(args, name)?.ok_or_else(|| CommandError::new(format!("Missing parameter {name}")))
}

fn named_index(args: &[&str], name: &str) -> Result<Option<usize>, CommandError> {
    named_value(args, name)?
        .map(|value| {
            if value >= 0.0 && value.fract() == 0.0 {
                Ok(value as usize)
            } else {
                Err(CommandError::new(format!("Parameter {name} must be a non-negative integer")))
            }
        })
        .transpose()
}

fn parse_number(text: &str) -> Result<f64, CommandError> {
    text.parse::<f64>()
        .ok()
        .filter(|number| number.is_finite())
        .ok_or_else(|| CommandError::new(format!("Unable to parse number '{text}'")))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_relative_extrusion_script() {
        let commands = parse_script("G91\nG1 E10 F180").unwrap();

        assert_eq!(
            commands,
            vec![Command::RelativePositioning, Command::Move(Move { e: Some(10.0), ..Move::default() }),]
        );
    }

    #[test]
    fn parses_mainsails_layer_pause_macros() {
        let commands =
            parse_script("SET_PAUSE_NEXT_LAYER\nSET_PAUSE_NEXT_LAYER ENABLE=0\nSET_PAUSE_AT_LAYER ENABLE=1 LAYER=42")
                .unwrap();

        assert_eq!(
            commands,
            vec![
                Command::LayerPause(LayerPauseCommand::NextLayer { enable: true }),
                Command::LayerPause(LayerPauseCommand::NextLayer { enable: false }),
                Command::LayerPause(LayerPauseCommand::AtLayer { enable: Some(true), layer: Some(42) }),
            ]
        );
    }

    #[test]
    fn parses_calibration_commands() {
        let commands =
            parse_script("TESTZ Z=-0.05\nTESTZ Z=++\nBED_MESH_PROFILE LOAD=hot\nBED_MESH_PROFILE SAVE=hot").unwrap();

        assert_eq!(
            commands,
            vec![
                Command::Calibration(CalibrationCommand::TestZ(TestZ::By(-0.05))),
                Command::Calibration(CalibrationCommand::TestZ(TestZ::Up { half: false })),
                Command::Calibration(CalibrationCommand::BedMeshLoad { profile: "hot".into() }),
                Command::Calibration(CalibrationCommand::BedMeshSave { profile: "hot".into() }),
            ]
        );
        assert!(parse_script("TESTZ Z=up").is_err());
    }

    #[test]
    fn parses_tuning_commands() {
        let commands =
            parse_script("M220 S110\nM221 S95\nSET_PRESSURE_ADVANCE ADVANCE=0.05\nM83\nSET_HEATER_TEMPERATURE HEATER=chamber TARGET=40")
                .unwrap();

        assert_eq!(
            commands,
            vec![
                Command::SetSpeedFactor(1.1),
                Command::SetExtrudeFactor(0.95),
                Command::SetPressureAdvance { advance: Some(0.05), smooth_time: None },
                Command::RelativeExtrusion,
                Command::SetHeaterTemperature { heater: "chamber".to_owned(), target: 40.0 },
            ]
        );
    }

    #[test]
    fn parses_homing_with_and_without_axes() {
        assert_eq!(parse_script("G28 X Y").unwrap(), vec![Command::Home(vec![Axis::X, Axis::Y])]);
        assert_eq!(parse_script("G28").unwrap(), vec![Command::Home(vec![])]);
    }

    #[test]
    fn ignores_comments_and_blank_lines() {
        let commands = parse_script("; nur Kommentar\n\nM104 S200 ; Duese\n").unwrap();

        assert_eq!(commands, vec![Command::SetExtruderTarget { tool: None, target: 200.0 }]);
    }

    #[test]
    fn scales_fan_speed_to_fraction() {
        assert_eq!(parse_script("M106 S255").unwrap(), vec![Command::SetFanSpeed(1.0)]);
        assert_eq!(parse_script("M107").unwrap(), vec![Command::SetFanSpeed(0.0)]);
    }

    #[test]
    fn parses_named_gcode_offset_arguments() {
        let commands = parse_script("SET_GCODE_OFFSET Z_ADJUST=-0.05 MOVE=1").unwrap();

        assert_eq!(commands, vec![Command::SetGcodeOffset { z: None, z_adjust: Some(-0.05) }]);
    }

    #[test]
    fn parses_happy_hare_commands() {
        let commands =
            parse_script("MMU_HOME\nMMU_GATE_MAP GATE=1 SPOOLID=8\nMMU_GATE_MAP GATE=2 SPOOLID=-1\nMMU_SELECT GATE=2\nMMU_CHANGE_TOOL TOOL=1\nT3\nMMU_LOAD\nMMU_UNLOAD")
                .unwrap();

        assert_eq!(
            commands,
            vec![
                Command::Mmu(MmuCommand::Home),
                Command::Mmu(MmuCommand::GateMap {
                    gate: 1,
                    spool_id: Some(8),
                    material: None,
                    color: None,
                    temperature: None
                }),
                Command::Mmu(MmuCommand::GateMap {
                    gate: 2,
                    spool_id: Some(-1),
                    material: None,
                    color: None,
                    temperature: None
                }),
                Command::Mmu(MmuCommand::Select { gate: 2 }),
                Command::Mmu(MmuCommand::ChangeTool { tool: 1 }),
                Command::Mmu(MmuCommand::ChangeTool { tool: 3 }),
                Command::Mmu(MmuCommand::Load),
                Command::Mmu(MmuCommand::Unload),
            ]
        );
    }

    #[test]
    fn parses_bypass_tool_map_and_endless_spool() {
        let commands = parse_script(
            "MMU_SELECT BYPASS=1\nMMU_SELECT_BYPASS\nMMU_TTG_MAP TOOL=3 GATE=1\nMMU_TTG_MAP RESET=1\nMMU_TTG_MAP\nMMU_ENDLESS_SPOOL ENABLE=1 GROUPS=0,0,1",
        )
        .unwrap();

        assert_eq!(
            commands,
            vec![
                Command::Mmu(MmuCommand::SelectBypass),
                Command::Mmu(MmuCommand::SelectBypass),
                Command::Mmu(MmuCommand::MapTool { tool: 3, gate: 1 }),
                Command::Mmu(MmuCommand::ResetToolMap),
                Command::Ignored,
                Command::Mmu(MmuCommand::EndlessSpool { enabled: Some(true), groups: Some(vec![0, 0, 1]) }),
            ]
        );
        assert!(parse_script("MMU_ENDLESS_SPOOL GROUPS=0,-1").is_err());
    }

    #[test]
    fn parses_the_service_commands_of_happy_hare() {
        let commands = parse_script(
            "MMU_MOTORS_OFF\nMMU_CHECK_GATE\nMMU_CHECK_GATE GATE=3\nMMU_CHECK_GATE ALL=1\nMMU_PRELOAD GATE=2\nMMU_SERVO POS=down\nMMU_SERVO",
        )
        .unwrap();

        assert_eq!(
            commands,
            vec![
                Command::Mmu(MmuCommand::MotorsOff),
                Command::Mmu(MmuCommand::CheckGate { gate: None, all: false }),
                Command::Mmu(MmuCommand::CheckGate { gate: Some(3), all: false }),
                Command::Mmu(MmuCommand::CheckGate { gate: None, all: true }),
                Command::Mmu(MmuCommand::Preload { gate: Some(2) }),
                Command::Mmu(MmuCommand::Servo { position: Some(ServoPosition::Down) }),
                Command::Mmu(MmuCommand::Servo { position: None }),
            ]
        );
        assert_eq!(parse_script("MMU_SERVO POS=sideways").unwrap_err().message(), "Invalid POS sideways");
    }

    #[test]
    fn the_gate_map_takes_material_colour_and_temperature_like_happy_hare() {
        assert_eq!(
            parse_script("MMU_GATE_MAP GATE=1 MATERIAL=petg COLOR=FF8000 TEMP=235").unwrap(),
            vec![Command::Mmu(MmuCommand::GateMap {
                gate: 1,
                spool_id: None,
                material: Some("PETG".into()),
                color: Some("ff8000".into()),
                temperature: Some(235),
            })]
        );
    }

    #[test]
    fn happy_hare_commands_validate_their_parameters() {
        assert_eq!(
            parse_script("MMU_EJECT\nMMU_EJECT GATE=2 FORCE=1").unwrap(),
            vec![
                Command::Mmu(MmuCommand::Eject { gate: None, force: false }),
                Command::Mmu(MmuCommand::Eject { gate: Some(2), force: true }),
            ]
        );
        assert_eq!(parse_script("MMU_SELECT").unwrap_err().message(), "Missing parameter GATE");
        assert!(parse_script("MMU_SELECT GATE=1.5").is_err());
        assert!(parse_script("MMU_SELECT GATE=-1").is_err());
    }

    #[test]
    fn only_digits_after_t_make_a_tool_change() {
        assert_eq!(parse_script("TURN_OFF_HEATERS").unwrap(), vec![Command::Ignored]);
    }

    #[test]
    fn parses_help() {
        assert_eq!(parse_script("help").unwrap(), vec![Command::Help]);
    }

    #[test]
    fn dwell_waits_the_given_milliseconds() {
        assert_eq!(parse_script("G4 P1500").unwrap(), vec![Command::Dwell(std::time::Duration::from_millis(1500))]);
    }

    #[test]
    fn unknown_commands_are_ignored() {
        assert_eq!(parse_script("M117 Hallo").unwrap(), vec![Command::Ignored]);
    }

    #[test]
    fn rejects_unparsable_numbers() {
        let error = parse_script("G1 Xabc").unwrap_err();

        assert_eq!(error.message(), "Unable to parse number 'abc'");
    }

    #[test]
    fn parses_device_and_limit_commands() {
        let commands = parse_script(
            "SET_FAN_SPEED FAN=nevermore SPEED=0.4\nSET_LED LED=chamber WHITE=1 SYNC=0\nSET_LED_EFFECT EFFECT=rainbow STOP=1\n\
             SET_VELOCITY_LIMIT ACCEL=4000\nSET_RETRACTION RETRACT_LENGTH=0.6\nSET_PRESSURE_ADVANCE SMOOTH_TIME=0.03",
        )
        .unwrap();

        assert_eq!(
            commands,
            vec![
                Command::Device(DeviceCommand::SetFanSpeed { fan: "nevermore".into(), speed: 0.4 }),
                Command::Device(DeviceCommand::SetLed { led: "chamber".into(), color: [0.0, 0.0, 0.0, 1.0] }),
                Command::Device(DeviceCommand::SetLedEffect { effect: "rainbow".into(), stop: true }),
                Command::SetVelocityLimit(VelocityLimit { accel: Some(4000.0), ..VelocityLimit::default() }),
                Command::SetRetraction(RetractionChange { retract_length: Some(0.6), ..RetractionChange::default() }),
                Command::SetPressureAdvance { advance: None, smooth_time: Some(0.03) },
            ]
        );
        assert!(parse_script("SET_FAN_SPEED FAN=nevermore").is_err());
    }
}
