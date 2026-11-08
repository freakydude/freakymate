//! Moonraker's `wled` component: LED strips with WLED firmware that Moonraker switches over HTTP or serial.
//!
//! Field names follow `get_strip_info` in `moonraker/components/wled.py`. Moonraker does not report changes, and its
//! state is only what it set itself: a change in WLED's own app is not seen, and the brightness is unknown (`-1`)
//! after a preset.

use crate::moonraker::Call;
use crate::sources::klipper::{label, Led};
use serde_json::{json, Map, Value};

/// Strips are named like Klipper's LED objects, `wled <strip>`, so they stand among them.
const OBJECT_PREFIX: &str = "wled ";
/// WLED's brightness runs from 1 to 255; Moonraker ignores 0.
const MAX_BRIGHTNESS: f64 = 255.0;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WledStrip {
    /// The name in `[wled <name>]`, as `post_strip` takes it.
    pub name: String,
    pub on: bool,
    /// 1 to 255; `None` until Moonraker set it, e.g. after a preset.
    pub brightness: Option<u8>,
}

impl WledStrip {
    pub fn object(&self) -> String {
        format!("{OBJECT_PREFIX}{}", self.name)
    }

    /// The strip as one of the lights beside Klipper's LEDs.
    pub fn led(&self) -> Led {
        let object = self.object();
        Led {
            label: label(&object),
            on: self.on,
            level: self.brightness.map(|brightness| f64::from(brightness) / MAX_BRIGHTNESS),
            object,
        }
    }
}

/// The strip of a light object (`wled <strip>`); `None` for Klipper's LEDs.
pub fn strip_name(object: &str) -> Option<&str> {
    object.strip_prefix(OBJECT_PREFIX)
}

/// `machine.wled.strips`.
pub fn strips_call() -> Call {
    Call::new("machine.wled.strips")
}

/// The strips of `machine.wled.strips`.
pub fn parse_strip_list(result: &Value) -> Vec<WledStrip> {
    result.get("strips").and_then(Value::as_object).map(parse_strips).unwrap_or_default()
}

/// The strips of an answer keyed by strip name, as `machine.wled.strips` and `machine.wled.post_strip` give it.
pub fn parse_strips(strips: &Map<String, Value>) -> Vec<WledStrip> {
    strips
        .iter()
        .map(|(name, info)| WledStrip {
            name: name.clone(),
            on: info.get("status").and_then(Value::as_str) == Some("on"),
            brightness: info.get("brightness").and_then(Value::as_u64).and_then(|value| u8::try_from(value).ok()),
        })
        .collect()
}

/// What a strip is told, see `_process_request` in `wled.py`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum StripAction {
    Off,
    /// On with the initial preset or colours from `moonraker.conf`.
    On,
    /// On, then at this brightness (0 to 1).
    OnAt(f64),
    /// The brightness (0 to 1) of a strip that is on; Moonraker ignores it while the strip is off.
    Dim(f64),
}

impl StripAction {
    /// The brightness `level` (0 to 1, 0 is off) for a strip that is `on` or not.
    pub fn light(on: bool, level: f64) -> Self {
        match (level > 0.0, on) {
            (false, _) => Self::Off,
            (true, true) => Self::Dim(level),
            (true, false) => Self::OnAt(level),
        }
    }
}

pub fn post_strip(strip: &str, action: StripAction) -> Call {
    let params = match action {
        StripAction::Off => json!({ "strip": strip, "action": "off" }),
        StripAction::On => json!({ "strip": strip, "action": "on" }),
        StripAction::OnAt(level) => json!({ "strip": strip, "action": "on", "brightness": brightness(level) }),
        StripAction::Dim(level) => json!({ "strip": strip, "action": "control", "brightness": brightness(level) }),
    };
    Call::with("machine.wled.post_strip", params)
}

fn brightness(level: f64) -> u8 {
    (level * MAX_BRIGHTNESS).round().clamp(1.0, MAX_BRIGHTNESS) as u8
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_strips_with_unknown_brightness() {
        let strips = parse_strip_list(&json!({ "strips": {
            "chamber": { "strip": "chamber", "status": "on", "chain_count": 34, "preset": 1, "brightness": -1,
                         "intensity": -1, "speed": -1, "error": null },
            "logo": { "strip": "logo", "status": "off", "chain_count": 8, "preset": -1, "brightness": 128,
                      "intensity": -1, "speed": -1, "error": null }
        }}));

        assert_eq!(strips[0], WledStrip { name: "chamber".into(), on: true, brightness: None });
        assert_eq!(strips[1], WledStrip { name: "logo".into(), on: false, brightness: Some(128) });
        let led = strips[1].led();
        assert_eq!((led.object.as_str(), led.label.as_str()), ("wled logo", "Logo"));
        assert!((led.level.unwrap() - 128.0 / 255.0).abs() < 1e-9);
        assert_eq!(strip_name(&led.object), Some("logo"));
        assert_eq!(strip_name("neopixel logo"), None);
    }

    #[test]
    fn brightness_is_controlled_only_while_on() {
        let params = |action| post_strip("chamber", action).params.unwrap();

        assert_eq!(StripAction::light(true, 0.0), StripAction::Off);
        assert_eq!(StripAction::light(true, 0.5), StripAction::Dim(0.5));
        assert_eq!(StripAction::light(false, 0.5), StripAction::OnAt(0.5));
        assert_eq!(params(StripAction::Off), json!({ "strip": "chamber", "action": "off" }));
        assert_eq!(params(StripAction::On), json!({ "strip": "chamber", "action": "on" }));
        assert_eq!(params(StripAction::OnAt(1.0)), json!({ "strip": "chamber", "action": "on", "brightness": 255 }));
        assert_eq!(
            params(StripAction::Dim(0.001)),
            json!({ "strip": "chamber", "action": "control", "brightness": 1 })
        );
    }
}
