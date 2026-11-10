//! Tool changer (`klipper-toolchanger`) commands and the generic macro and variable commands around them. Behaviour and
//! error texts follow the sources: viesturz/klipper-toolchanger `klipper/extras/toolchanger.py` (`cmd_SELECT_TOOL`,
//! `cmd_UNSELECT_TOOL`), Klipper `klippy/gcode.py` (`_cmd_mux`), `extras/gcode_macro.py` (`cmd_SET_GCODE_VARIABLE`),
//! `extras/save_variables.py` (`cmd_SAVE_VARIABLE`), read 2026-11-10.

use super::*;

/// Parses `name` if it belongs to this group.
pub(super) fn parse(name: &str, args: &[&str]) -> Result<Option<Command>, CommandError> {
    let command: Result<Command, CommandError> = match name {
        "SELECT_TOOL" => select_tool(args),
        "UNSELECT_TOOL" => Ok(Command::Toolchanger(ToolchangerCommand::UnselectTool)),
        "ACTIVATE_EXTRUDER" => Ok(Command::Toolchanger(ToolchangerCommand::ActivateExtruder {
            extruder: required_text(args, "EXTRUDER")?,
        })),
        "SET_GCODE_VARIABLE" => Ok(Command::Toolchanger(ToolchangerCommand::SetGcodeVariable {
            macro_name: required_text(args, "MACRO")?,
            variable: required_text(args, "VARIABLE")?,
            value: parse_literal(&required_text(args, "VALUE")?)?,
        })),
        "SAVE_VARIABLE" => Ok(Command::Toolchanger(ToolchangerCommand::SaveVariable {
            variable: required_text(args, "VARIABLE")?,
            value: parse_literal(&required_text(args, "VALUE")?)?,
        })),
        _ => return Ok(None),
    };
    command.map(Some)
}

fn select_tool(args: &[&str]) -> Result<Command, CommandError> {
    // `TOOL` is checked before `T`, as the plugin does.
    let selector = match (named_text(args, "TOOL"), named_index(args, "T")?) {
        (Some(name), _) => ToolSelector::Name(name),
        (None, Some(number)) => ToolSelector::Number(number),
        (None, None) => return Err(CommandError::new("Select tool: Either TOOL or T needs to be specified")),
    };
    Ok(Command::Toolchanger(ToolchangerCommand::SelectTool(selector)))
}

/// A Python literal as Klipper's `ast.literal_eval` reads it, limited to what macros hold in practice.
fn parse_literal(text: &str) -> Result<serde_json::Value, CommandError> {
    use serde_json::Value;
    match text {
        "None" => return Ok(Value::Null),
        "True" => return Ok(Value::Bool(true)),
        "False" => return Ok(Value::Bool(false)),
        _ => {}
    }
    let quoted = |quote: char| text.len() >= 2 && text.starts_with(quote) && text.ends_with(quote);
    if quoted('\'') || quoted('"') {
        return Ok(Value::String(text[1..text.len() - 1].to_owned()));
    }
    match serde_json::from_str::<Value>(text) {
        Ok(number @ Value::Number(_)) => Ok(number),
        _ => Err(CommandError::new(format!("Unable to parse '{text}' as a literal"))),
    }
}
