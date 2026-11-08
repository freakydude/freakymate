//! Lights: Klipper LEDs and WLED strips.

use crate::commands::{Command, CommandError};
use crate::domain::light;
use crate::sources::klipper::Setup;
use crate::sources::moonraker_server::wled::{self, StripAction};
use crate::sources::{PrinterStatus, Sources};

/// `level` on a light: white light with the channels a Klipper LED has (see [`Setup::white_light`]), the brightness
/// of a WLED strip.
pub(super) fn light(setup: &Setup, status: &PrinterStatus, led: &str, level: f64) -> Command {
    if let Some(strip) = wled::strip_name(led) {
        let on = status.leds.iter().any(|candidate| candidate.object == led && candidate.on);
        return Command::SetWledStrip { strip: strip.to_owned(), action: StripAction::light(on, level) };
    }
    let [red, green, blue, white] = setup.white_light(led, level);
    Command::SetLed { led: led.to_owned(), red, green, blue, white }
}

/// Switches the main light on or off.
pub(super) fn toggle(sources: &Sources, status: &PrinterStatus) -> Result<Vec<Command>, CommandError> {
    let led = light::main_light(sources).ok_or(CommandError::NoLight)?;
    let lit = light::main_led(sources, &status.leds).is_some_and(|main| main.on);
    // A WLED strip comes back with its initial preset rather than at full brightness.
    Ok(match wled::strip_name(led) {
        Some(strip) => vec![Command::SetWledStrip {
            strip: strip.to_owned(),
            action: if lit { StripAction::Off } else { StripAction::On },
        }],
        None => vec![self::light(&sources.klipper, status, led, if lit { 0.0 } else { 1.0 })],
    })
}
