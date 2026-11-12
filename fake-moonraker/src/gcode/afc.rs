//! AFC (AFC-Klipper-Add-On at 5ea84ea) lane commands: `AFC.py` L461-462 (`TOOL_UNLOAD`, `CHANGE_TOOL`), L389-390
//! (`LANE_UNLOAD`, `TOOL_LOAD`, registered per lane), `AFC_spool.py` L54-59 (`SET_COLOR`, `SET_MATERIAL`,
//! `SET_SPOOL_ID`, `SET_MAP`, registered per lane); the map and service commands `RESET_AFC_MAPPING`
//! (`AFC_spool.py` L44), `RESET_FAILURE` (`AFC_error.py` L58), `AFC_LANE_RESET` (`AFC_functions.py` L178) and
//! `AFC_UNSELECT_TOOL` (`AFC_Toolchanger.py` L51); the test hook `FAKE_AFC_ERROR`.

use super::*;

/// Parses `name` if it belongs to this group. A missing parameter is not an error here: a printer without AFC ignores
/// the command, and AFC reports what it misses.
pub(super) fn parse(name: &str, args: &[&str]) -> Result<Option<Command>, CommandError> {
    let lane = named_text(args, "LANE");
    let command = match name {
        "CHANGE_TOOL" => AfcCommand::ChangeTool { lane },
        "TOOL_LOAD" => AfcCommand::ToolLoad { lane },
        "TOOL_UNLOAD" => AfcCommand::ToolUnload { lane },
        "LANE_UNLOAD" => AfcCommand::LaneUnload { lane },
        "SET_SPOOL_ID" => AfcCommand::SetSpoolId { lane, spool_id: named_text(args, "SPOOL_ID") },
        "SET_COLOR" => AfcCommand::SetColor { lane, color: named_text(args, "COLOR") },
        "SET_MATERIAL" => AfcCommand::SetMaterial { lane, material: named_text(args, "MATERIAL") },
        "SET_MAP" => AfcCommand::SetMap { lane, map: named_text(args, "MAP") },
        "RESET_AFC_MAPPING" => AfcCommand::ResetMapping,
        "RESET_FAILURE" => AfcCommand::ResetFailure,
        "AFC_LANE_RESET" => AfcCommand::LaneReset { lane },
        "AFC_UNSELECT_TOOL" => AfcCommand::UnselectTool,
        "FAKE_AFC_ERROR" => AfcCommand::FakeError { lane },
        _ => return Ok(None),
    };
    Ok(Some(Command::Afc(command)))
}
