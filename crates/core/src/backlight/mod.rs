//! Backlight of the display freakymate runs on: brightness and screen standby.
//!
//! Like the network this goes to the local system, not over Moonraker, and is not always there: the BTT HDMI5 has
//! no controllable backlight, DSI and SPI displays and laptops have one under `/sys/class/backlight`. Each kind of
//! control is one implementation of [`Backlight`]; the UI only offers brightness and standby when one is writable.

mod service;
#[cfg(feature = "dev-mode")]
mod simulated;
mod standby;
mod sysfs;

pub use service::{BacklightBackend, BacklightService};
pub use standby::{Screen, Standby, StandbyTimeouts, WAKE_LOCK};
pub use sysfs::Sysfs;

use serde::{Deserialize, Serialize};
use std::time::Duration;
use thiserror::Error;

/// Lowest brightness the UI can set, so the screen never gets too dark to be used; off is the standby.
pub const MIN_BRIGHTNESS: u8 = 10;

/// One way to control a backlight. Calls are expected to be quick (sysfs writes); a slow control such as DDC/CI
/// has to run its own thread behind this trait.
pub trait Backlight: Send {
    /// Name of the device for the log, e.g. `10-0045` or `intel_backlight`.
    fn name(&self) -> &str;
    /// Current brightness in percent.
    fn brightness(&self) -> Result<u8, BacklightError>;
    fn set_brightness(&mut self, percent: u8) -> Result<(), BacklightError>;
    /// Switches the light off and on again; on restores the brightness.
    fn set_power(&mut self, on: bool) -> Result<(), BacklightError>;
}

/// Backlight part of `freakymate.json`. Everything off by default: after an update the display behaves as before.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct BacklightSettings {
    /// Device under `/sys/class/backlight`; without one the first by type (firmware, platform, raw) is used.
    pub device: Option<String>,
    /// Last brightness chosen in the UI, in percent; without one the brightness stays as the system set it.
    pub brightness: Option<u8>,
    /// Screen off after this many minutes without a touch while no print runs; without a value never.
    pub off_when_idle_minutes: Option<u32>,
    /// The same while a print runs or is paused.
    pub off_when_printing_minutes: Option<u32>,
}

impl BacklightSettings {
    pub fn timeouts(&self) -> StandbyTimeouts {
        let minutes = |value: Option<u32>| value.map(|minutes| Duration::from_secs(u64::from(minutes) * 60));
        StandbyTimeouts { idle: minutes(self.off_when_idle_minutes), printing: minutes(self.off_when_printing_minutes) }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum Availability {
    #[default]
    Checking,
    /// A writable backlight; the UI shows brightness and standby.
    Available { device: String },
    /// Nothing to control; the reason goes to the log only.
    Unavailable(String),
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct BacklightState {
    pub availability: Availability,
    /// Brightness in percent while the screen is on.
    pub brightness: u8,
    pub screen: Screen,
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum BacklightError {
    #[error("no backlight device in {0}")]
    NoDevice(String),
    #[error("backlight '{0}' not found")]
    NotFound(String),
    #[error("no permission to change backlight '{0}' (udev rule missing or user not in group 'video'?)")]
    PermissionDenied(String),
    #[error("backlight is not supported on this system")]
    Unsupported,
    #[error("backlight '{name}': {message}")]
    Device { name: String, message: String },
}

/// Brightness in percent as a raw value of a device with `max` steps; above 0 % never 0, which is off on some
/// drivers.
pub fn raw_from_percent(percent: u8, max: u32) -> u32 {
    let raw = (u64::from(percent.min(100)) * u64::from(max) + 50) / 100;
    let raw = u32::try_from(raw).unwrap_or(max);
    if percent > 0 {
        raw.max(1)
    } else {
        raw
    }
}

pub fn percent_from_raw(raw: u32, max: u32) -> u8 {
    if max == 0 {
        return 0;
    }
    let percent = (u64::from(raw.min(max)) * 100 + u64::from(max) / 2) / u64::from(max);
    u8::try_from(percent).unwrap_or(100)
}

/// Brightness the UI may set: between [`MIN_BRIGHTNESS`] and 100 %.
pub fn usable_brightness(percent: u8) -> u8 {
    percent.clamp(MIN_BRIGHTNESS, 100)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn percent_and_raw_round_trip() {
        assert_eq!(raw_from_percent(100, 255), 255);
        assert_eq!(raw_from_percent(50, 255), 128);
        assert_eq!(raw_from_percent(0, 255), 0);
        assert_eq!(percent_from_raw(128, 255), 50);
        assert_eq!(percent_from_raw(96_000, 96_000), 100);
        for percent in [10, 33, 77, 100] {
            assert_eq!(percent_from_raw(raw_from_percent(percent, 96_000), 96_000), percent);
        }
    }

    #[test]
    fn low_percent_never_switches_a_coarse_device_off() {
        assert_eq!(raw_from_percent(1, 7), 1);
        assert_eq!(raw_from_percent(10, 7), 1);
    }

    #[test]
    fn raw_above_max_counts_as_full() {
        assert_eq!(percent_from_raw(300, 255), 100);
        assert_eq!(percent_from_raw(5, 0), 0);
    }

    #[test]
    fn brightness_stays_usable() {
        assert_eq!(usable_brightness(0), MIN_BRIGHTNESS);
        assert_eq!(usable_brightness(55), 55);
        assert_eq!(usable_brightness(200), 100);
    }

    #[test]
    fn settings_default_to_never_off_and_untouched_brightness() {
        let settings: BacklightSettings = serde_json::from_str("{}").unwrap();
        assert_eq!(settings, BacklightSettings::default());
        assert_eq!(settings.timeouts(), StandbyTimeouts { idle: None, printing: None });
        let settings: BacklightSettings = serde_json::from_str(r#"{"off_when_idle_minutes": 5}"#).unwrap();
        assert_eq!(settings.timeouts().idle, Some(Duration::from_secs(300)));
    }
}
