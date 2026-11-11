//! Turns the intents of [`Command`] into the commands a printer needs, area by area.

mod calibration;
mod filament;
mod heating;
mod lanes;
mod layer_pause;
mod lighting;
mod mmu;

use super::{Command, CommandError};
use crate::sources::{afc, tool_macros, PrinterStatus, Sources};

impl Command {
    /// Turns an intent into the commands this printer needs; every other command stays as it is. `status` shows the
    /// heater targets including those sent but not yet reported, so steps build on each other.
    pub fn resolve(self, sources: &Sources, status: &PrinterStatus) -> Result<Vec<Command>, CommandError> {
        let setup = &sources.klipper;
        let mode = status.calibration.mode.as_ref();
        let commands = match self {
            Self::Level => calibration::level(setup)?,
            Self::ApplyZOffset => calibration::apply_z_offset(setup),
            Self::CalibrateZ => calibration::calibrate_z(setup)?,
            Self::TestZ { .. } | Self::ScrewAdjusted | Self::AcceptCalibration | Self::AbortCalibration => {
                calibration::within_mode(self, mode)
            }
            Self::SetHeaterTarget { heater, target } => vec![heating::target(&heater, target)],
            Self::ApplyPreset { extruder, bed } => heating::preset(status, extruder, bed),
            Self::CoolDown => heating::cool_down(status),
            Self::StepHeater { heater, step } => heating::step(status, &heater, step)?,
            Self::StepTuning { key, direction } => heating::step_tuning(status, key, direction)?,
            Self::ResetTuning { key } => heating::reset_tuning(setup, key)?,
            Self::SetLight { led, level } => vec![lighting::light(setup, status, &led, level)],
            Self::ToggleLight => lighting::toggle(sources, status)?,
            Self::LoadFilament => filament::load(sources, status)?,
            Self::UnloadFilament => filament::unload(sources, status)?,
            Self::PauseAtLayer { layer } => layer_pause::pause_at(sources, status, layer)?,
            Self::ClearLayerPause => layer_pause::clear(sources)?,
            Self::SetGateFilament { gate, material, color } => mmu::set_gate_filament(status, gate, material, color)?,
            Self::SetEndlessSpoolGroup { gate, group } => mmu::set_endless_spool_group(status, gate, group)?,
            Self::LoadLane { lane } => lanes::load(status, lane)?,
            Self::UnloadLane { lane } => lanes::unload(status, lane)?,
            Self::EjectLane { lane } => lanes::eject(status, lane)?,
            Self::AssignLaneSpool { lane, spool_id } => lanes::assign_spool(status, lane, spool_id)?,
            Self::SetLaneFilament { lane, material, color } => lanes::set_filament(status, lane, material, color)?,
            Self::MapToolToLane { tool, lane } => lanes::map_tool(status, tool, lane)?,
            Self::ResetToolMap => lanes::reset_tool_map(status)?,
            Self::AfcResetFailure => vec![lanes::afc_reset_failure(status)?],
            Self::AfcResetLane { lane } => vec![lanes::afc_reset_lane(status, &lane)?],
            Self::AssignToolSpool { tool, spool_id } => vec![assign_tool_spool(sources, status, tool, spool_id)?],
            Self::RunGcode { script } => match script.trim() {
                "" => Vec::new(),
                trimmed => vec![Self::RunGcode { script: trimmed.to_owned() }],
            },
            other => vec![other],
        };
        Ok(commands)
    }
}

/// `spool_id` goes into the AFC lane mapped to the tool, else into the tool macro under the name Klipper reports, so
/// `SET_GCODE_VARIABLE` finds it.
fn assign_tool_spool(
    sources: &Sources,
    status: &PrinterStatus,
    tool: usize,
    spool_id: Option<u32>,
) -> Result<Command, CommandError> {
    let afc_lane = status.afc.iter().flat_map(|afc| &afc.lanes).find(|lane| lane.tool == Some(tool));
    if let Some(lane) = afc_lane {
        return Ok(Command::RunGcode { script: afc::assign_spool(&lane.name, spool_id) });
    }
    let declared = status.tool_spools.iter().find(|spool| spool.tool == tool);
    let macro_name = &declared.ok_or(CommandError::NoToolSpoolVariable(tool))?.macro_name;
    let script = tool_macros::assign_spool(macro_name, spool_id, sources.klipper.save_variables);
    Ok(Command::RunGcode { script })
}
