//! Simulated printer: heaters, toolhead and print job.

use crate::afc::Afc;
use crate::calibration::Calibration;
use crate::devices::{DeviceInputs, Devices};
use crate::files::{GcodeFile, LAYER_HEIGHT};
use crate::gcode::{
    AfcCommand, Axis, Command, CommandError, DeviceCommand, MmuCommand, Move, RetractionChange, VelocityLimit,
};
use crate::happy_hare::HappyHare;
use crate::history::{Sample, TemperatureStore};
use crate::layer_pause::LayerPause;
use crate::machine::{MachineProfile, FILAMENT_SENSORS};
use crate::print_objects::{self, Definitions};
use serde_json::{json, Map, Value};
use std::time::Duration;

mod afc;
mod config;
mod status;
#[cfg(test)]
mod tests;
mod toolchanger;
#[cfg(test)]
mod units_tests;
pub(crate) use toolchanger::mux_error;

const AMBIENT_TEMPERATURE: f64 = 24.0;
const HEAT_RATE_PER_SECOND: f64 = 3.0;
const COOL_RATE_PER_SECOND: f64 = 0.5;
const MIN_EXTRUDE_TEMPERATURE: f64 = 170.0;
const MAX_EXTRUDER_TARGET: f64 = 300.0;
const MAX_BED_TARGET: f64 = 120.0;
const MIN_HEATER_TEMPERATURE: f64 = 10.0;
const MAX_CHAMBER_TARGET: f64 = 70.0;
const HOST_TEMPERATURE: f64 = 45.0;
/// The host temperature wanders this much around `HOST_TEMPERATURE`, like a real CPU under changing load. Klipper
/// only reports changed values; this way status updates keep coming as on a real printer, whose readings never stand
/// still.
const HOST_TEMPERATURE_SWING: f64 = 0.4;
/// Macros every simulated printer knows; `_` macros are helper macros and invisible to the UI.
const MACROS: [&str; 10] = [
    "PRINT_START",
    "PRINT_END",
    "LOAD_FILAMENT",
    "UNLOAD_FILAMENT",
    "PARK",
    "_PURGE_HELPER",
    "PAUSE",
    LAYER_PAUSE_NEXT_MACRO,
    LAYER_PAUSE_AT_MACRO,
    PRINT_STATS_INFO_MACRO,
];
/// Mainsail's layer pause macros (mainsail-config), see [`LayerPause`].
const LAYER_PAUSE_NEXT_MACRO: &str = "SET_PAUSE_NEXT_LAYER";
const LAYER_PAUSE_AT_MACRO: &str = "SET_PAUSE_AT_LAYER";
const PRINT_STATS_INFO_MACRO: &str = "SET_PRINT_STATS_INFO";
/// Templates of the macros above, so parameters and defaults can be read like on a real printer.
const MACRO_TEMPLATES: [(&str, &str); 10] = [
    ("PRINT_START", "{% set BED = params.BED|default(60) %}\nM140 S{BED}\nM109 S{params.EXTRUDER|default(210)}"),
    ("PRINT_END", "M104 S0\nM140 S0"),
    ("LOAD_FILAMENT", "{% set TEMP = params.TEMP|default(230)|float %}\nM104 S{TEMP}"),
    ("UNLOAD_FILAMENT", "{% if 'SKIP_HEAT' not in params %}M104 S{params.TEMP|default(230)}{% endif %}"),
    ("PARK", "{% set Z = params.Z|default(10) %}\nG91\nG1 Z{Z}\nG90"),
    ("_PURGE_HELPER", "G1 E{params.AMOUNT|default(5)}"),
    ("PAUSE", "PAUSE_BASE"),
    (LAYER_PAUSE_NEXT_MACRO, "{% set ENABLE = params.ENABLE|default(1)|int != 0 %}\n{% set MACRO = params.MACRO|default('PAUSE') %}"),
    (
        LAYER_PAUSE_AT_MACRO,
        "{% set ENABLE = params.ENABLE|int != 0 if params.ENABLE is defined else params.LAYER is defined %}\n{% set LAYER = params.LAYER|default(0)|int %}\n{% set MACRO = params.MACRO|default('PAUSE') %}",
    ),
    (PRINT_STATS_INFO_MACRO, "SET_PRINT_STATS_INFO_BASE {rawparams}"),
];
/// Klipper's message after a move outside the axis limits, for `FAKE_PRINT_ERROR`.
const PRINT_ERROR_MESSAGE: &str = "Move out of range: 310.000 150.000 12.400 [482.120]";
const AXIS_MINIMUM: [f64; 4] = [0.0; 4];
const AXIS_MAXIMUM: [f64; 4] = [300.0, 300.0, 300.0, 0.0];
const ALL_AXES: [Axis; 3] = [Axis::X, Axis::Y, Axis::Z];
const EXTRUDER_INDEX: usize = 3;
const SHUTDOWN_MESSAGE: &str = "Shutdown due to M112 command";
/// Klipper's default `[idle_timeout] timeout`; afterwards heaters and motors are switched off.
const IDLE_TIMEOUT_SECONDS: f64 = 600.0;
/// How long `idle_timeout` reports "Printing" (busy) after a command.
const BUSY_SECONDS: f64 = 2.0;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum KlippyState {
    Ready,
    Shutdown,
}

impl KlippyState {
    fn as_str(self) -> &'static str {
        match self {
            KlippyState::Ready => "ready",
            KlippyState::Shutdown => "shutdown",
        }
    }

    fn message(self) -> &'static str {
        match self {
            KlippyState::Ready => "Printer is ready",
            KlippyState::Shutdown => SHUTDOWN_MESSAGE,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PrintState {
    Standby,
    Printing,
    Paused,
    Complete,
    Cancelled,
    Error,
}

impl PrintState {
    pub fn as_str(self) -> &'static str {
        match self {
            PrintState::Standby => "standby",
            PrintState::Printing => "printing",
            PrintState::Paused => "paused",
            PrintState::Complete => "complete",
            PrintState::Cancelled => "cancelled",
            PrintState::Error => "error",
        }
    }

    pub fn is_active(self) -> bool {
        matches!(self, PrintState::Printing | PrintState::Paused)
    }
}

#[derive(Debug, Clone, Copy)]
struct Heater {
    temperature: f64,
    target: f64,
}

impl Heater {
    fn cold() -> Self {
        Self { temperature: AMBIENT_TEMPERATURE, target: 0.0 }
    }

    fn advance(&mut self, seconds: f64) {
        let goal = self.target.max(AMBIENT_TEMPERATURE);
        let rate = if self.temperature < goal { HEAT_RATE_PER_SECOND } else { COOL_RATE_PER_SECOND };
        self.temperature = step_towards(self.temperature, goal, rate * seconds);
    }

    fn set_target(&mut self, target: f64, maximum: f64) -> Result<(), CommandError> {
        // Like Klipper: 0 switches off, everything else must lie within min_temp..=max_temp.
        if target != 0.0 && !(MIN_HEATER_TEMPERATURE..=maximum).contains(&target) {
            return Err(CommandError::new(format!(
                "Requested temperature ({target:.1}) out of range ({MIN_HEATER_TEMPERATURE:.1}:{maximum:.1})"
            )));
        }
        self.target = target;
        Ok(())
    }

    fn to_json(self) -> Value {
        json!({ "temperature": round(self.temperature, 2), "target": self.target })
    }
}

#[derive(Debug, Clone)]
struct PrintJob {
    estimated_time: f64,
    layer_count: u32,
    filament_total: f64,
    /// Klipper keeps the objects of the last file until the next one starts.
    object_definitions: Definitions,
}

/// `[printer]` limits, changed by `SET_VELOCITY_LIMIT` until the next restart.
#[derive(Debug, Clone, Copy)]
struct Limits {
    max_velocity: f64,
    max_accel: f64,
    square_corner_velocity: f64,
    minimum_cruise_ratio: f64,
}

const CONFIGURED_LIMITS: Limits =
    Limits { max_velocity: 300.0, max_accel: 3000.0, square_corner_velocity: 5.0, minimum_cruise_ratio: 0.5 };
const CONFIGURED_PRESSURE_ADVANCE: f64 = 0.04;
const CONFIGURED_SMOOTH_TIME: f64 = 0.04;

/// `[firmware_retraction]`, changed by `SET_RETRACTION`.
#[derive(Debug, Clone, Copy)]
struct Retraction {
    retract_length: f64,
    retract_speed: f64,
    unretract_extra_length: f64,
    unretract_speed: f64,
}

const CONFIGURED_RETRACTION: Retraction =
    Retraction { retract_length: 0.5, retract_speed: 35.0, unretract_extra_length: 0.0, unretract_speed: 30.0 };

#[derive(Debug, Clone)]
pub struct Printer {
    time_scale: f64,
    /// Simulated seconds since the start.
    uptime: f64,
    profile: MachineProfile,
    mmu: Option<HappyHare>,
    /// `None` without AFC.
    afc: Option<Afc>,
    /// `None` without a tool changer; the heaters of its extra extruders live in there.
    toolchanger: Option<toolchanger::Toolchanger>,
    klippy: KlippyState,
    extruder: Heater,
    bed: Heater,
    chamber: Option<Heater>,
    position: [f64; 4],
    homed: [bool; 3],
    relative_positioning: bool,
    relative_extrusion: bool,
    fan_speed: f64,
    speed_factor: f64,
    extrude_factor: f64,
    pressure_advance: f64,
    smooth_time: f64,
    limits: Limits,
    /// `None` without the module (profile `basic`).
    retraction: Option<Retraction>,
    /// Fans, LEDs and sensors beyond the basics; `None` in the profile `basic`.
    devices: Option<Devices>,
    temperatures: TemperatureStore,
    z_offset: f64,
    calibration: Calibration,
    gantry_leveled: bool,
    print_state: PrintState,
    filename: String,
    message: String,
    print_duration: f64,
    total_duration: f64,
    job: Option<PrintJob>,
    excluded_objects: Vec<String>,
    layer_pause: LayerPause,
    /// Simulated seconds since the last command; drives `idle_timeout.state`.
    inactive_seconds: f64,
}

impl Printer {
    pub fn new(time_scale: f64) -> Self {
        Self::with_profile(time_scale, MachineProfile::default())
    }

    /// The Spoolman spool that Happy Hare (the selected gate) or AFC (the lane in the toolhead) makes the active one;
    /// `None` without either or without a spool.
    pub fn selected_gate_spool(&self) -> Option<u32> {
        match &self.afc {
            Some(afc) => afc.active_spool(),
            None => self.mmu.as_ref().and_then(HappyHare::selected_spool),
        }
    }

    /// Whether a filament changer tells Moonraker the active spool.
    pub fn has_mmu(&self) -> bool {
        self.mmu.is_some() || self.afc.is_some()
    }

    pub fn with_profile(time_scale: f64, profile: MachineProfile) -> Self {
        let afc = profile.afc.map(Afc::new);
        let mut printer = Self {
            time_scale,
            uptime: 0.0,
            profile,
            mmu: match (profile.mmu_units, profile.mmu_gates) {
                (Some(units), _) => Some(HappyHare::with_units(units, profile.spoolman)),
                (None, gates) => gates.map(|gates| HappyHare::new(gates, profile.spoolman)),
            },
            toolchanger: if profile.tools > 0 {
                Some(toolchanger::Toolchanger::new(&profile))
            } else {
                afc.as_ref()
                    .filter(|afc| afc.is_toolchanger())
                    .map(|afc| toolchanger::Toolchanger::afc_heads(afc.head_count(), 0))
            },
            afc,
            klippy: KlippyState::Ready,
            extruder: Heater::cold(),
            bed: Heater::cold(),
            chamber: profile.chamber_heater.then(Heater::cold),
            position: [0.0; 4],
            homed: [false; 3],
            relative_positioning: false,
            relative_extrusion: false,
            fan_speed: 0.0,
            speed_factor: 1.0,
            extrude_factor: 1.0,
            pressure_advance: CONFIGURED_PRESSURE_ADVANCE,
            smooth_time: CONFIGURED_SMOOTH_TIME,
            limits: CONFIGURED_LIMITS,
            retraction: profile.devices.then_some(CONFIGURED_RETRACTION),
            devices: profile.devices.then(Devices::default),
            temperatures: TemperatureStore::default(),
            z_offset: 0.0,
            calibration: Calibration::new(profile.probe, profile.bed_mesh, profile.bed_screws),
            gantry_leveled: false,
            print_state: PrintState::Standby,
            filename: String::new(),
            message: String::new(),
            print_duration: 0.0,
            total_duration: 0.0,
            job: None,
            excluded_objects: Vec::new(),
            layer_pause: LayerPause::default(),
            inactive_seconds: BUSY_SECONDS,
        };
        printer.follow_afc_shuttle();
        printer
    }

    /// The file of a running or paused job; Moonraker does not delete it.
    pub fn is_printing(&self, path: &str) -> bool {
        self.print_state.is_active() && self.filename == path
    }

    pub fn print_state(&self) -> PrintState {
        self.print_state
    }

    /// The job as Moonraker's history records it when it ended; `None` while it runs or without a job.
    pub fn ended_job(&self, end_time: f64) -> Option<Value> {
        let status = match self.print_state {
            PrintState::Complete => "completed",
            PrintState::Cancelled => "cancelled",
            PrintState::Error if self.klippy == KlippyState::Shutdown => "klippy_shutdown",
            PrintState::Error => "error",
            PrintState::Standby | PrintState::Printing | PrintState::Paused => return None,
        };
        Some(json!({
            "job_id": format!("{:06X}", end_time as u64),
            "filename": self.filename,
            "status": status,
            "start_time": end_time - self.total_duration,
            "end_time": end_time,
            "print_duration": round(self.print_duration, 2),
            "total_duration": round(self.total_duration, 2),
            "filament_used": round(self.filament_used(), 2),
            "exists": true,
        }))
    }

    pub fn advance(&mut self, elapsed: Duration) {
        let seconds = elapsed.as_secs_f64() * self.time_scale;
        self.uptime += seconds;
        self.extruder.advance(seconds);
        if let Some(toolchanger) = self.toolchanger.as_mut() {
            toolchanger.advance(seconds);
        }
        self.bed.advance(seconds);
        if let Some(chamber) = self.chamber.as_mut() {
            chamber.advance(seconds);
        }
        self.advance_print(seconds);
        self.advance_idle(seconds);
        let samples = self.temperature_samples();
        self.temperatures.advance(seconds, || samples.clone());
    }

    fn host_temperature(&self) -> f64 {
        HOST_TEMPERATURE + HOST_TEMPERATURE_SWING * (self.uptime / 7.0).sin()
    }

    /// `server.temperature_store`.
    pub fn temperature_store(&self) -> Value {
        self.temperatures.to_json()
    }

    fn temperature_samples(&self) -> Vec<(String, Sample)> {
        let heater = |heater: &Heater| Sample {
            temperature: heater.temperature,
            heater: Some((heater.target, if heater.target > heater.temperature { 1.0 } else { 0.0 })),
        };
        let mut samples =
            vec![("extruder".to_owned(), heater(&self.extruder)), ("heater_bed".to_owned(), heater(&self.bed))];
        for (name, extruder) in self.toolchanger.iter().flat_map(|toolchanger| toolchanger.extra_extruders()) {
            samples.push((name, heater(extruder)));
        }
        if let Some(chamber) = &self.chamber {
            samples.push(("heater_generic chamber".to_owned(), heater(chamber)));
        }
        samples.push((
            "temperature_sensor raspberry_pi".to_owned(),
            Sample { temperature: self.host_temperature(), heater: None },
        ));
        samples
    }

    fn advance_idle(&mut self, seconds: f64) {
        if self.print_state == PrintState::Printing {
            self.inactive_seconds = 0.0;
            return;
        }
        let was_idle = self.inactive_seconds >= IDLE_TIMEOUT_SECONDS;
        self.inactive_seconds += seconds;
        if !was_idle && self.inactive_seconds >= IDLE_TIMEOUT_SECONDS {
            self.switch_heaters_off();
            self.homed = [false; 3];
        }
    }

    fn idle_state(&self) -> &'static str {
        if self.print_state == PrintState::Printing || self.inactive_seconds < BUSY_SECONDS {
            "Printing"
        } else if self.inactive_seconds >= IDLE_TIMEOUT_SECONDS {
            "Idle"
        } else {
            "Ready"
        }
    }

    /// Runs one command; a rejected command reaches the tool changer plugin as Klipper's `gcode:command_error` event.
    pub fn apply(&mut self, command: Command) -> Result<(), CommandError> {
        let result = self.apply_command(command);
        if result.is_err() {
            self.note_command_error();
        }
        result
    }

    fn apply_command(&mut self, command: Command) -> Result<(), CommandError> {
        self.ensure_running()?;
        self.inactive_seconds = 0.0;
        match command {
            Command::Home(axes) => {
                self.initialize_toolchanger_on_home()?;
                self.home(&axes)
            }
            Command::SetExtruderTarget { tool, target } => {
                self.extruder_of_tool_mut(tool)?.set_target(target, MAX_EXTRUDER_TARGET)?
            }
            Command::SetBedTarget(target) => self.bed.set_target(target, MAX_BED_TARGET)?,
            Command::SetFanSpeed(speed) => self.set_part_fan_speed(speed),
            Command::AbsolutePositioning => self.relative_positioning = false,
            Command::RelativePositioning => self.relative_positioning = true,
            Command::Move(movement) => self.move_toolhead(movement)?,
            Command::DisableSteppers => self.homed = [false; 3],
            Command::SetGcodeOffset { z, z_adjust } => self.set_z_offset(z, z_adjust),
            Command::AbsoluteExtrusion => self.relative_extrusion = false,
            Command::RelativeExtrusion => self.relative_extrusion = true,
            Command::SetSpeedFactor(factor) => self.speed_factor = factor.max(0.01),
            Command::SetExtrudeFactor(factor) => self.extrude_factor = factor.max(0.01),
            Command::SetPressureAdvance { advance, smooth_time } => {
                self.pressure_advance = advance.unwrap_or(self.pressure_advance).max(0.0);
                self.smooth_time = smooth_time.unwrap_or(self.smooth_time).max(0.0);
            }
            Command::SetVelocityLimit(limit) => self.set_velocity_limit(limit),
            Command::SetRetraction(change) => self.set_retraction(change)?,
            Command::Device(command) => self.apply_device(command)?,
            Command::SetHeaterTemperature { heater, target } => self.set_named_heater(&heater, target)?,
            Command::ExcludeObject { name } => self.exclude_object(&name),
            Command::SaveConfig => self.calibration.save(),
            Command::Calibration(command) => {
                let homed = self.homed.iter().all(|homed| *homed);
                self.calibration.apply(command, homed)?;
            }
            Command::QuadGantryLevel => self.level_gantry()?,
            Command::Mmu(command) => self.apply_mmu(command)?,
            Command::Toolchanger(command) => self.apply_toolchanger(command)?,
            Command::Afc(command) => self.apply_afc(command)?,
            Command::LayerPause(command) => self.layer_pause.apply(command),
            Command::FakePrintError => self.fail_print()?,
            // The session delays the answer of a dwell; the printer itself does nothing.
            Command::Help | Command::Dwell(_) | Command::Ignored => {}
        }
        Ok(())
    }

    pub fn start_print(&mut self, file: &GcodeFile) -> Result<(), CommandError> {
        self.ensure_running()?;
        if self.print_state.is_active() {
            return Err(CommandError::new("Print job already in progress"));
        }
        self.extruder.target = file.first_layer_extruder_temperature;
        self.bed.target = file.first_layer_bed_temperature;
        self.filename = file.path.clone();
        self.message.clear();
        self.print_duration = 0.0;
        self.total_duration = 0.0;
        self.excluded_objects.clear();
        self.job = Some(PrintJob {
            estimated_time: file.estimated_time,
            layer_count: file.layer_count,
            filament_total: file.filament_total(),
            object_definitions: file.object_definitions(),
        });
        self.print_state = PrintState::Printing;
        Ok(())
    }

    pub fn pause_print(&mut self) -> Result<(), CommandError> {
        self.transition(PrintState::Printing, PrintState::Paused)
    }

    pub fn resume_print(&mut self) -> Result<(), CommandError> {
        self.transition(PrintState::Paused, PrintState::Printing)
    }

    pub fn cancel_print(&mut self) -> Result<(), CommandError> {
        self.ensure_running()?;
        if !self.print_state.is_active() {
            return Err(CommandError::new("No active print job"));
        }
        self.print_state = PrintState::Cancelled;
        self.end_job();
        Ok(())
    }

    fn fail_print(&mut self) -> Result<(), CommandError> {
        if !self.print_state.is_active() {
            return Err(CommandError::new("No active print job"));
        }
        self.print_state = PrintState::Error;
        self.message = PRINT_ERROR_MESSAGE.to_owned();
        self.end_job();
        Ok(())
    }

    fn end_job(&mut self) {
        self.switch_heaters_off();
        self.layer_pause.clear();
    }

    /// Like Klipper: `print_stats` has no shutdown handler, virtual_sdcard stops reading and notes a pause, so the
    /// lost print stays "paused" until the restart.
    pub fn emergency_stop(&mut self) {
        self.klippy = KlippyState::Shutdown;
        self.switch_heaters_off();
        if self.print_state == PrintState::Printing {
            self.print_state = PrintState::Paused;
        }
    }

    pub fn restart(&mut self) {
        *self = Self::with_profile(self.time_scale, self.profile);
    }
    /// Klipper text for the current state, as returned by `printer.info`.
    pub fn state_message(&self) -> &'static str {
        self.klippy.message()
    }

    pub fn klippy_state(&self) -> &'static str {
        self.klippy.as_str()
    }

    fn set_named_heater(&mut self, heater: &str, target: f64) -> Result<(), CommandError> {
        if let Some(extruder) = self.toolchanger.as_mut().and_then(|toolchanger| toolchanger.extra_extruder_mut(heater))
        {
            return extruder.set_target(target, MAX_EXTRUDER_TARGET);
        }
        match (heater, self.chamber.as_mut()) {
            ("extruder", _) => self.extruder.set_target(target, MAX_EXTRUDER_TARGET),
            ("heater_bed", _) => self.bed.set_target(target, MAX_BED_TARGET),
            ("chamber", Some(chamber)) => chamber.set_target(target, MAX_CHAMBER_TARGET),
            _ => Err(CommandError::new(format!("Unknown heater '{heater}'"))),
        }
    }

    fn set_velocity_limit(&mut self, limit: VelocityLimit) {
        let limits = &mut self.limits;
        limits.max_velocity = limit.velocity.unwrap_or(limits.max_velocity);
        limits.max_accel = limit.accel.unwrap_or(limits.max_accel);
        limits.square_corner_velocity = limit.square_corner_velocity.unwrap_or(limits.square_corner_velocity);
        limits.minimum_cruise_ratio = limit.minimum_cruise_ratio.unwrap_or(limits.minimum_cruise_ratio);
    }

    fn set_retraction(&mut self, change: RetractionChange) -> Result<(), CommandError> {
        let retraction =
            self.retraction.as_mut().ok_or_else(|| CommandError::new("Unknown command:\"SET_RETRACTION\""))?;
        retraction.retract_length = change.retract_length.unwrap_or(retraction.retract_length);
        retraction.retract_speed = change.retract_speed.unwrap_or(retraction.retract_speed);
        retraction.unretract_extra_length = change.unretract_extra_length.unwrap_or(retraction.unretract_extra_length);
        retraction.unretract_speed = change.unretract_speed.unwrap_or(retraction.unretract_speed);
        Ok(())
    }

    fn apply_device(&mut self, command: DeviceCommand) -> Result<(), CommandError> {
        // `SET_FAN_SPEED` of a head's fan (`toolchanger.py` Z.837-838) is no device of the machine.
        if let DeviceCommand::SetFanSpeed { fan, speed } = &command {
            if let Some(toolchanger) = self.toolchanger.as_mut() {
                if toolchanger.set_head_fan_speed(fan, *speed).is_some() {
                    return Ok(());
                }
            }
        }
        match self.devices.as_mut() {
            Some(devices) => devices.apply(command),
            None => Err(CommandError::new("Unknown command for this machine")),
        }
    }

    fn apply_mmu(&mut self, command: MmuCommand) -> Result<(), CommandError> {
        if let MmuCommand::ChangeTool { tool } = command {
            // AFC registers `T<n>` for its lanes.
            if let Some(result) = self.afc_change_tool(tool) {
                return result;
            }
            // With a tool changer `T<n>` mounts a tool instead of changing a gate; with Happy Hare as well only the
            // numbers of its tools do, the others are gates.
            if self.mmu.is_none() || self.toolchanger_has_tool(tool) {
                if let Some(result) = self.select_tool(tool) {
                    return result;
                }
            }
        }
        match self.mmu.as_mut() {
            Some(mmu) => mmu.apply(command, self.print_state.as_str()),
            None => Err(CommandError::new(format!("Unknown command:\"{}\"", command.gcode_name()))),
        }
    }

    /// Mirrors Klipper's `cmd_EXCLUDE_OBJECT` (klippy/extras/exclude_object.py): names are upper-cased, unknown
    /// names are accepted, the list stays sorted, and a missing NAME only lists the objects (not simulated here).
    /// Without the module the command is unknown and ignored.
    fn exclude_object(&mut self, name: &str) {
        if !self.profile.exclude_object || name.is_empty() {
            return;
        }
        let name = name.to_ascii_uppercase();
        if !self.excluded_objects.contains(&name) {
            self.excluded_objects.push(name);
            self.excluded_objects.sort();
        }
    }

    /// Before the first print no file has defined objects.
    fn object_definitions(&self) -> Definitions {
        self.job.as_ref().map_or(Definitions::None, |job| job.object_definitions)
    }

    /// Klipper keeps the last object while the print is paused.
    fn current_object(&self) -> Option<&'static str> {
        if !self.print_state.is_active() {
            return None;
        }
        print_objects::current(self.object_definitions(), self.layers().0?, &self.excluded_objects)
    }

    fn ensure_running(&self) -> Result<(), CommandError> {
        match self.klippy {
            KlippyState::Ready => Ok(()),
            KlippyState::Shutdown => Err(CommandError::new("Klipper reports shutdown, restart required")),
        }
    }

    fn transition(&mut self, from: PrintState, to: PrintState) -> Result<(), CommandError> {
        self.ensure_running()?;
        if self.print_state != from {
            return Err(CommandError::new(format!("Print job is not {}", from.as_str())));
        }
        self.print_state = to;
        Ok(())
    }

    fn switch_heaters_off(&mut self) {
        self.extruder.target = 0.0;
        if let Some(toolchanger) = self.toolchanger.as_mut() {
            toolchanger.switch_heaters_off();
        }
        self.bed.target = 0.0;
    }

    fn advance_print(&mut self, seconds: f64) {
        match self.print_state {
            PrintState::Printing => {
                let layer_before = self.layers().0;
                self.total_duration += seconds;
                self.print_duration += seconds;
                if self.progress() >= 1.0 {
                    self.print_state = PrintState::Complete;
                    self.end_job();
                } else if let (Some(before), Some(now)) = (layer_before, self.layers().0) {
                    // The slicer's `SET_PRINT_STATS_INFO CURRENT_LAYER=` runs Mainsail's override at every layer;
                    // a long tick of the simulation may pass several.
                    if (before + 1..=now).any(|layer| self.layer_pause.on_layer_change(layer)) {
                        self.print_state = PrintState::Paused;
                    }
                }
            }
            PrintState::Paused => self.total_duration += seconds,
            _ => {}
        }
    }

    fn home(&mut self, axes: &[Axis]) {
        let axes = if axes.is_empty() { &ALL_AXES[..] } else { axes };
        for axis in axes {
            self.homed[axis.index()] = true;
            self.position[axis.index()] = AXIS_MINIMUM[axis.index()];
        }
    }

    fn move_toolhead(&mut self, movement: Move) -> Result<(), CommandError> {
        let mut target = self.position;
        for (axis, value) in movement.linear_axes() {
            let index = axis.index();
            if !self.homed[index] {
                return Err(CommandError::new("Must home axis first"));
            }
            let position = self.resolve(self.position[index], value);
            if !(AXIS_MINIMUM[index]..=AXIS_MAXIMUM[index]).contains(&position) {
                return Err(CommandError::new("Move out of range"));
            }
            target[index] = position;
        }
        if let Some(distance) = movement.e {
            if self.active_extruder_mut().temperature < MIN_EXTRUDE_TEMPERATURE {
                return Err(CommandError::new("Extrude below minimum temp"));
            }
            target[EXTRUDER_INDEX] = if self.relative_positioning || self.relative_extrusion {
                self.position[EXTRUDER_INDEX] + distance
            } else {
                distance
            };
        }
        self.position = target;
        Ok(())
    }

    fn resolve(&self, current: f64, value: f64) -> f64 {
        if self.relative_positioning {
            current + value
        } else {
            value
        }
    }

    fn set_z_offset(&mut self, z: Option<f64>, z_adjust: Option<f64>) {
        if let Some(z) = z {
            self.z_offset = z;
        }
        if let Some(adjustment) = z_adjust {
            self.z_offset += adjustment;
        }
    }

    fn level_gantry(&mut self) -> Result<(), CommandError> {
        if !self.homed.iter().all(|homed| *homed) {
            return Err(CommandError::new("Must home axes first"));
        }
        self.gantry_leveled = true;
        Ok(())
    }

    fn progress(&self) -> f64 {
        self.job.as_ref().map_or(0.0, |job| (self.print_duration / job.estimated_time).min(1.0))
    }

    fn filament_used(&self) -> f64 {
        self.job.as_ref().map_or(0.0, |job| job.filament_total * self.progress())
    }

    fn layers(&self) -> (Option<u32>, Option<u32>) {
        match &self.job {
            Some(job) => {
                let layer = (self.progress() * f64::from(job.layer_count)).floor() as u32 + 1;
                (Some(layer.min(job.layer_count)), Some(job.layer_count))
            }
            None => (None, None),
        }
    }

    fn homed_axes(&self) -> String {
        ALL_AXES.iter().filter(|axis| self.homed[axis.index()]).map(|axis| axis.letter()).collect()
    }

    /// The position in the file's coordinates: while a job runs the nozzle is at the top of the current layer.
    fn gcode_position(&self) -> [f64; 4] {
        let mut position = self.rounded_position();
        position[2] = match self.layers() {
            (Some(layer), _) if self.print_state.is_active() => f64::from(layer) * LAYER_HEIGHT,
            _ => position[2] - self.z_offset,
        };
        position.map(|value| round(value, 3))
    }

    fn rounded_position(&self) -> [f64; 4] {
        self.position.map(|value| round(value, 3))
    }
}

fn step_towards(current: f64, goal: f64, max_step: f64) -> f64 {
    if (goal - current).abs() <= max_step {
        goal
    } else {
        current + max_step * (goal - current).signum()
    }
}

fn round(value: f64, decimals: i32) -> f64 {
    let factor = 10f64.powi(decimals);
    (value * factor).round() / factor
}
