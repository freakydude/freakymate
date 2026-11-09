//! Happy Hare (`MMU_*`) commands and tool changes.

use super::*;

/// Parses `name` if it belongs to this group.
pub(super) fn parse(name: &str, args: &[&str]) -> Result<Option<Command>, CommandError> {
    if let Some(tool) = tool_number(name) {
        return Ok(Some(Command::Mmu(MmuCommand::ChangeTool { tool })));
    }

    let command: Result<Command, CommandError> = match name {
        "MMU_HOME" => Ok(Command::Mmu(MmuCommand::Home)),
        "MMU_SELECT" if named_index(args, "BYPASS")?.is_some_and(|value| value != 0) => {
            Ok(Command::Mmu(MmuCommand::SelectBypass))
        }
        "MMU_SELECT" => Ok(Command::Mmu(MmuCommand::Select { gate: required_index(args, "GATE")? })),
        "MMU_SELECT_BYPASS" => Ok(Command::Mmu(MmuCommand::SelectBypass)),
        // Without `RESET` or `TOOL` and `GATE` Happy Hare only shows the map.
        "MMU_TTG_MAP" if named_index(args, "RESET")?.is_some_and(|value| value != 0) => {
            Ok(Command::Mmu(MmuCommand::ResetToolMap))
        }
        "MMU_TTG_MAP" => match (named_index(args, "TOOL")?, named_index(args, "GATE")?) {
            (Some(tool), Some(gate)) => Ok(Command::Mmu(MmuCommand::MapTool { tool, gate })),
            _ => Ok(Command::Ignored),
        },
        "MMU_ENDLESS_SPOOL" => Ok(Command::Mmu(MmuCommand::EndlessSpool {
            enabled: named_index(args, "ENABLE")?.map(|value| value != 0),
            groups: named_text(args, "GROUPS").map(|groups| parse_groups(&groups)).transpose()?,
        })),
        "MMU_CHANGE_TOOL" => Ok(Command::Mmu(MmuCommand::ChangeTool { tool: required_index(args, "TOOL")? })),
        "MMU_LOAD" => Ok(Command::Mmu(MmuCommand::Load)),
        "MMU_UNLOAD" => Ok(Command::Mmu(MmuCommand::Unload)),
        "MMU_GATE_MAP" => Ok(Command::Mmu(MmuCommand::GateMap {
            gate: required_index(args, "GATE")?,
            spool_id: named_value(args, "SPOOLID")?.map(|id| id as i64),
            material: named_text(args, "MATERIAL").map(|material| material.to_ascii_uppercase()),
            color: named_text(args, "COLOR").map(|color| color.to_ascii_lowercase()),
            temperature: named_value(args, "TEMP")?.map(|degrees| degrees.round() as i32),
        })),
        "FAKE_MMU_FILAMENT" => Ok(Command::Mmu(MmuCommand::FakeFilament {
            position: named_value(args, "POS")?
                .ok_or_else(|| CommandError::new("Missing parameter POS"))?
                .clamp(-1.0, 10.0) as i32,
            direction: named_value(args, "DIRECTION")?.unwrap_or(0.0).signum() as i32,
        })),
        "FAKE_MMU_TANGLE" => Ok(Command::Mmu(MmuCommand::FakeTangle { boosting: required_index(args, "BOOST")? != 0 })),
        "MMU_MOTORS_OFF" => Ok(Command::Mmu(MmuCommand::MotorsOff)),
        "MMU_CHECK_GATE" => Ok(Command::Mmu(MmuCommand::CheckGate {
            gate: named_index(args, "GATE")?,
            all: named_index(args, "ALL")?.is_some_and(|value| value != 0),
        })),
        "MMU_PRELOAD" => Ok(Command::Mmu(MmuCommand::Preload { gate: named_index(args, "GATE")? })),
        "MMU_SERVO" => Ok(Command::Mmu(MmuCommand::Servo {
            position: named_text(args, "POS").as_deref().map(ServoPosition::parse).transpose()?,
        })),
        "MMU_EJECT" => Ok(Command::Mmu(MmuCommand::Eject {
            gate: named_index(args, "GATE")?,
            force: named_index(args, "FORCE")?.is_some_and(|value| value != 0),
        })),
        _ => return Ok(None),
    };
    command.map(Some)
}

/// `GROUPS=0,1,0`: non-negative whole numbers, as Happy Hare demands (`mmu_endless_spool.py`).
fn parse_groups(groups: &str) -> Result<Vec<u32>, CommandError> {
    groups
        .split(',')
        .map(|group| group.trim().parse().map_err(|_| CommandError::new(format!("Invalid GROUPS value: {groups}"))))
        .collect()
}

fn tool_number(name: &str) -> Option<usize> {
    name.strip_prefix('T')
        .filter(|digits| !digits.is_empty() && digits.bytes().all(|byte| byte.is_ascii_digit()))
        .and_then(|digits| digits.parse().ok())
}
