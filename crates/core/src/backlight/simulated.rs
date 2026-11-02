//! In-memory backlight for development and screenshots: brightness and standby without a device.

use super::{Backlight, BacklightError};

#[derive(Debug)]
pub struct Simulated {
    brightness: u8,
    on: bool,
}

impl Default for Simulated {
    fn default() -> Self {
        Self { brightness: 80, on: true }
    }
}

impl Backlight for Simulated {
    fn name(&self) -> &str {
        "simulated"
    }

    fn brightness(&self) -> Result<u8, BacklightError> {
        Ok(self.brightness)
    }

    fn set_brightness(&mut self, percent: u8) -> Result<(), BacklightError> {
        self.brightness = percent.min(100);
        Ok(())
    }

    fn set_power(&mut self, on: bool) -> Result<(), BacklightError> {
        log::debug!("simulated backlight {}", if on { "on" } else { "off" });
        self.on = on;
        Ok(())
    }
}
