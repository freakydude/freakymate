//! Operator commands for the printer: intents resolved by what the printer has, and which source carries out each
//! concrete command. What a source is told (G-code, Moonraker methods) is known by the source itself.

use crate::domain::filament::{self, FilamentHandling};
use crate::domain::light;
use crate::moonraker::Call;
use crate::sources::happy_hare::commands as mmu;
use crate::sources::klipper::commands::{self as klipper, run_gcode};
use crate::sources::klipper::tuning::TuningKey;
use crate::sources::klipper::{Axis, CalibrationMode, Leveling, Motion, Setup, TestZ};
use crate::sources::macro_conventions::{LOAD_FILAMENT_MACRO, UNLOAD_FILAMENT_MACRO};
use crate::sources::moonraker_server::wled::{self, StripAction};
use crate::sources::moonraker_server::{files, machine, power, update_manager};
use crate::sources::{led_effect, spoolman, PrinterStatus, Sources};

/// A command triggered by a person. The UI only knows this level, not G-code or JSON-RPC.
///
/// Some commands state an intent (`Level`, `ApplyPreset`, `CoolDown`, `StepHeater`, `ApplyZOffset`, `LoadFilament`,
/// `UnloadFilament`, the layer pauses, `CalibrateZ`, `SetGateFilament`, `SetEndlessSpoolGroup` and the commands within a calibration mode); [`Command::resolve`] turns them into the commands this printer needs. Only those reach
/// [`Command::to_call`].
#[derive(Debug, Clone, PartialEq)]
pub enum Command {
    // Print job
    StartPrint {
        filename: String,
    },
    /// Deletes a G-code file; `path` as in the file list. Moonraker refuses the file that is being printed.
    DeleteFile {
        path: String,
    },
    PausePrint,
    ResumePrint,
    CancelPrint,
    /// Plans a pause when the print reaches `layer` with Mainsail's macros, see
    /// [`crate::domain::pause::layer_pause_offer`].
    PauseAtLayer {
        layer: u32,
    },
    /// Takes back a pause planned at a layer, wherever it was planned.
    ClearLayerPause,
    // Motion
    Home {
        x: bool,
        y: bool,
        z: bool,
    },
    /// Relative move; the target is clamped to the axis range.
    Jog {
        axis: Axis,
        distance: f64,
    },
    DisableMotors,
    /// Levels with what the printer has: quad gantry level, else Z tilt.
    Level,
    QuadGantryLevel,
    ZTiltAdjust,
    // Temperatures and fans
    SetHeaterTarget {
        heater: String,
        target: f64,
    },
    /// Raises or lowers the target by `step` (with sign) from the target shown now, see [`crate::sources::klipper::Heater::stepped_target`].
    StepHeater {
        heater: String,
        step: f64,
    },
    /// Heats the nozzle and, if there is one, the bed.
    ApplyPreset {
        extruder: f64,
        bed: f64,
    },
    /// Switches every heater off.
    CoolDown,
    /// `speed` from 0 to 1; only part and generic fans can be set, Klipper drives the others.
    SetFanSpeed {
        fan: String,
        speed: f64,
    },
    /// Light at `level` (0 to 1, 0 is off): on a Klipper LED white light, with the white channel if the LED has one,
    /// else all colours; on a WLED strip its brightness.
    SetLight {
        led: String,
        level: f64,
    },
    /// Switches the main light (see [`light::main_light`]) on or off.
    ToggleLight,
    /// Colour channels from 0 to 1 (`SET_LED`).
    SetLed {
        led: String,
        red: f64,
        green: f64,
        blue: f64,
        white: f64,
    },
    /// Starts or stops an effect of the led_effect plugin.
    SetLedEffect {
        effect: String,
        on: bool,
    },
    // Tuning
    AdjustZOffset {
        delta: f64,
    },
    ResetZOffset,
    /// Applies the babystep offset to the probe if there is one, else to the endstop (followed by `SaveConfig`).
    ApplyZOffset,
    ApplyZOffsetToProbe,
    ApplyZOffsetToEndstop,
    SetSpeedFactor {
        percent: f64,
    },
    /// Sets a motion or extrusion value until the next restart.
    SetTuning {
        key: TuningKey,
        value: f64,
    },
    /// One step up (`direction` 1) or down (-1) from the current value.
    StepTuning {
        key: TuningKey,
        direction: f64,
    },
    /// Back to the configured value.
    ResetTuning {
        key: TuningKey,
    },
    SaveConfig,
    // Calibration
    CalibrateBedMesh,
    LoadBedMesh {
        profile: String,
    },
    ClearBedMesh,
    /// Saves the active mesh as a profile for this session.
    SaveBedMesh {
        profile: String,
    },
    RemoveBedMesh {
        profile: String,
    },
    /// Measures each bed screw with the probe; the result tells how far to turn them.
    CalculateScrewsTilt,
    /// Paper test at each bed screw; Klipper waits in its `bed_screws` mode.
    AdjustBedScrews,
    /// Paper test for the Z height: of the probe if there is one, else of the Z endstop (followed by `SaveConfig`).
    CalibrateZ,
    ProbeCalibrate,
    ZEndstopCalibrate,
    /// Within `manual_probe`; dropped if Klipper is not waiting there, since it knows the command only then.
    TestZ {
        step: TestZ,
    },
    /// Ends the waiting mode with the current height or screw; dropped if none waits.
    AcceptCalibration,
    /// Within `bed_screws`: the screw had to be turned, the round starts over.
    ScrewAdjusted,
    /// Leaves the waiting mode without a result; dropped if none waits.
    AbortCalibration,
    // Extruder
    Extrude {
        length: f64,
        speed: f64,
    },
    /// Loads or unloads with what the printer has: Happy Hare, else the macros `LOAD_FILAMENT`/`UNLOAD_FILAMENT`.
    LoadFilament,
    UnloadFilament,
    // Free-form commands
    RunMacro {
        name: String,
    },
    /// A blank script is ignored.
    RunGcode {
        script: String,
    },
    /// Skips an object of the running print (Klipper `exclude_object`).
    ExcludeObject {
        name: String,
    },
    // Happy Hare. `MMU_LOAD` and `MMU_UNLOAD` act on the selected gate; they have no gate parameter.
    MmuSelectGate {
        gate: usize,
    },
    MmuLoad,
    /// Selects a gate and loads it.
    MmuLoadGate {
        gate: usize,
    },
    MmuUnload,
    /// Ejects the filament of a gate completely; without `gate` the selected one.
    MmuEject {
        gate: Option<usize>,
    },
    MmuHome,
    MmuRecover,
    MmuUnlock,
    MmuMotorsOff,
    MmuCheckGates {
        gates: mmu::Gates,
    },
    /// Grabs filament pushed into a gate and parks it there.
    MmuPreload {
        gate: usize,
    },
    MmuServo {
        position: mmu::ServoCommand,
    },
    /// Selects the bypass for filament fed by hand past the gates.
    MmuSelectBypass,
    /// Selects the bypass and loads the filament fed into it.
    MmuLoadBypass,
    /// Lets `tool` take its filament from `gate`.
    MmuMapTool {
        tool: usize,
        gate: usize,
    },
    /// Back to the configured tool-to-gate map.
    MmuResetToolMap,
    MmuEndlessSpool {
        enabled: bool,
    },
    /// Moves a gate into an EndlessSpool group; the other gates keep theirs.
    SetEndlessSpoolGroup {
        gate: usize,
        group: u32,
    },
    /// Puts a Spoolman spool into a gate; `None` takes the spool out of the record.
    AssignGateSpool {
        gate: usize,
        spool_id: Option<u32>,
    },
    /// Sets material or colour of a gate in Happy Hare's gate map (without Spoolman); `None` keeps the value.
    SetGateFilament {
        gate: usize,
        material: Option<String>,
        color: Option<[u8; 3]>,
    },
    // Spoolman
    /// The spool Moonraker tracks the filament use of; `None` for none.
    SetActiveSpool {
        spool_id: Option<u32>,
    },
    /// Sets the weight of a spool from a measurement; `weight` is the gross weight of the spool in grams.
    MeasureSpool {
        spool_id: u32,
        weight: f64,
    },
    // Klipper and host
    EmergencyStop,
    RestartKlipper,
    FirmwareRestart,
    SetPowerDevice {
        device: String,
        on: bool,
    },
    SetWledStrip {
        strip: String,
        action: StripAction,
    },
    RestartService {
        service: String,
    },
    RebootHost,
    ShutdownHost,
    RefreshUpdates,
    Upgrade {
        name: String,
    },
}

/// Why an intent cannot be carried out on this printer.
const NO_LEVELING: &str = "this printer has neither quad gantry level nor Z tilt";
const NO_LOAD_WAY: &str = "no way to load filament: no Happy Hare and no LOAD_FILAMENT macro";
const NO_UNLOAD_WAY: &str = "no way to unload filament: no Happy Hare and no UNLOAD_FILAMENT macro";
const NO_LIGHT: &str = "this printer has no LED";
const NO_Z_CALIBRATION: &str = "this printer has neither a probe nor a Z endstop with position_endstop";
const NO_LAYER_PAUSE: &str = "this printer has no macro to pause at a layer";
const NO_LAYER_REPORT: &str = "the print file reports no layers";
const NO_MMU: &str = "this printer has no MMU";
const GATE_MAP_FROM_SPOOLMAN: &str =
    "Happy Hare takes material and colour of the gates from Spoolman (spoolman_support: pull)";

/// `level` on a light: white light with the channels a Klipper LED has (see [`Setup::white_light`]), the brightness
/// of a WLED strip.
fn light(setup: &Setup, status: &PrinterStatus, led: &str, level: f64) -> Command {
    if let Some(strip) = wled::strip_name(led) {
        let on = status.leds.iter().any(|candidate| candidate.object == led && candidate.on);
        return Command::SetWledStrip { strip: strip.to_owned(), action: StripAction::light(on, level) };
    }
    let [red, green, blue, white] = setup.white_light(led, level);
    Command::SetLed { led: led.to_owned(), red, green, blue, white }
}

/// A call of one of Mainsail's layer pause macros, see [`crate::sources::mainsail_config`].
fn layer_pause(script: Option<String>) -> Result<Vec<Command>, String> {
    script.map(|script| vec![Command::RunGcode { script }]).ok_or_else(|| NO_LAYER_PAUSE.to_owned())
}

impl Command {
    /// Turns an intent into the commands this printer needs; every other command stays as it is. `status` shows the
    /// heater targets including those sent but not yet reported, so steps build on each other.
    pub fn resolve(self, sources: &Sources, status: &PrinterStatus) -> Result<Vec<Command>, String> {
        let setup = &sources.klipper;
        let mode = status.calibration.mode.as_ref();
        let heater_target = |heater: &str, target: f64| Command::SetHeaterTarget { heater: heater.to_owned(), target };
        let commands = match self {
            Self::Level => match setup.leveling {
                Some(Leveling::QuadGantry) => vec![Self::QuadGantryLevel],
                Some(Leveling::ZTilt) => vec![Self::ZTiltAdjust],
                None => return Err(NO_LEVELING.to_owned()),
            },
            Self::ApplyZOffset if setup.probe => vec![Self::ApplyZOffsetToProbe],
            Self::ApplyZOffset => vec![Self::ApplyZOffsetToEndstop],
            Self::CalibrateZ if setup.probe => vec![Self::ProbeCalibrate],
            Self::CalibrateZ if setup.calibration.z_endstop => vec![Self::ZEndstopCalibrate],
            Self::CalibrateZ => return Err(NO_Z_CALIBRATION.to_owned()),
            Self::TestZ { .. } if !matches!(mode, Some(CalibrationMode::ManualProbe { .. })) => Vec::new(),
            Self::ScrewAdjusted if !matches!(mode, Some(CalibrationMode::BedScrews { .. })) => Vec::new(),
            Self::AcceptCalibration | Self::AbortCalibration if mode.is_none() => Vec::new(),
            Self::ApplyPreset { extruder, bed } => {
                let mut commands = vec![heater_target("extruder", extruder)];
                if status.heater("heater_bed").is_some() {
                    commands.push(heater_target("heater_bed", bed));
                }
                commands
            }
            Self::CoolDown => status.heaters.iter().map(|heater| heater_target(&heater.object, 0.0)).collect(),
            Self::SetLight { led, level } => vec![light(setup, status, &led, level)],
            Self::ToggleLight => {
                let led = light::main_light(sources).ok_or_else(|| NO_LIGHT.to_owned())?;
                let lit = light::main_led(sources, &status.leds).is_some_and(|main| main.on);
                // A WLED strip comes back with its initial preset rather than at full brightness.
                match wled::strip_name(led) {
                    Some(strip) => vec![Self::SetWledStrip {
                        strip: strip.to_owned(),
                        action: if lit { StripAction::Off } else { StripAction::On },
                    }],
                    None => vec![light(setup, status, led, if lit { 0.0 } else { 1.0 })],
                }
            }
            Self::StepTuning { key, direction } => {
                let current = key.current(status).ok_or_else(|| format!("this printer has no {}", key.id()))?;
                vec![Self::SetTuning { key, value: key.stepped(current, direction) }]
            }
            Self::ResetTuning { key } => {
                let value = setup.tuning.get(key).ok_or_else(|| format!("{} is not in the configuration", key.id()))?;
                vec![Self::SetTuning { key, value }]
            }
            Self::StepHeater { heater, step } => {
                let current = status.heater(&heater).ok_or_else(|| format!("unknown heater {heater}"))?;
                vec![heater_target(&heater, current.stepped_target(step))]
            }
            Self::LoadFilament => match filament::handling(sources, status) {
                FilamentHandling::Mmu => vec![Self::MmuLoad],
                FilamentHandling::Macros(macros) if macros.load => {
                    vec![Self::RunMacro { name: LOAD_FILAMENT_MACRO.into() }]
                }
                _ => return Err(NO_LOAD_WAY.to_owned()),
            },
            Self::UnloadFilament => match filament::handling(sources, status) {
                FilamentHandling::Mmu => vec![Self::MmuUnload],
                FilamentHandling::Macros(macros) if macros.unload => {
                    vec![Self::RunMacro { name: UNLOAD_FILAMENT_MACRO.into() }]
                }
                _ => return Err(NO_UNLOAD_WAY.to_owned()),
            },
            Self::PauseAtLayer { layer } => {
                let current = status.job.current_layer.ok_or_else(|| NO_LAYER_REPORT.to_owned())?;
                layer_pause(sources.layer_pause.and_then(|macros| macros.pause_at(layer, current)))?
            }
            Self::ClearLayerPause => {
                let scripts = sources.layer_pause.map(|macros| macros.clear()).unwrap_or_default();
                if scripts.is_empty() {
                    return Err(NO_LAYER_PAUSE.to_owned());
                }
                scripts.into_iter().map(|script| Self::RunGcode { script }).collect()
            }
            Self::SetGateFilament { gate, material, color } => {
                let mmu = status.mmu.as_ref().ok_or_else(|| NO_MMU.to_owned())?;
                if !mmu.gate_map_is_local() {
                    return Err(GATE_MAP_FROM_SPOOLMAN.to_owned());
                }
                let temperature = mmu.gates.get(gate).ok_or_else(|| format!("gate {gate} does not exist"))?.temperature;
                vec![Self::RunGcode { script: mmu::set_gate_filament(gate, material.as_deref(), color, temperature)? }]
            }
            Self::SetEndlessSpoolGroup { gate, group } => {
                let mmu = status.mmu.as_ref().ok_or_else(|| NO_MMU.to_owned())?;
                if gate >= mmu.gates.len() {
                    return Err(format!("gate {gate} does not exist"));
                }
                let groups = mmu
                    .endless_spool_groups_with(gate, group)
                    .ok_or_else(|| "Happy Hare reports no EndlessSpool groups".to_owned())?;
                vec![Self::RunGcode { script: mmu::endless_spool_groups(&groups) }]
            }
            Self::RunGcode { script } => match script.trim() {
                "" => Vec::new(),
                trimmed => vec![Self::RunGcode { script: trimmed.to_owned() }],
            },
            other => vec![other],
        };
        Ok(commands)
    }

    /// The call of the source that carries out this command; `motion` provides the current position for clamped
    /// moves. `None` if the command would have no effect in the current state.
    pub fn to_call(&self, motion: &Motion) -> Option<Call> {
        let call = match self {
            // Klipper
            Self::StartPrint { filename } => klipper::start_print(filename),
            Self::DeleteFile { path } => files::delete_file(path),
            Self::PausePrint => klipper::pause_print(),
            Self::ResumePrint => klipper::resume_print(),
            Self::CancelPrint => klipper::cancel_print(),
            Self::EmergencyStop => klipper::emergency_stop(),
            Self::RestartKlipper => klipper::restart(),
            Self::FirmwareRestart => klipper::firmware_restart(),
            Self::Home { x, y, z } => run_gcode(klipper::home(*x, *y, *z)),
            Self::Jog { axis, distance } => run_gcode(klipper::jog(motion, *axis, *distance)?),
            Self::DisableMotors => run_gcode(klipper::DISABLE_MOTORS),
            Self::QuadGantryLevel => run_gcode(klipper::QUAD_GANTRY_LEVEL),
            Self::ZTiltAdjust => run_gcode(klipper::Z_TILT_ADJUST),
            Self::SetHeaterTarget { heater, target } => run_gcode(klipper::heater_target(heater, *target)),
            Self::SetFanSpeed { fan, speed } => run_gcode(klipper::fan_speed(fan, *speed)?),
            Self::SetLed { led, red, green, blue, white } => {
                run_gcode(klipper::set_led(led, [*red, *green, *blue, *white]))
            }
            Self::SetTuning { key, value } => run_gcode(key.gcode(*value)),
            Self::AdjustZOffset { delta } => run_gcode(klipper::adjust_z_offset(*delta)),
            Self::ResetZOffset => run_gcode(klipper::RESET_Z_OFFSET),
            Self::ApplyZOffsetToProbe => run_gcode(klipper::APPLY_Z_OFFSET_TO_PROBE),
            Self::ApplyZOffsetToEndstop => run_gcode(klipper::APPLY_Z_OFFSET_TO_ENDSTOP),
            Self::SetSpeedFactor { percent } => run_gcode(klipper::speed_factor(*percent)),
            Self::SaveConfig => run_gcode(klipper::SAVE_CONFIG),
            Self::CalibrateBedMesh => run_gcode(klipper::BED_MESH_CALIBRATE),
            Self::LoadBedMesh { profile } => run_gcode(klipper::load_bed_mesh(profile)?),
            Self::ClearBedMesh => run_gcode(klipper::BED_MESH_CLEAR),
            Self::SaveBedMesh { profile } => run_gcode(klipper::save_bed_mesh(profile)?),
            Self::RemoveBedMesh { profile } => run_gcode(klipper::remove_bed_mesh(profile)?),
            Self::CalculateScrewsTilt => run_gcode(klipper::SCREWS_TILT_CALCULATE),
            Self::AdjustBedScrews => run_gcode(klipper::BED_SCREWS_ADJUST),
            Self::ProbeCalibrate => run_gcode(klipper::PROBE_CALIBRATE),
            Self::ZEndstopCalibrate => run_gcode(klipper::Z_ENDSTOP_CALIBRATE),
            Self::TestZ { step } => run_gcode(klipper::test_z(*step)),
            Self::AcceptCalibration => run_gcode(klipper::ACCEPT),
            Self::ScrewAdjusted => run_gcode(klipper::ADJUSTED),
            Self::AbortCalibration => run_gcode(klipper::ABORT),
            Self::Extrude { length, speed } => run_gcode(klipper::extrude(*length, *speed)),
            Self::RunMacro { name } => run_gcode(name.clone()),
            Self::RunGcode { script } => run_gcode(script.clone()),
            Self::ExcludeObject { name } => run_gcode(klipper::exclude_object(name)?),
            // Klipper plugins
            Self::SetLedEffect { effect, on } => run_gcode(led_effect::set_effect(effect, *on)),
            Self::MmuSelectGate { gate } => run_gcode(mmu::select_gate(*gate)),
            Self::MmuLoad => run_gcode(mmu::LOAD),
            Self::MmuLoadGate { gate } => run_gcode(mmu::load_gate(*gate)),
            Self::MmuUnload => run_gcode(mmu::UNLOAD),
            Self::MmuEject { gate } => run_gcode(mmu::eject(*gate)),
            Self::MmuHome => run_gcode(mmu::HOME),
            Self::MmuRecover => run_gcode(mmu::RECOVER),
            Self::MmuUnlock => run_gcode(mmu::UNLOCK),
            Self::MmuMotorsOff => run_gcode(mmu::MOTORS_OFF),
            Self::MmuCheckGates { gates } => run_gcode(mmu::check_gates(*gates)),
            Self::MmuPreload { gate } => run_gcode(mmu::preload(*gate)),
            Self::MmuServo { position } => run_gcode(mmu::servo(*position)),
            Self::MmuSelectBypass => run_gcode(mmu::SELECT_BYPASS),
            Self::MmuLoadBypass => run_gcode(mmu::load_bypass()),
            Self::MmuMapTool { tool, gate } => run_gcode(mmu::map_tool(*tool, *gate)),
            Self::MmuResetToolMap => run_gcode(mmu::RESET_TOOL_MAP),
            Self::MmuEndlessSpool { enabled } => run_gcode(mmu::endless_spool(*enabled)),
            Self::AssignGateSpool { gate, spool_id } => run_gcode(mmu::assign_gate_spool(*gate, *spool_id)),
            // Moonraker's components
            Self::SetActiveSpool { spool_id } => spoolman::set_active_spool(*spool_id),
            Self::MeasureSpool { spool_id, weight } => spoolman::measure_spool(*spool_id, *weight),
            Self::SetPowerDevice { device, on } => power::switch_device(device, *on),
            Self::SetWledStrip { strip, action } => wled::post_strip(strip, *action),
            Self::RestartService { service } => machine::restart_service(service),
            Self::RebootHost => machine::reboot(),
            Self::ShutdownHost => machine::shutdown(),
            Self::RefreshUpdates => update_manager::refresh(),
            Self::Upgrade { name } => update_manager::upgrade(name),
            // Intents are resolved before; none of them is a call of its own.
            Self::Level
            | Self::CalibrateZ
            | Self::StepHeater { .. }
            | Self::ApplyPreset { .. }
            | Self::CoolDown
            | Self::SetLight { .. }
            | Self::ToggleLight
            | Self::StepTuning { .. }
            | Self::ResetTuning { .. }
            | Self::ApplyZOffset
            | Self::LoadFilament
            | Self::UnloadFilament
            | Self::PauseAtLayer { .. }
            | Self::ClearLayerPause
            | Self::SetGateFilament { .. }
            | Self::SetEndlessSpoolGroup { .. } => return None,
        };
        Some(call)
    }

    /// The command changes what Spoolman knows, so the spools have to be read again when it succeeded.
    pub fn changes_spools(&self) -> bool {
        matches!(self, Self::SetActiveSpool { .. } | Self::MeasureSpool { .. } | Self::AssignGateSpool { .. })
    }

    /// Short name for feedback in the UI.
    pub fn describe(&self) -> String {
        match self {
            Self::RunMacro { name } => name.clone(),
            Self::RunGcode { script } => script.lines().next().unwrap_or_default().to_owned(),
            Self::Upgrade { name } => format!("Upgrade {name}"),
            other => spaced_words(format!("{other:?}").split([' ', '{', '(']).next().unwrap_or_default()),
        }
    }

    /// Restarts, reboots and updates end a running print; they are refused while one runs
    /// ([`PrinterStatus::print_running`](crate::sources::PrinterStatus::print_running)).
    pub fn interrupts_print(&self) -> bool {
        matches!(
            self,
            Self::RestartKlipper
                | Self::FirmwareRestart
                | Self::RestartService { .. }
                | Self::RebootHost
                | Self::ShutdownHost
                | Self::Upgrade { .. }
        )
    }

    /// Moves the axes, the extruder or the MMU on its own; refused while the print moves the machine
    /// ([`PrinterStatus::print_moving`](crate::sources::PrinterStatus::print_moving)), allowed while it is paused.
    /// The Move and Filament pages are locked then, too; G-code typed in the console is up to the person.
    pub fn moves_machine(&self) -> bool {
        matches!(
            self,
            Self::Home { .. }
                | Self::Jog { .. }
                | Self::DisableMotors
                | Self::Level
                | Self::QuadGantryLevel
                | Self::ZTiltAdjust
                | Self::CalibrateZ
                | Self::ProbeCalibrate
                | Self::ZEndstopCalibrate
                | Self::CalibrateBedMesh
                | Self::CalculateScrewsTilt
                | Self::AdjustBedScrews
                | Self::Extrude { .. }
                | Self::LoadFilament
                | Self::UnloadFilament
                | Self::MmuSelectGate { .. }
                | Self::MmuLoad
                | Self::MmuLoadGate { .. }
                | Self::MmuUnload
                | Self::MmuEject { .. }
                | Self::MmuHome
                | Self::MmuMotorsOff
                | Self::MmuCheckGates { .. }
                | Self::MmuPreload { .. }
                | Self::MmuServo { .. }
                | Self::MmuSelectBypass
                | Self::MmuLoadBypass
        )
    }
}

/// `RefreshUpdates` → `Refresh Updates`, for names that are shown to people.
fn spaced_words(name: &str) -> String {
    let mut spaced = String::with_capacity(name.len() + 4);
    for (index, letter) in name.char_indices() {
        if index > 0 && letter.is_uppercase() {
            spaced.push(' ');
        }
        spaced.push(letter);
    }
    spaced
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sources::klipper::{PrintJob, WhiteChannel};
    use serde_json::json;

    fn motion_at(position: [f64; 4]) -> Motion {
        Motion {
            position,
            axis_minimum: [0.0, 0.0, -2.0, 0.0],
            axis_maximum: [300.0, 300.0, 250.0, 0.0],
            ..Motion::default()
        }
    }

    fn script(command: Command) -> String {
        let call = command.to_call(&motion_at([10.0, 10.0, 5.0, 0.0])).unwrap();
        assert_eq!(call.method, "printer.gcode.script");
        call.params.unwrap()["script"].as_str().unwrap().to_owned()
    }

    #[test]
    fn each_command_reaches_its_source() {
        let motion = Motion::default();
        let method = |command: Command| command.to_call(&motion).unwrap().method;

        assert_eq!(script(Command::Jog { axis: Axis::Z, distance: 0.1 }), "G90\nG1 Z5.1 F600");
        assert_eq!(script(Command::MmuEject { gate: Some(1) }), "MMU_EJECT GATE=1");
        assert_eq!(script(Command::MmuCheckGates { gates: mmu::Gates::All }), "MMU_CHECK_GATE ALL=1");
        assert_eq!(
            script(Command::SetLedEffect { effect: "rainbow".into(), on: false }),
            "SET_LED_EFFECT EFFECT=rainbow STOP=1"
        );
        assert_eq!(script(Command::RunMacro { name: "PARK".into() }), "PARK");
        assert_eq!(method(Command::StartPrint { filename: "a.gcode".into() }), "printer.print.start");
        assert_eq!(method(Command::DeleteFile { path: "a.gcode".into() }), "server.files.delete_file");
        assert_eq!(method(Command::SetActiveSpool { spool_id: None }), "server.spoolman.post_spool_id");
        assert_eq!(
            method(Command::SetPowerDevice { device: "printer".into(), on: true }),
            "machine.device_power.post_device"
        );
        assert_eq!(method(Command::RebootHost), "machine.reboot");
        assert_eq!(method(Command::Upgrade { name: "klipper".into() }), "machine.update.upgrade");
    }

    #[test]
    fn commands_without_effect_are_not_sent() {
        let at_minimum = motion_at([0.0; 4]);
        assert_eq!(Command::Jog { axis: Axis::X, distance: -1.0 }.to_call(&at_minimum), None);
        assert_eq!(Command::ExcludeObject { name: "Part 2".into() }.to_call(&at_minimum), None);
        assert_eq!(Command::SetFanSpeed { fan: "heater_fan hotend".into(), speed: 1.0 }.to_call(&at_minimum), None);
    }

    fn printer(objects: &[&str]) -> Sources {
        let objects: Vec<String> = objects.iter().map(|name| (*name).to_owned()).collect();
        Sources::detect(&objects, Default::default())
    }

    fn status_with_targets(targets: &[(&str, f64)]) -> PrinterStatus {
        PrinterStatus {
            heaters: targets
                .iter()
                .map(|(object, target)| crate::sources::klipper::Heater {
                    object: (*object).to_owned(),
                    target: *target,
                    min_temp: 10.0,
                    ..crate::sources::klipper::Heater::default()
                })
                .collect(),
            ..PrinterStatus::default()
        }
    }

    fn resolved(command: Command, objects: &[&str]) -> Result<Vec<Command>, String> {
        command.resolve(&printer(objects), &PrinterStatus::default())
    }

    fn set(heater: &str, target: f64) -> Command {
        Command::SetHeaterTarget { heater: heater.into(), target }
    }

    #[test]
    fn leveling_uses_quad_gantry_level_before_z_tilt() {
        assert_eq!(resolved(Command::Level, &["quad_gantry_level", "z_tilt"]), Ok(vec![Command::QuadGantryLevel]));
        assert_eq!(resolved(Command::Level, &["z_tilt"]), Ok(vec![Command::ZTiltAdjust]));
        assert!(resolved(Command::Level, &["extruder"]).is_err());
    }

    #[test]
    fn layer_pauses_call_mainsails_macros_with_documented_parameters_only() {
        let both = ["gcode_macro SET_PAUSE_NEXT_LAYER", "gcode_macro SET_PAUSE_AT_LAYER"];
        let gcode = |script: &str| Command::RunGcode { script: script.into() };
        let at_layer_10 = PrinterStatus {
            job: PrintJob { current_layer: Some(10), ..PrintJob::default() },
            ..PrinterStatus::default()
        };
        let pause_at =
            |layer: u32, objects: &[&str]| Command::PauseAtLayer { layer }.resolve(&printer(objects), &at_layer_10);

        assert_eq!(pause_at(11, &both), Ok(vec![gcode("SET_PAUSE_NEXT_LAYER ENABLE=1")]));
        assert_eq!(pause_at(42, &both), Ok(vec![gcode("SET_PAUSE_AT_LAYER ENABLE=1 LAYER=42")]));
        assert!(pause_at(42, &["gcode_macro SET_PAUSE_NEXT_LAYER"]).is_err());
        assert!(resolved(Command::PauseAtLayer { layer: 42 }, &both).is_err());
        assert_eq!(
            resolved(Command::ClearLayerPause, &both),
            Ok(vec![gcode("SET_PAUSE_NEXT_LAYER ENABLE=0"), gcode("SET_PAUSE_AT_LAYER ENABLE=0")])
        );
        assert_eq!(
            resolved(Command::ClearLayerPause, &["gcode_macro SET_PAUSE_AT_LAYER"]),
            Ok(vec![gcode("SET_PAUSE_AT_LAYER ENABLE=0")])
        );
        assert!(resolved(Command::ClearLayerPause, &["extruder"]).is_err());
    }

    #[test]
    fn the_z_offset_goes_to_the_probe_if_there_is_one() {
        assert_eq!(resolved(Command::ApplyZOffset, &["probe"]), Ok(vec![Command::ApplyZOffsetToProbe]));
        assert_eq!(resolved(Command::ApplyZOffset, &["extruder"]), Ok(vec![Command::ApplyZOffsetToEndstop]));
        assert_eq!(script(Command::ApplyZOffsetToProbe), "Z_OFFSET_APPLY_PROBE");
        assert_eq!(script(Command::ApplyZOffsetToEndstop), "Z_OFFSET_APPLY_ENDSTOP");
    }

    #[test]
    fn the_z_height_is_calibrated_at_the_probe_else_at_the_endstop() {
        assert_eq!(resolved(Command::CalibrateZ, &["probe"]), Ok(vec![Command::ProbeCalibrate]));
        assert!(resolved(Command::CalibrateZ, &["extruder"]).is_err());
        let mut sources = printer(&["extruder"]);
        sources.klipper.calibration.z_endstop = true;
        let resolved = Command::CalibrateZ.resolve(&sources, &PrinterStatus::default());
        assert_eq!(resolved, Ok(vec![Command::ZEndstopCalibrate]));
        assert_eq!(script(Command::ZEndstopCalibrate), "Z_ENDSTOP_CALIBRATE");
    }

    #[test]
    fn mode_commands_reach_klipper_only_while_it_waits() {
        let in_mode = |mode: Option<CalibrationMode>, command: Command| {
            let mut status = PrinterStatus::default();
            status.calibration.mode = mode;
            command.resolve(&printer(&["probe"]), &status).map(|commands| commands.len())
        };
        let probing = || Some(CalibrationMode::ManualProbe { z: 1.0, lower: None, upper: None });
        let screws =
            || Some(CalibrationMode::BedScrews { screw: "front left".into(), fine: false, accepted: 0, screws: 4 });
        let test_z = || Command::TestZ { step: TestZ::By(0.1) };

        assert_eq!(in_mode(probing(), test_z()), Ok(1));
        assert_eq!(in_mode(screws(), test_z()), Ok(0));
        assert_eq!(in_mode(screws(), Command::ScrewAdjusted), Ok(1));
        assert_eq!(in_mode(probing(), Command::ScrewAdjusted), Ok(0));
        assert_eq!(in_mode(probing(), Command::AbortCalibration), Ok(1));
        assert_eq!(in_mode(None, Command::AbortCalibration), Ok(0));
        assert_eq!(in_mode(None, Command::AcceptCalibration), Ok(0));
        assert_eq!(script(Command::AbortCalibration), "ABORT");
    }

    #[test]
    fn a_preset_heats_the_bed_only_if_there_is_one() {
        let preset = Command::ApplyPreset { extruder: 210.0, bed: 60.0 };

        let with_bed = status_with_targets(&[("extruder", 0.0), ("heater_bed", 0.0)]);
        let without_bed = status_with_targets(&[("extruder", 0.0)]);

        assert_eq!(
            preset.clone().resolve(&printer(&[]), &with_bed),
            Ok(vec![set("extruder", 210.0), set("heater_bed", 60.0)])
        );
        assert_eq!(preset.resolve(&printer(&[]), &without_bed), Ok(vec![set("extruder", 210.0)]));
    }

    #[test]
    fn cooling_down_switches_every_heater_off() {
        let status =
            status_with_targets(&[("extruder", 210.0), ("heater_bed", 60.0), ("heater_generic chamber", 40.0)]);

        let commands = Command::CoolDown.resolve(&printer(&[]), &status);

        assert_eq!(
            commands,
            Ok(vec![set("extruder", 0.0), set("heater_bed", 0.0), set("heater_generic chamber", 0.0)])
        );
    }

    #[test]
    fn a_step_starts_from_the_target_shown_now() {
        let status = status_with_targets(&[("extruder", 200.0)]);
        let step = |step| Command::StepHeater { heater: "extruder".into(), step };

        assert_eq!(step(5.0).resolve(&printer(&[]), &status), Ok(vec![set("extruder", 205.0)]));
        assert_eq!(step(-5.0).resolve(&printer(&[]), &status), Ok(vec![set("extruder", 195.0)]));
        assert!(Command::StepHeater { heater: "nozzle".into(), step: 5.0 }.resolve(&printer(&[]), &status).is_err());
    }

    #[test]
    fn loading_prefers_happy_hare_then_the_macros() {
        let macro_named = |name: &str| Command::RunMacro { name: name.into() };
        let happy_hare = PrinterStatus {
            mmu: Some(crate::sources::happy_hare::MmuStatus {
                version: "4.1.0".into(),
                enabled: true,
                ..Default::default()
            }),
            ..PrinterStatus::default()
        };
        let with_happy_hare = |command: Command| command.resolve(&printer(&["gcode_macro LOAD_FILAMENT"]), &happy_hare);

        assert_eq!(with_happy_hare(Command::LoadFilament), Ok(vec![Command::MmuLoad]));
        assert_eq!(with_happy_hare(Command::UnloadFilament), Ok(vec![Command::MmuUnload]));
        assert_eq!(
            resolved(Command::LoadFilament, &["gcode_macro LOAD_FILAMENT"]),
            Ok(vec![macro_named("LOAD_FILAMENT")])
        );
        assert_eq!(
            resolved(Command::UnloadFilament, &["gcode_macro UNLOAD_FILAMENT"]),
            Ok(vec![macro_named("UNLOAD_FILAMENT")])
        );
        assert!(resolved(Command::LoadFilament, &["gcode_macro UNLOAD_FILAMENT"]).is_err());
        assert!(resolved(Command::UnloadFilament, &["extruder"]).is_err());
    }

    #[test]
    fn a_gate_is_loaded_after_selecting_it_in_the_same_script() {
        assert_eq!(script(Command::MmuLoadGate { gate: 2 }), "MMU_SELECT GATE=2\nMMU_LOAD");
    }

    #[test]
    fn the_filament_of_a_gate_is_set_unless_happy_hare_takes_it_from_spoolman() {
        let with_support = |spoolman_support: &str| PrinterStatus {
            mmu: Some(crate::sources::happy_hare::MmuStatus {
                spoolman_support: spoolman_support.into(),
                gates: vec![crate::sources::happy_hare::MmuGate { temperature: Some(215.0), ..Default::default() }],
                ..Default::default()
            }),
            ..PrinterStatus::default()
        };
        let red = |gate| Command::SetGateFilament { gate, material: None, color: Some([255, 0, 0]) };

        assert_eq!(
            red(0).resolve(&printer(&[]), &with_support("off")),
            Ok(vec![Command::RunGcode { script: "MMU_GATE_MAP GATE=0 COLOR=ff0000 TEMP=215".into() }])
        );
        assert!(red(0).resolve(&printer(&[]), &with_support("pull")).is_err());
        assert!(red(1).resolve(&printer(&[]), &with_support("off")).is_err());
        assert!(red(0).resolve(&printer(&[]), &PrinterStatus::default()).is_err());
    }

    #[test]
    fn bypass_tool_map_and_endless_spool_are_happy_hare_gcode() {
        assert_eq!(script(Command::MmuSelectBypass), "MMU_SELECT BYPASS=1");
        assert_eq!(script(Command::MmuLoadBypass), "MMU_SELECT BYPASS=1\nMMU_LOAD");
        assert_eq!(script(Command::MmuMapTool { tool: 3, gate: 1 }), "MMU_TTG_MAP TOOL=3 GATE=1");
        assert_eq!(script(Command::MmuResetToolMap), "MMU_TTG_MAP RESET=1");
        assert_eq!(script(Command::MmuEndlessSpool { enabled: true }), "MMU_ENDLESS_SPOOL ENABLE=1");
    }

    #[test]
    fn an_endless_spool_group_is_sent_with_the_groups_of_all_other_gates() {
        let with_groups = |groups: &[Option<u32>]| PrinterStatus {
            mmu: Some(crate::sources::happy_hare::MmuStatus {
                gates: groups
                    .iter()
                    .enumerate()
                    .map(|(index, group)| crate::sources::happy_hare::MmuGate {
                        index,
                        endless_spool_group: *group,
                        ..Default::default()
                    })
                    .collect(),
                ..Default::default()
            }),
            ..PrinterStatus::default()
        };
        let move_gate = |gate| Command::SetEndlessSpoolGroup { gate, group: 0 };
        let groups = [Some(0), Some(1), Some(2)];

        assert_eq!(
            move_gate(2).resolve(&printer(&[]), &with_groups(&groups)),
            Ok(vec![Command::RunGcode { script: "MMU_ENDLESS_SPOOL GROUPS=0,1,0".into() }])
        );
        assert!(move_gate(3).resolve(&printer(&[]), &with_groups(&groups)).is_err());
        assert!(move_gate(0).resolve(&printer(&[]), &with_groups(&[None, None])).is_err());
    }

    #[test]
    fn typed_gcode_is_trimmed_and_blank_input_is_dropped() {
        let gcode = |script: &str| Command::RunGcode { script: script.into() };

        assert_eq!(resolved(gcode("  G28 \n"), &[]), Ok(vec![gcode("G28")]));
        assert_eq!(resolved(gcode("   "), &[]), Ok(Vec::new()));
    }

    #[test]
    fn other_commands_stay_as_they_are() {
        assert_eq!(resolved(Command::PausePrint, &[]), Ok(vec![Command::PausePrint]));
    }

    #[test]
    fn intents_are_no_calls_of_their_own() {
        assert_eq!(Command::Level.to_call(&Motion::default()), None);
        assert_eq!(Command::CoolDown.to_call(&Motion::default()), None);
    }

    #[test]
    fn spool_commands_make_the_spools_be_read_again() {
        assert!(Command::MeasureSpool { spool_id: 1, weight: 1.0 }.changes_spools());
        assert!(Command::AssignGateSpool { gate: 0, spool_id: None }.changes_spools());
        assert!(!Command::PausePrint.changes_spools());
    }

    #[test]
    fn describes_commands_briefly() {
        assert_eq!(Command::PausePrint.describe(), "Pause Print");
        assert_eq!(Command::RefreshUpdates.describe(), "Refresh Updates");
        assert_eq!(Command::Jog { axis: Axis::X, distance: 1.0 }.describe(), "Jog");
        assert_eq!(Command::RunMacro { name: "PARK".into() }.describe(), "PARK");
        assert_eq!(Command::Upgrade { name: "klipper".into() }.describe(), "Upgrade klipper");
    }

    #[test]
    fn light_uses_white_where_there_is_one_and_toggles_the_main_light() {
        let mut sources = printer(&["neopixel sb", "neopixel chamber"]);
        sources.klipper.led_white =
            vec![("neopixel chamber".into(), WhiteChannel::All), ("neopixel sb".into(), WhiteChannel::Some)];
        let mut status = PrinterStatus {
            leds: vec![crate::sources::klipper::Led {
                object: "neopixel chamber".into(),
                label: "Chamber".into(),
                on: false,
                level: Some(0.0),
            }],
            ..PrinterStatus::default()
        };

        let on = Command::ToggleLight.resolve(&sources, &status).unwrap();
        assert_eq!(script(on[0].clone()), "SET_LED LED=chamber RED=0.00 GREEN=0.00 BLUE=0.00 WHITE=1.00 SYNC=0");
        status.leds[0].on = true;
        let off = Command::ToggleLight.resolve(&sources, &status).unwrap();
        assert_eq!(script(off[0].clone()), "SET_LED LED=chamber RED=0.00 GREEN=0.00 BLUE=0.00 WHITE=0.00 SYNC=0");
        let mixed = Command::SetLight { led: "neopixel sb".into(), level: 0.5 }.resolve(&sources, &status).unwrap();
        assert_eq!(script(mixed[0].clone()), "SET_LED LED=sb RED=0.50 GREEN=0.50 BLUE=0.50 WHITE=0.50 SYNC=0");
        let rgb = Command::SetLight { led: "neopixel status".into(), level: 0.5 }.resolve(&sources, &status).unwrap();
        assert_eq!(script(rgb[0].clone()), "SET_LED LED=status RED=0.50 GREEN=0.50 BLUE=0.50 WHITE=0.00 SYNC=0");
        assert!(Command::ToggleLight.resolve(&printer(&[]), &status).is_err());
    }

    #[test]
    fn a_wled_strip_is_switched_and_dimmed_by_moonraker() {
        let mut sources = printer(&["neopixel sb"]);
        sources.wled_strips = vec!["wled chamber".into()];
        let strip = wled::WledStrip { name: "chamber".into(), on: false, brightness: None };
        let mut status = PrinterStatus { leds: vec![strip.led()], ..PrinterStatus::default() };
        let resolve = |command: Command, status: &PrinterStatus| command.resolve(&sources, status).unwrap();
        let wled = |action| vec![Command::SetWledStrip { strip: "chamber".into(), action }];
        let dim = Command::SetLight { led: "wled chamber".into(), level: 0.5 };

        assert_eq!(resolve(Command::ToggleLight, &status), wled(StripAction::On));
        assert_eq!(resolve(dim.clone(), &status), wled(StripAction::OnAt(0.5)));
        status.leds[0].on = true;
        assert_eq!(resolve(Command::ToggleLight, &status), wled(StripAction::Off));
        assert_eq!(resolve(dim, &status), wled(StripAction::Dim(0.5)));
        let call = Command::SetWledStrip { strip: "chamber".into(), action: StripAction::Off }
            .to_call(&Motion::default())
            .unwrap();
        assert_eq!(call.method, "machine.wled.post_strip");
    }

    #[test]
    fn tuning_steps_from_the_current_value_and_resets_to_the_configuration() {
        let mut sources = printer(&[]);
        sources.apply_settings(Some(&json!({ "printer": { "max_accel": 5000.0 } })));
        let mut status = PrinterStatus::default();
        status.motion.max_accel = 3000.0;

        let step = Command::StepTuning { key: TuningKey::MaxAccel, direction: 1.0 }.resolve(&sources, &status).unwrap();
        assert_eq!(step, [Command::SetTuning { key: TuningKey::MaxAccel, value: 3500.0 }]);
        let reset = Command::ResetTuning { key: TuningKey::MaxAccel }.resolve(&sources, &status).unwrap();
        assert_eq!(script(reset[0].clone()), "SET_VELOCITY_LIMIT ACCEL=5000");
        assert!(Command::ResetTuning { key: TuningKey::SmoothTime }.resolve(&sources, &status).is_err());
        assert!(Command::StepTuning { key: TuningKey::RetractLength, direction: 1.0 }
            .resolve(&sources, &status)
            .is_err());
    }
}
