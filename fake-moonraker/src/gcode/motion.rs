//! Motion, positioning, speed and extrusion settings.

use super::*;

/// Parses `name` if it belongs to this group.
pub(super) fn parse(name: &str, args: &[&str]) -> Result<Option<Command>, CommandError> {
    let command: Result<Command, CommandError> = match name {
        "G28" => Ok(Command::Home(parse_axes(args))),
        "G4" => Ok(Command::Dwell(std::time::Duration::from_millis(
            letter_value(args, 'P')?.unwrap_or(0.0).max(0.0) as u64
        ))),
        "M84" | "M18" => Ok(Command::DisableSteppers),
        "G90" => Ok(Command::AbsolutePositioning),
        "G91" => Ok(Command::RelativePositioning),
        "G0" | "G1" => Ok(Command::Move(parse_move(args)?)),
        "SET_GCODE_OFFSET" => {
            Ok(Command::SetGcodeOffset { z: named_value(args, "Z")?, z_adjust: named_value(args, "Z_ADJUST")? })
        }
        "M82" => Ok(Command::AbsoluteExtrusion),
        "M83" => Ok(Command::RelativeExtrusion),
        "M220" => Ok(Command::SetSpeedFactor(letter_value(args, 'S')?.unwrap_or(100.0) / 100.0)),
        "M221" => Ok(Command::SetExtrudeFactor(letter_value(args, 'S')?.unwrap_or(100.0) / 100.0)),
        "SET_PRESSURE_ADVANCE" => Ok(Command::SetPressureAdvance {
            advance: named_value(args, "ADVANCE")?,
            smooth_time: named_value(args, "SMOOTH_TIME")?,
        }),
        "SET_VELOCITY_LIMIT" => Ok(Command::SetVelocityLimit(VelocityLimit {
            velocity: named_value(args, "VELOCITY")?,
            accel: named_value(args, "ACCEL")?,
            square_corner_velocity: named_value(args, "SQUARE_CORNER_VELOCITY")?,
            minimum_cruise_ratio: named_value(args, "MINIMUM_CRUISE_RATIO")?,
        })),
        "SET_RETRACTION" => Ok(Command::SetRetraction(RetractionChange {
            retract_length: named_value(args, "RETRACT_LENGTH")?,
            retract_speed: named_value(args, "RETRACT_SPEED")?,
            unretract_extra_length: named_value(args, "UNRETRACT_EXTRA_LENGTH")?,
            unretract_speed: named_value(args, "UNRETRACT_SPEED")?,
        })),
        "QUAD_GANTRY_LEVEL" => Ok(Command::QuadGantryLevel),
        _ => return Ok(None),
    };
    command.map(Some)
}

fn parse_move(args: &[&str]) -> Result<Move, CommandError> {
    Ok(Move {
        x: letter_value(args, 'X')?,
        y: letter_value(args, 'Y')?,
        z: letter_value(args, 'Z')?,
        e: letter_value(args, 'E')?,
    })
}

fn parse_axes(args: &[&str]) -> Vec<Axis> {
    args.iter()
        .filter_map(|arg| match arg.to_ascii_uppercase().as_str() {
            "X" => Some(Axis::X),
            "Y" => Some(Axis::Y),
            "Z" => Some(Axis::Z),
            _ => None,
        })
        .collect()
}
