//! Operator commands for the printer: intents resolved by what the printer has, and which source carries out each
//! concrete command. What a source is told (G-code, Moonraker methods) is known by the source itself.

mod error;
mod resolve;
#[cfg(test)]
mod tests;

pub use error::CommandError;

use crate::domain::machine_tree::LaneId;
use crate::moonraker::Call;
use crate::sources::happy_hare::commands as mmu;
use crate::sources::klipper::commands::{self as klipper, run_gcode};
use crate::sources::klipper::tuning::TuningKey;
use crate::sources::klipper::{Axis, Motion, TestZ};
use crate::sources::moonraker_server::wled::{self, StripAction};
use crate::sources::moonraker_server::{files, machine, power, update_manager};
use crate::sources::{led_effect, spoolman, toolchanger};

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
    /// Picks up the head with this name (`SELECT_TOOL`) on a tool changer.
    PickUpHead {
        head: String,
    },
    /// Parks the mounted head (`UNSELECT_TOOL`).
    DropHead,
    /// Initialises the tool changer; `recover` also restores the position after an error.
    InitializeToolchanger {
        recover: bool,
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
    // A lane of Happy Hare or AFC alike (`docs/multi-tool.md`, D2); resolved into the commands of its source.
    /// Loads the lane into the toolhead (unloading another one first where the source does that).
    LoadLane {
        lane: LaneId,
    },
    /// Unloads the lane from the toolhead.
    UnloadLane {
        lane: LaneId,
    },
    /// Ejects the filament of the lane out of its unit.
    EjectLane {
        lane: LaneId,
    },
    /// Puts a Spoolman spool into the lane; `None` takes it out.
    AssignLaneSpool {
        lane: LaneId,
        spool_id: Option<u32>,
    },
    /// Sets material or colour of the lane where the source keeps them; `None` keeps the value.
    SetLaneFilament {
        lane: LaneId,
        material: Option<String>,
        color: Option<[u8; 3]>,
    },
    // Spoolman
    /// The spool Moonraker tracks the filament use of; `None` for none.
    SetActiveSpool {
        spool_id: Option<u32>,
    },
    /// Puts a Spoolman spool into the variable `spool_id` of the tool macro `T<n>` (the convention of Fluidd and
    /// Mainsail); `None` takes it out. Resolved into G-code that also saves it where `[save_variables]` exists.
    AssignToolSpool {
        tool: usize,
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

impl Command {
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
            Self::PickUpHead { head } => run_gcode(toolchanger::commands::pick_up(head)?),
            Self::DropHead => run_gcode(toolchanger::commands::DROP),
            Self::InitializeToolchanger { recover } => run_gcode(toolchanger::commands::initialize(*recover)),
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
            | Self::SetEndlessSpoolGroup { .. }
            | Self::LoadLane { .. }
            | Self::UnloadLane { .. }
            | Self::EjectLane { .. }
            | Self::AssignLaneSpool { .. }
            | Self::SetLaneFilament { .. }
            | Self::AssignToolSpool { .. } => return None,
        };
        Some(call)
    }

    /// The command changes what Spoolman knows, so the spools have to be read again when it succeeded.
    pub fn changes_spools(&self) -> bool {
        matches!(self, Self::SetActiveSpool { .. } | Self::MeasureSpool { .. } | Self::AssignGateSpool { .. })
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

    /// Changes the position or the leveling a paused print resumes from (homing, leveling, probing, motors off); refused
    /// for the whole print, pause included. What a pause is for (moving away, extruding, the MMU, the heads) stays
    /// allowed.
    pub fn loses_print_position(&self) -> bool {
        matches!(
            self,
            Self::Home { .. }
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
                | Self::PickUpHead { .. }
                | Self::DropHead
                | Self::InitializeToolchanger { .. }
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
                | Self::LoadLane { .. }
                | Self::UnloadLane { .. }
                | Self::EjectLane { .. }
        )
    }
}
