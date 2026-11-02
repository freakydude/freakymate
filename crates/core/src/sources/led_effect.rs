//! The optional led_effect plugin: one Klipper object per effect (`led_effect <name>`).
//!
//! Field names follow `klipper-led_effect/src/led_effect.py` (`enabled`).

use super::klipper::{instance, is_hidden, label};
use super::objects::ObjectStore;
use super::subscription::Subscription;
use serde_json::Value;

const OBJECT_PREFIX: &str = "led_effect ";

#[derive(Debug, Clone, PartialEq)]
pub struct LedEffect {
    /// The effect's name, as `SET_LED_EFFECT EFFECT=` takes it.
    pub name: String,
    pub label: String,
    pub enabled: bool,
}

pub fn is_effect(object: &str) -> bool {
    object.starts_with(OBJECT_PREFIX)
}

/// The effect objects of the plugin, without helpers starting with `_`; empty without it.
pub(super) fn detect<'a>(objects: impl IntoIterator<Item = &'a str>) -> Vec<String> {
    objects.into_iter().filter(|name| is_effect(name) && !is_hidden(name)).map(str::to_owned).collect()
}

pub(super) fn subscribe(effects: &[String], subscription: &mut Subscription) {
    for effect in effects {
        subscription.add(effect, Some(&["enabled"]));
    }
}

pub(super) fn read_effects(store: &ObjectStore) -> Vec<LedEffect> {
    store
        .names()
        .filter(|name| is_effect(name))
        .filter_map(|name| {
            let value = store.object(name)?;
            Some(LedEffect {
                name: instance(name).to_owned(),
                label: label(name),
                enabled: value.get("enabled").and_then(Value::as_bool).unwrap_or_default(),
            })
        })
        .collect()
}

/// Starts or stops an effect.
pub fn set_effect(effect: &str, on: bool) -> String {
    if on {
        format!("SET_LED_EFFECT EFFECT={effect}")
    } else {
        format!("SET_LED_EFFECT EFFECT={effect} STOP=1")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn effects_report_whether_they_run() {
        let mut store = ObjectStore::default();
        store.replace(&json!({ "led_effect rainbow": { "enabled": true } }));

        let effects = read_effects(&store);

        assert_eq!(effects, [LedEffect { name: "rainbow".into(), label: "Rainbow".into(), enabled: true }]);
    }

    #[test]
    fn helper_effects_are_left_out() {
        let effects = detect(["led_effect rainbow", "led_effect _boot", "neopixel _status"]);

        assert_eq!(effects, ["led_effect rainbow"]);
    }

    #[test]
    fn effects_start_and_stop() {
        assert_eq!(set_effect("rainbow", true), "SET_LED_EFFECT EFFECT=rainbow");
        assert_eq!(set_effect("rainbow", false), "SET_LED_EFFECT EFFECT=rainbow STOP=1");
    }
}
