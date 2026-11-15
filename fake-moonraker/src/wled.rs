//! Simulated WLED strips of Moonraker's `wled` component, following `moonraker/components/wled.py`: switching on
//! applies the initial preset, after which Moonraker does not know the brightness (`-1`), and `control` only acts
//! while the strip is on.

use serde_json::{Map, Value, json};

const INITIAL_PRESET: i64 = 1;

#[derive(Debug, Clone)]
struct Strip {
    name: &'static str,
    chain_count: u32,
    on: bool,
    preset: i64,
    brightness: i64,
}

#[derive(Debug, Clone)]
pub struct Wled {
    strips: Vec<Strip>,
}

impl Default for Wled {
    fn default() -> Self {
        Self { strips: vec![Strip { name: "enclosure", chain_count: 34, on: false, preset: -1, brightness: -1 }] }
    }
}

impl Wled {
    /// `machine.wled.strips`.
    pub fn strips(&self) -> Value {
        let strips: Map<String, Value> = self.strips.iter().map(|strip| (strip.name.to_owned(), info(strip))).collect();
        json!({ "strips": strips })
    }

    /// `machine.wled.post_strip`; a brightness of -1 means none was given.
    pub fn post_strip(&mut self, name: &str, action: &str, brightness: i64) -> Result<Value, String> {
        let strip = self
            .strips
            .iter_mut()
            .find(|strip| strip.name == name)
            .ok_or_else(|| format!("No valid strip named {name}"))?;
        let action = match action {
            "toggle" if strip.on => "off",
            "toggle" => "on",
            other => other,
        };
        match action {
            "off" => strip.on = false,
            "on" | "control" => {
                if action == "on" {
                    strip.on = true;
                    strip.preset = INITIAL_PRESET;
                    strip.brightness = -1;
                }
                if strip.on && (1..=255).contains(&brightness) {
                    strip.brightness = brightness;
                }
            }
            other => return Err(format!("Invalid requested action '{other}'")),
        }
        Ok(json!({ name: info(strip) }))
    }
}

fn info(strip: &Strip) -> Value {
    json!({
        "strip": strip.name,
        "status": if strip.on { "on" } else { "off" },
        "chain_count": strip.chain_count,
        "preset": strip.preset,
        "brightness": strip.brightness,
        "intensity": -1,
        "speed": -1,
        "error": null,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn brightness_is_unknown_after_the_preset_and_set_only_while_on() {
        let mut wled = Wled::default();

        let dimmed_off = wled.post_strip("enclosure", "control", 100).unwrap();
        assert_eq!(dimmed_off["enclosure"]["brightness"], -1);
        let on = wled.post_strip("enclosure", "on", -1).unwrap();
        assert_eq!(
            (on["enclosure"]["status"].as_str(), on["enclosure"]["brightness"].as_i64()),
            (Some("on"), Some(-1))
        );
        let dimmed = wled.post_strip("enclosure", "control", 100).unwrap();
        assert_eq!(dimmed["enclosure"]["brightness"], 100);
        assert_eq!(wled.post_strip("enclosure", "toggle", -1).unwrap()["enclosure"]["status"], "off");
        assert_eq!(wled.strips()["strips"]["enclosure"]["status"], "off");
        assert!(wled.post_strip("nope", "on", -1).is_err());
        assert!(wled.post_strip("enclosure", "blink", -1).is_err());
    }
}
