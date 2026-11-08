//! Happy Hare's gate map and EndlessSpool groups.

use crate::commands::{Command, CommandError};
use crate::sources::happy_hare::commands as mmu;
use crate::sources::PrinterStatus;

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
    let temperature = state.gates.get(gate).ok_or(CommandError::UnknownGate(gate))?.temperature;
    Ok(vec![Command::RunGcode { script: mmu::set_gate_filament(gate, material.as_deref(), color, temperature)? }])
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
