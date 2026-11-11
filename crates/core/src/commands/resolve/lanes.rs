//! The commands of a lane, the same for a gate of Happy Hare and a lane of AFC: each source gets its own G-code.

use super::mmu;
use crate::commands::{Command, CommandError};
use crate::domain::machine_tree::LaneId;
use crate::sources::{afc, PrinterStatus};

pub(super) fn load(status: &PrinterStatus, lane: LaneId) -> Result<Vec<Command>, CommandError> {
    Ok(vec![match lane {
        LaneId::Gate(gate) => Command::MmuLoadGate { gate },
        LaneId::Bypass => Command::MmuLoadBypass,
        LaneId::Afc(name) => gcode(afc::load_lane(afc_lane(status, &name)?)),
    }])
}

/// Happy Hare unloads the loaded filament whichever gate it came from; AFC names the lane.
pub(super) fn unload(status: &PrinterStatus, lane: LaneId) -> Result<Vec<Command>, CommandError> {
    Ok(vec![match lane {
        LaneId::Gate(_) | LaneId::Bypass => Command::MmuUnload,
        LaneId::Afc(name) => gcode(afc::unload_lane(afc_lane(status, &name)?)),
    }])
}

pub(super) fn eject(status: &PrinterStatus, lane: LaneId) -> Result<Vec<Command>, CommandError> {
    Ok(vec![match lane {
        LaneId::Gate(gate) => Command::MmuEject { gate: Some(gate) },
        LaneId::Bypass => return Err(CommandError::NotForBypass),
        LaneId::Afc(name) => gcode(afc::eject_lane(afc_lane(status, &name)?)),
    }])
}

pub(super) fn assign_spool(
    status: &PrinterStatus,
    lane: LaneId,
    spool_id: Option<u32>,
) -> Result<Vec<Command>, CommandError> {
    Ok(vec![match lane {
        LaneId::Gate(gate) => Command::AssignGateSpool { gate, spool_id },
        LaneId::Bypass => return Err(CommandError::NotForBypass),
        LaneId::Afc(name) => gcode(afc::assign_spool(afc_lane(status, &name)?, spool_id)),
    }])
}

pub(super) fn set_filament(
    status: &PrinterStatus,
    lane: LaneId,
    material: Option<String>,
    color: Option<[u8; 3]>,
) -> Result<Vec<Command>, CommandError> {
    match lane {
        LaneId::Gate(gate) => mmu::set_gate_filament(status, gate, material, color),
        LaneId::Bypass => Err(CommandError::NotForBypass),
        LaneId::Afc(name) => {
            let script = afc::set_filament(afc_lane(status, &name)?, material.as_deref(), color);
            Ok(if script.is_empty() { Vec::new() } else { vec![gcode(script)] })
        }
    }
}

pub(super) fn map_tool(status: &PrinterStatus, tool: usize, lane: LaneId) -> Result<Vec<Command>, CommandError> {
    Ok(vec![match lane {
        LaneId::Gate(gate) => Command::MmuMapTool { tool, gate },
        LaneId::Bypass => return Err(CommandError::NotForBypass),
        LaneId::Afc(name) => gcode(afc::map_tool(afc_lane(status, &name)?, tool)),
    }])
}

/// The tool map of the source in charge: Happy Hare while it has the say, else AFC.
pub(super) fn reset_tool_map(status: &PrinterStatus) -> Result<Vec<Command>, CommandError> {
    if status.happy_hare().is_some() {
        return Ok(vec![Command::MmuResetToolMap]);
    }
    status.afc.as_ref().ok_or(CommandError::NoAfc)?;
    Ok(vec![gcode(afc::RESET_TOOL_MAP.to_owned())])
}

pub(super) fn afc_reset_failure(status: &PrinterStatus) -> Result<Command, CommandError> {
    status.afc.as_ref().ok_or(CommandError::NoAfc)?;
    Ok(gcode(afc::RESET_FAILURE.to_owned()))
}

pub(super) fn afc_reset_lane(status: &PrinterStatus, name: &str) -> Result<Command, CommandError> {
    Ok(gcode(afc::reset_lane(afc_lane(status, name)?)))
}

/// The name of a lane AFC reports; AFC only logs an unknown lane, so it is refused here.
fn afc_lane<'a>(status: &PrinterStatus, name: &'a str) -> Result<&'a str, CommandError> {
    let afc = status.afc.as_ref().ok_or(CommandError::NoAfc)?;
    afc.lane(name).map(|_| name).ok_or_else(|| CommandError::UnknownLane(name.to_owned()))
}

fn gcode(script: String) -> Command {
    Command::RunGcode { script }
}
