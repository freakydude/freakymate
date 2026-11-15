//! The main light, the quick switch in the status bar, among Klipper's LEDs and Moonraker's WLED strips.

use crate::sources::Sources;
use crate::sources::klipper::{Led, instance};

/// Names that mark the light of the build chamber.
const LIGHT_NAMES: [&str; 4] = ["chamber", "case", "light", "enclosure"];

/// The light named like a chamber or case light, else the first; Klipper's LEDs come before the WLED strips.
pub fn main_light(sources: &Sources) -> Option<&str> {
    let lights: Vec<&str> = sources.klipper.leds.iter().chain(&sources.wled_strips).map(String::as_str).collect();
    let named = lights.iter().find(|light| {
        let name = instance(light).to_ascii_lowercase();
        LIGHT_NAMES.iter().any(|light_name| name.contains(light_name))
    });
    named.or(lights.first()).copied()
}

/// The state of the main light among the reported lights; `None` if there is none or it reports nothing yet.
pub fn main_led<'a>(sources: &Sources, leds: &'a [Led]) -> Option<&'a Led> {
    let main_light = main_light(sources)?;
    leds.iter().find(|led| led.object == main_light)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sources(leds: &[&str], wled_strips: &[&str]) -> Sources {
        let mut sources = Sources::default();
        sources.klipper.leds = leds.iter().map(|led| (*led).to_owned()).collect();
        sources.wled_strips = wled_strips.iter().map(|strip| (*strip).to_owned()).collect();
        sources
    }

    #[test]
    fn the_chamber_light_is_preferred_whatever_drives_it() {
        assert_eq!(main_light(&sources(&["neopixel sb", "neopixel chamber"], &[])), Some("neopixel chamber"));
        assert_eq!(main_light(&sources(&["neopixel sb"], &[])), Some("neopixel sb"));
        assert_eq!(main_light(&sources(&["neopixel sb"], &["wled chamber"])), Some("wled chamber"));
        assert_eq!(main_light(&sources(&["neopixel case"], &["wled chamber"])), Some("neopixel case"));
        assert_eq!(main_light(&sources(&[], &["wled logo"])), Some("wled logo"));
        assert_eq!(main_light(&sources(&[], &[])), None);
    }
}
