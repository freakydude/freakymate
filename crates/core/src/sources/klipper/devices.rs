//! Fans and LEDs: what they report and which of them a person can switch.
//!
//! Field names follow Klipper's Status Reference (`speed`, `rpm`, `color_data`).

use super::status::label;
use crate::sources::objects::ObjectStore;
use serde_json::Value;

/// LED drivers of Klipper; all of them report `color_data` and take `SET_LED`.
const LED_KINDS: [&str; 5] = ["led", "neopixel", "dotstar", "pca9533", "pca9632"];
/// Klipper's default of `kick_start_time` in seconds.
const KLIPPER_KICK_START_TIME: f64 = 0.1;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum FanKind {
    /// `[fan]`, the part cooling fan of `M106`.
    Part,
    /// `[fan_generic]`, e.g. an exhaust or a filter; set with `SET_FAN_SPEED`.
    Generic,
    /// `[heater_fan]`, runs with its heater.
    Heater,
    /// `[controller_fan]`, runs with the steppers.
    Controller,
}

impl FanKind {
    pub fn of(object: &str) -> Option<Self> {
        match object.split_once(' ').map_or(object, |(kind, _)| kind) {
            "fan" if object == "fan" => Some(Self::Part),
            "fan_generic" => Some(Self::Generic),
            "heater_fan" => Some(Self::Heater),
            "controller_fan" => Some(Self::Controller),
            _ => None,
        }
    }

    /// Klipper drives heater and controller fans itself; they are only shown.
    pub fn is_controllable(self) -> bool {
        matches!(self, Self::Part | Self::Generic)
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Fan {
    pub object: String,
    pub label: String,
    pub kind: FanKind,
    /// 0 to 1.
    pub speed: f64,
    /// Measured speed; only with a `tachometer_pin`.
    pub rpm: Option<f64>,
    /// Klipper keeps the fan off below this speed (`off_below`, 0 to 1), although `speed` reports the request.
    pub off_below: f64,
    /// Seconds Klipper runs the fan at full power when switching it on or raising it a lot (`kick_start_time`).
    pub kick_start_time: f64,
    /// The heaters a `heater_fan` runs with (`heater` in the configuration, Klipper's default is `extruder`); empty
    /// for every other kind. Names the heaters only, not where the fan sits.
    pub heaters: Vec<String>,
    /// At a speed where its tachometer has measured before, it has reported 0 rpm for a while (set by `FanWatch`).
    pub stalled: bool,
}

impl Fan {
    /// Klipper drives the fan: a speed is asked for and it is not below `off_below`.
    pub fn should_turn(&self) -> bool {
        self.speed > 0.0 && self.speed >= self.off_below
    }
}

/// A light the person can dim: a Klipper LED or, from Moonraker, a WLED strip.
#[derive(Debug, Clone, PartialEq)]
pub struct Led {
    pub object: String,
    pub label: String,
    pub on: bool,
    /// Brightness from 0 to 1, for a Klipper LED its brightest channel; `None` if unknown (a WLED strip after a
    /// preset).
    pub level: Option<f64>,
}

pub fn is_led(object: &str) -> bool {
    object.split_once(' ').is_some_and(|(kind, _)| LED_KINDS.contains(&kind))
}

/// Which chips of an LED chain have a white channel.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum WhiteChannel {
    #[default]
    None,
    /// Only some, e.g. a Stealthburner chain with RGB logo and RGBW nozzle LEDs.
    Some,
    All,
}

impl WhiteChannel {
    /// Channel levels `[red, green, blue, white]` for white light at `level` (0 to 1, 0 is off): the white channel
    /// where every chip has one, the same level on every colour where none has, and both for a mixed chain, so its
    /// RGB chips light up too.
    pub fn white_light(self, level: f64) -> [f64; 4] {
        let level = level.clamp(0.0, 1.0);
        let (colour, white) = match self {
            Self::All => (0.0, level),
            Self::Some => (level, level),
            Self::None => (level, 0.0),
        };
        [colour, colour, colour, white]
    }
}

/// From the configuration: `color_order` with `W` (neopixel, dotstar; one entry per chip in `configfile.settings`)
/// or a `white_pin` (PWM LEDs).
pub fn white_channel(section: Option<&Value>) -> WhiteChannel {
    let Some(section) = section else { return WhiteChannel::None };
    if section.get("white_pin").is_some_and(|pin| !pin.is_null()) {
        return WhiteChannel::All;
    }
    let orders: Vec<&str> = match section.get("color_order") {
        Some(Value::String(order)) => vec![order.as_str()],
        Some(Value::Array(orders)) => orders.iter().filter_map(Value::as_str).collect(),
        _ => Vec::new(),
    };
    match orders.iter().filter(|order| order.contains('W')).count() {
        0 => WhiteChannel::None,
        white if white == orders.len() => WhiteChannel::All,
        _ => WhiteChannel::Some,
    }
}

/// The instance name of an object, e.g. `chamber` for `neopixel chamber`.
pub fn instance(object: &str) -> &str {
    object.split_once(' ').map_or(object, |(_, name)| name)
}

/// A configuration section whose name starts with `_` is a helper that frontends do not show, by the convention of
/// Mainsail: `gcode_macro _HELPER`, `fan_generic _exhaust`, `output_pin _status`.
pub fn is_hidden(object: &str) -> bool {
    object.split_once(' ').is_some_and(|(_, name)| name.starts_with('_'))
}

/// Klipper's default of `heater` in `[heater_fan]`.
const DEFAULT_FAN_HEATER: &str = "extruder";

/// `heater` of a `[heater_fan]` section: one name or several separated by commas (Klipper splits it at the comma).
/// Without the section (not subscribed yet) nothing is known, so nothing is claimed.
fn heater_names(section: Option<&Value>) -> Vec<String> {
    let Some(section) = section else { return Vec::new() };
    let heater = section.get("heater").and_then(Value::as_str).unwrap_or(DEFAULT_FAN_HEATER);
    heater.split(',').map(str::trim).filter(|name| !name.is_empty()).map(str::to_owned).collect()
}

/// Part fan first, then generic, heater and controller fans, each in name order.
pub(super) fn read_fans(store: &ObjectStore) -> Vec<Fan> {
    let settings = store.object("configfile").and_then(|configfile| configfile.get("settings"));
    let mut fans: Vec<Fan> = store
        .names()
        .filter_map(|name| Some((name, FanKind::of(name)?)))
        .filter_map(|(name, kind)| {
            let value = store.object(name)?;
            let section = settings.and_then(|settings| settings.get(name.to_lowercase()));
            let setting = |key: &str| section.and_then(|section| section.get(key)).and_then(Value::as_f64);
            Some(Fan {
                object: name.to_owned(),
                label: if kind == FanKind::Part { "Part fan".to_owned() } else { label(name) },
                kind,
                speed: value.get("speed").and_then(Value::as_f64).unwrap_or_default().clamp(0.0, 1.0),
                rpm: value.get("rpm").and_then(Value::as_f64),
                off_below: setting("off_below").unwrap_or_default(),
                kick_start_time: setting("kick_start_time").unwrap_or(KLIPPER_KICK_START_TIME),
                heaters: if kind == FanKind::Heater { heater_names(section) } else { Vec::new() },
                stalled: false,
            })
        })
        .collect();
    fans.sort_by(|a, b| (a.kind, &a.object).cmp(&(b.kind, &b.object)));
    fans
}

pub(super) fn read_leds(store: &ObjectStore) -> Vec<Led> {
    store
        .names()
        .filter(|name| is_led(name))
        .filter_map(|name| {
            let chips = store.object(name)?.get("color_data")?.as_array()?;
            let level =
                chips.iter().filter_map(Value::as_array).flatten().filter_map(Value::as_f64).fold(0.0_f64, f64::max);
            let level = level.clamp(0.0, 1.0);
            Some(Led { object: name.to_owned(), label: label(name), on: level > 0.0, level: Some(level) })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn store(status: Value) -> ObjectStore {
        let mut store = ObjectStore::default();
        store.replace(&status);
        store
    }

    #[test]
    fn fans_come_part_first_and_know_who_controls_them() {
        let fans = read_fans(&store(json!({
            "controller_fan board": { "speed": 1.0, "rpm": 3100.0 },
            "fan_generic nevermore": { "speed": 0.4, "rpm": null },
            "fan": { "speed": 0.5 },
            "heater_fan hotend": { "speed": 1.0 },
            "fan_generic chamber_light": { "speed": 2.0 }
        })));

        let names: Vec<&str> = fans.iter().map(|fan| fan.object.as_str()).collect();
        assert_eq!(
            names,
            ["fan", "fan_generic chamber_light", "fan_generic nevermore", "heater_fan hotend", "controller_fan board"]
        );
        assert_eq!(fans[0].label, "Part fan");
        assert_eq!(fans[1].speed, 1.0, "speed is clamped");
        assert_eq!(fans[2].rpm, None);
        assert_eq!(fans[4].rpm, Some(3100.0));
        assert!(fans[0].kind.is_controllable() && fans[2].kind.is_controllable());
        assert!(!fans[3].kind.is_controllable() && !fans[4].kind.is_controllable());
    }

    #[test]
    fn a_heater_fan_names_its_heaters_from_the_configuration() {
        let fans = read_fans(&store(json!({
            "heater_fan hotend": { "speed": 1.0 },
            "heater_fan default": { "speed": 0.0 },
            "heater_fan both": { "speed": 0.0 },
            "heater_fan unknown": { "speed": 0.0 },
            "fan": { "speed": 0.5 },
            "configfile": { "settings": {
                "heater_fan hotend": { "heater": "extruder1" },
                "heater_fan default": {},
                "heater_fan both": { "heater": "extruder, extruder1" }
            } }
        })));

        let heaters = |object: &str| fans.iter().find(|fan| fan.object == object).unwrap().heaters.clone();
        assert_eq!(heaters("heater_fan hotend"), ["extruder1"]);
        assert_eq!(heaters("heater_fan default"), ["extruder"], "Klipper's default");
        assert_eq!(heaters("heater_fan both"), ["extruder", "extruder1"]);
        assert!(heaters("heater_fan unknown").is_empty(), "no section, no claim");
        assert!(heaters("fan").is_empty());
    }

    #[test]
    fn a_fan_below_off_below_is_not_expected_to_turn() {
        let fans = read_fans(&store(json!({
            "fan_generic exhaust": { "speed": 0.1, "rpm": 0.0 },
            "fan": { "speed": 0.1, "rpm": 0.0 },
            "configfile": { "settings": { "fan_generic exhaust": { "off_below": 0.15, "kick_start_time": 0.5 } } }
        })));

        assert_eq!(fans[1].off_below, 0.15);
        assert_eq!((fans[0].kick_start_time, fans[1].kick_start_time), (0.1, 0.5));
        assert!(fans[0].should_turn());
        assert!(!fans[1].should_turn());
    }

    #[test]
    fn an_led_is_as_bright_as_its_brightest_channel() {
        let leds = read_leds(&store(json!({
            "neopixel chamber": { "color_data": [[0.0, 0.0, 0.0, 0.75], [0.1, 0.0, 0.0, 0.0]] },
            "led status": { "color_data": [[0.0, 0.0, 0.0, 0.0]] },
            "neopixel broken": { "color_data": null }
        })));

        assert_eq!(leds.len(), 2);
        assert_eq!((leds[0].label.as_str(), leds[0].on, leds[0].level), ("Status", false, Some(0.0)));
        assert_eq!((leds[1].label.as_str(), leds[1].on, leds[1].level), ("Chamber", true, Some(0.75)));
    }

    #[test]
    fn white_comes_from_the_colour_order_of_each_chip_or_a_white_pin() {
        let white = |section: Value| white_channel(Some(&section));
        assert_eq!(white(json!({ "color_order": ["GRBW"] })), WhiteChannel::All);
        assert_eq!(white(json!({ "color_order": "RGBW" })), WhiteChannel::All);
        assert_eq!(white(json!({ "color_order": ["GRB", "GRB", "GRBW", "GRBW"] })), WhiteChannel::Some);
        assert_eq!(white(json!({ "color_order": ["GRB"] })), WhiteChannel::None);
        assert_eq!(white(json!({ "white_pin": "PA1" })), WhiteChannel::All);
        assert_eq!(white(json!({ "white_pin": null })), WhiteChannel::None);
        assert_eq!(white_channel(None), WhiteChannel::None);
    }
}
