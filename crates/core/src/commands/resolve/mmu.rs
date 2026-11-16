//! Happy Hare's gate map and EndlessSpool groups.

use super::lanes::is_new_material;
use crate::commands::{Command, CommandError};
use crate::sources::PrinterStatus;
use crate::sources::happy_hare::commands as mmu;

pub(super) fn set_gate_filament(
    status: &PrinterStatus,
    gate: usize,
    material: Option<String>,
    color: Option<[u8; 3]>,
) -> Result<Vec<Command>, CommandError> {
    let state = status.mmu.as_ref().ok_or(CommandError::NoMmu)?;
    if !state.gate_map_is_local() {
        return Err(CommandError::GateMapFromSpoolman);
    }
    let current = state.gates.get(gate).ok_or(CommandError::UnknownGate(gate))?;
    // The old temperature belongs to the old material (ASA at 255 °C stays on a gate switched to PLA otherwise).
    let temperature = if is_new_material(&current.material, material.as_deref()) {
        Some(state.default_extruder_temp)
    } else {
        current.temperature
    };
    Ok(vec![Command::RunGcode { script: mmu::set_gate_filament(gate, material.as_deref(), color, temperature)? }])
}

pub(super) fn set_gate_temperature(
    status: &PrinterStatus,
    gate: usize,
    temperature: f64,
) -> Result<Vec<Command>, CommandError> {
    let state = status.mmu.as_ref().ok_or(CommandError::NoMmu)?;
    if !state.gate_map_is_local() {
        return Err(CommandError::GateMapFromSpoolman);
    }
    state.gates.get(gate).ok_or(CommandError::UnknownGate(gate))?;
    Ok(vec![Command::RunGcode { script: mmu::set_gate_filament(gate, None, None, Some(temperature))? }])
}

pub(super) fn set_endless_spool_group(
    status: &PrinterStatus,
    gate: usize,
    group: u32,
) -> Result<Vec<Command>, CommandError> {
    let state = status.mmu.as_ref().ok_or(CommandError::NoMmu)?;
    if gate >= state.gates.len() {
        return Err(CommandError::UnknownGate(gate));
    }
    let groups = state.endless_spool_groups_with(gate, group).ok_or(CommandError::NoEndlessSpoolGroups)?;
    Ok(vec![Command::RunGcode { script: mmu::endless_spool_groups(&groups) }])
}
