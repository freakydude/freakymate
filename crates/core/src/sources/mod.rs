//! Everything the printer offers, one module per origin: Klipper, Moonraker's own components and the optional
//! extensions. Each source detects itself, names what it subscribes to, reads its state and knows its commands.
//! Questions that need several sources are answered in [`crate::domain`].
//!
//! Nothing can be switched off: what is detected is used. Happy Hare is detected from the state it reports (see
//! [`PrinterStatus::happy_hare`]), Spoolman from Moonraker's components.

mod fields;
pub mod happy_hare;
pub mod klipper;
pub mod led_effect;
pub mod macro_conventions;
pub mod mainsail_config;
pub mod moonraker_server;
mod objects;
pub mod spoolman;
mod status;
mod subscription;
pub mod tool_macros;
pub mod toolchanger;

pub use objects::ObjectStore;
pub use status::PrinterStatus;

use macro_conventions::FilamentMacros;
use mainsail_config::LayerPauseMacros;
use moonraker_server::ServerComponents;
use serde_json::Value;
use subscription::Subscription;

/// What this printer has, detected when subscribing from `printer.objects.list`, `server.info` and the
/// configuration. Each optional source is `None` or empty without it.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Sources {
    pub klipper: klipper::Setup,
    pub moonraker: ServerComponents,
    /// Effect objects of the led_effect plugin.
    pub led_effects: Vec<String>,
    /// Mainsail's macros to pause at a layer.
    pub layer_pause: Option<LayerPauseMacros>,
    /// The macros `LOAD_FILAMENT` and `UNLOAD_FILAMENT`.
    pub filament_macros: Option<FilamentMacros>,
    /// Moonraker's WLED strips as light objects (`wled <strip>`), see [`moonraker_server::wled`].
    pub wled_strips: Vec<String>,
}

impl Sources {
    pub fn detect(objects: &[String], moonraker: ServerComponents) -> Self {
        let names = || objects.iter().map(String::as_str);
        let klipper = klipper::Setup::from_objects(names());
        Self {
            layer_pause: LayerPauseMacros::detect(&klipper),
            filament_macros: FilamentMacros::detect(&klipper),
            led_effects: led_effect::detect(names()),
            wled_strips: Vec::new(),
            moonraker,
            klipper,
        }
    }

    /// One line for the log: what was detected on this printer.
    pub fn summary(&self) -> String {
        let klipper = &self.klipper;
        let moonraker = &self.moonraker;
        let flags = [
            ("leveling", klipper.leveling.is_some()),
            ("probe", klipper.probe),
            ("layer pause", self.layer_pause.is_some()),
            ("filament macros", self.filament_macros.is_some()),
            ("power", moonraker.power),
            ("update manager", moonraker.update_manager),
            ("history", moonraker.history),
            ("Spoolman", moonraker.spoolman),
        ];
        let counts = [
            ("macros", klipper.macros.len()),
            ("extra heaters", klipper.extra_heaters.len()),
            ("temperature sensors", klipper.temperature_sensors.len()),
            ("filament sensors", klipper.filament_sensors.len()),
            ("fans", klipper.fans.len()),
            ("LEDs", klipper.leds.len()),
            ("LED effects", self.led_effects.len()),
            ("WLED strips", self.wled_strips.len()),
        ];
        let flags = flags.iter().filter(|(_, present)| *present).map(|(name, _)| (*name).to_owned());
        let counts = counts.iter().filter(|(_, count)| *count > 0).map(|(name, count)| format!("{count} {name}"));
        flags.chain(counts).collect::<Vec<_>>().join(", ")
    }

    /// What only the configuration tells, from `configfile.settings` of the first subscription answer.
    pub fn apply_settings(&mut self, settings: Option<&Value>) {
        self.klipper.apply_settings(settings);
    }

    /// Parameters for `printer.objects.subscribe`: every source adds its objects, only `available` ones are kept.
    pub fn subscription(&self, available: &[String]) -> Value {
        let mut subscription = Subscription::new(available);
        self.klipper.subscribe(&mut subscription);
        led_effect::subscribe(&self.led_effects, &mut subscription);
        happy_hare::subscribe(&mut subscription);
        toolchanger::subscribe(&mut subscription);
        tool_macros::subscribe(&self.klipper, &mut subscription);
        if self.layer_pause.is_some() {
            mainsail_config::subscribe(&mut subscription);
        }
        subscription.into_parameters()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn subscribes_only_available_objects() {
        let available: Vec<String> =
            ["webhooks", "extruder", "heater_generic chamber", "gcode_macro START"].map(String::from).to_vec();
        let sources = Sources::detect(&available, ServerComponents::default());

        let parameters = sources.subscription(&available);

        let objects = parameters["objects"].as_object().unwrap();
        let names: Vec<&str> = objects.keys().map(String::as_str).collect();
        assert_eq!(names, ["extruder", "heater_generic chamber", "webhooks"]);
        assert_eq!(objects["webhooks"], Value::Null);
        assert_eq!(objects["extruder"][0], "temperature");
    }

    #[test]
    fn summarizes_what_was_detected_for_the_log() {
        let available: Vec<String> =
            ["extruder", "heater_generic chamber", "fan", "heater_fan hotend", "gcode_macro START"]
                .map(String::from)
                .to_vec();
        let components = ServerComponents::from_names(["power", "spoolman"]);
        let sources = Sources::detect(&available, components);

        let summary = sources.summary();

        assert!(summary.contains("power, Spoolman"), "{summary}");
        assert!(summary.contains("1 macros"), "{summary}");
        assert!(summary.contains("1 extra heaters"), "{summary}");
        assert!(summary.contains("2 fans"), "{summary}");
        assert!(!summary.contains("history"), "{summary}");
    }

    #[test]
    fn plugins_and_happy_hare_add_their_objects() {
        let available: Vec<String> = ["led_effect rainbow", "mmu", "mmu_machine"].map(String::from).to_vec();
        let sources = Sources::detect(&available, ServerComponents::default());

        let parameters = sources.subscription(&available);

        assert_eq!(sources.led_effects, ["led_effect rainbow"]);
        assert_eq!(parameters["objects"]["led_effect rainbow"][0], "enabled");
        assert_eq!(parameters["objects"]["mmu"], Value::Null);
        assert_eq!(parameters["objects"]["mmu_machine"], Value::Null);
    }

    #[test]
    fn macro_conventions_are_detected_from_klippers_macros() {
        let available: Vec<String> =
            ["gcode_macro LOAD_FILAMENT", "gcode_macro SET_PAUSE_NEXT_LAYER"].map(String::from).to_vec();

        let sources = Sources::detect(&available, ServerComponents::default());

        assert_eq!(sources.filament_macros, Some(FilamentMacros { load: true, unload: false }));
        assert_eq!(sources.layer_pause, Some(LayerPauseMacros { next_layer: true, at_layer: false }));
    }
}
