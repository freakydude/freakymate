//! Heaters, fans and LEDs.

use super::*;

/// Parses `name` if it belongs to this group.
pub(super) fn parse(name: &str, args: &[&str]) -> Result<Option<Command>, CommandError> {
    let command: Result<Command, CommandError> = match name {
        "M104" => Ok(Command::SetExtruderTarget {
            tool: letter_value(args, 'T')?.map(|tool| tool as usize),
            target: letter_value(args, 'S')?.unwrap_or(0.0),
        }),
        "M140" => Ok(Command::SetBedTarget(letter_value(args, 'S')?.unwrap_or(0.0))),
        "M106" => Ok(Command::SetFanSpeed(fan_speed(args)?)),
        "M107" => Ok(Command::SetFanSpeed(0.0)),
        "SET_FAN_SPEED" => Ok(Command::Device(DeviceCommand::SetFanSpeed {
            fan: required_text(args, "FAN")?,
            speed: named_value(args, "SPEED")?.ok_or_else(|| CommandError::new("SET_FAN_SPEED must specify SPEED"))?,
        })),
        "SET_LED" => {
            let channel = |name| named_value(args, name).map(|value| value.unwrap_or(0.0).clamp(0.0, 1.0));
            Ok(Command::Device(DeviceCommand::SetLed {
                led: required_text(args, "LED")?,
                color: [channel("RED")?, channel("GREEN")?, channel("BLUE")?, channel("WHITE")?],
            }))
        }
        "SET_LED_EFFECT" => Ok(Command::Device(DeviceCommand::SetLedEffect {
            effect: required_text(args, "EFFECT")?,
            stop: named_index(args, "STOP")?.is_some_and(|stop| stop >= 1),
        })),
        "STOP_LED_EFFECTS" => Ok(Command::Device(DeviceCommand::StopLedEffects)),
        "FAKE_FAN_STALL" => Ok(Command::Device(DeviceCommand::FakeFanStall {
            fan: required_text(args, "FAN")?,
            stall: named_index(args, "STALL")?.is_none_or(|stall| stall != 0),
        })),
        "SET_HEATER_TEMPERATURE" => Ok(Command::SetHeaterTemperature {
            heater: named_text(args, "HEATER").ok_or_else(|| CommandError::new("Missing parameter HEATER"))?,
            target: named_value(args, "TARGET")?.unwrap_or(0.0),
        }),
        _ => return Ok(None),
    };
    command.map(Some)
}

fn fan_speed(args: &[&str]) -> Result<f64, CommandError> {
    let raw = letter_value(args, 'S')?.unwrap_or(255.0);
    Ok(raw.clamp(0.0, 255.0) / 255.0)
}
