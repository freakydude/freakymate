//! Turns the intents of [`Command`] into the commands a printer needs, area by area.

mod calibration;
mod filament;
mod heating;
mod layer_pause;
mod lighting;
mod mmu;

use super::{Command, CommandError};
use crate::sources::{PrinterStatus, Sources};

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
            Self::RunGcode { script } => match script.trim() {
                "" => Vec::new(),
                trimmed => vec![Self::RunGcode { script: trimmed.to_owned() }],
            },
            other => vec![other],
        };
        Ok(commands)
    }
}
