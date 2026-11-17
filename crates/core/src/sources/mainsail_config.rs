//! Macros of mainsail-config (`client.cfg`, optional) that Sailhand calls. Macros are black boxes: they are called
//! by name with their documented parameters, and nothing is assumed about what they do inside. The one thing read
//! back is the plan of the layer pause macros: the variables `pause_next_layer` and `pause_at_layer` of their
//! `SET_PRINT_STATS_INFO`, only if they exist in the expected form (Frank, 2026-10-31).

use super::klipper::Setup;
use super::objects::ObjectStore;
use super::subscription::Subscription;
use serde_json::Value;

/// Pauses at the next layer change: `SET_PAUSE_NEXT_LAYER [ENABLE=[0|1]] [MACRO=<name>]`.
pub const PAUSE_NEXT_LAYER_MACRO: &str = "SET_PAUSE_NEXT_LAYER";
/// Pauses when a layer is reached: `SET_PAUSE_AT_LAYER [ENABLE=[0|1]] [LAYER=<number>] [MACRO=<name>]`.
pub const PAUSE_AT_LAYER_MACRO: &str = "SET_PAUSE_AT_LAYER";

/// mainsail-config's override of Klipper's `SET_PRINT_STATS_INFO`; its variables hold the plans of both macros.
const PRINT_STATS_INFO_OBJECT: &str = "gcode_macro SET_PRINT_STATS_INFO";
const PLAN_VARIABLES: [&str; 2] = ["pause_next_layer", "pause_at_layer"];

/// The pause planned with the layer macros, from the variables `pause_next_layer` and `pause_at_layer`.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct PlannedLayerPause {
    pub next_layer: bool,
    /// The layer of a planned pause at a layer.
    pub at_layer: Option<u32>,
}

impl PlannedLayerPause {
    pub fn is_planned(self) -> bool {
        self.next_layer || self.at_layer.is_some()
    }
}

pub(super) fn subscribe(subscription: &mut Subscription) {
    subscription.add_any_case(PRINT_STATS_INFO_OBJECT, Some(&PLAN_VARIABLES));
}

/// The planned pause; `None` if the variables are missing or unreadable, then the plan is not known.
pub(super) fn read_plan(store: &ObjectStore) -> Option<PlannedLayerPause> {
    let name = store.names().find(|name| name.eq_ignore_ascii_case(PRINT_STATS_INFO_OBJECT))?;
    let [next_layer, at_layer] = PLAN_VARIABLES.map(|variable| store.object(name)?.get(variable));
    let enabled = |variable: &Value| variable.get("enable").and_then(Value::as_bool);
    let (next_layer, at_layer) = (next_layer?, at_layer?);
    let layer = at_layer.get("layer").and_then(Value::as_u64).and_then(|layer| u32::try_from(layer).ok());
    Some(PlannedLayerPause {
        next_layer: enabled(next_layer)?,
        at_layer: if enabled(at_layer)? { Some(layer?) } else { None },
    })
}

/// A pause at `layer` while `current_layer` prints is one at the next layer change.
pub fn is_next_layer(layer: u32, current_layer: u32) -> bool {
    layer <= current_layer + 1
}

/// Which of the layer pause macros exist.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LayerPauseMacros {
    pub next_layer: bool,
    pub at_layer: bool,
}

impl LayerPauseMacros {
    /// `None` if neither macro exists.
    pub fn detect(klipper: &Setup) -> Option<Self> {
        let next_layer = klipper.has_macro(PAUSE_NEXT_LAYER_MACRO);
        let at_layer = klipper.has_macro(PAUSE_AT_LAYER_MACRO);
        (next_layer || at_layer).then_some(Self { next_layer, at_layer })
    }

    /// The call that plans a pause at `layer` while `current_layer` prints. The next layer pauses with
    /// `SET_PAUSE_NEXT_LAYER`, which cannot miss a layer that begins meanwhile; a later one with `SET_PAUSE_AT_LAYER`.
    /// `None` without the macro for it.
    pub fn pause_at(self, layer: u32, current_layer: u32) -> Option<String> {
        if is_next_layer(layer, current_layer) && self.next_layer {
            return Some(format!("{PAUSE_NEXT_LAYER_MACRO} ENABLE=1"));
        }
        (self.at_layer && layer > current_layer).then(|| format!("{PAUSE_AT_LAYER_MACRO} ENABLE=1 LAYER={layer}"))
    }

    /// The calls that take back a planned pause, whichever macro planned it.
    pub fn clear(self) -> Vec<String> {
        [(self.next_layer, PAUSE_NEXT_LAYER_MACRO), (self.at_layer, PAUSE_AT_LAYER_MACRO)]
            .into_iter()
            .filter(|(exists, _)| *exists)
            .map(|(_, name)| format!("{name} ENABLE=0"))
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn detect(macros: &[&str]) -> Option<LayerPauseMacros> {
        LayerPauseMacros::detect(&Setup::with_macros(macros))
    }

    #[test]
    fn each_macro_is_detected_on_its_own() {
        assert_eq!(detect(&[]), None);
        assert_eq!(detect(&["set_pause_at_layer"]), Some(LayerPauseMacros { next_layer: false, at_layer: true }));
        assert_eq!(
            detect(&[PAUSE_NEXT_LAYER_MACRO, PAUSE_AT_LAYER_MACRO]),
            Some(LayerPauseMacros { next_layer: true, at_layer: true })
        );
    }

    #[test]
    fn only_existing_macros_are_called_with_documented_parameters() {
        let at_layer = LayerPauseMacros { next_layer: false, at_layer: true };

        assert_eq!(at_layer.pause_at(11, 10).as_deref(), Some("SET_PAUSE_AT_LAYER ENABLE=1 LAYER=11"));
        assert_eq!(at_layer.pause_at(42, 10).as_deref(), Some("SET_PAUSE_AT_LAYER ENABLE=1 LAYER=42"));
        assert_eq!(at_layer.pause_at(10, 10), None);
        assert_eq!(at_layer.clear(), ["SET_PAUSE_AT_LAYER ENABLE=0"]);
    }

    #[test]
    fn the_next_layer_pauses_with_its_own_macro_even_if_the_layer_moved_on() {
        let both = LayerPauseMacros { next_layer: true, at_layer: true };
        let next_only = LayerPauseMacros { next_layer: true, at_layer: false };

        assert_eq!(both.pause_at(11, 10).as_deref(), Some("SET_PAUSE_NEXT_LAYER ENABLE=1"));
        assert_eq!(both.pause_at(11, 11).as_deref(), Some("SET_PAUSE_NEXT_LAYER ENABLE=1"));
        assert_eq!(both.pause_at(42, 10).as_deref(), Some("SET_PAUSE_AT_LAYER ENABLE=1 LAYER=42"));
        assert_eq!(next_only.pause_at(42, 10), None);
    }

    fn plan(variables: serde_json::Value) -> Option<PlannedLayerPause> {
        let mut store = ObjectStore::default();
        store.replace(&serde_json::json!({ "gcode_macro set_print_stats_info": variables }));
        read_plan(&store)
    }

    #[test]
    fn the_plan_is_read_from_mainsails_variables() {
        let variables = |next: bool, at: bool| {
            serde_json::json!({
                "pause_next_layer": { "enable": next, "call": "PAUSE" },
                "pause_at_layer": { "enable": at, "layer": 42, "call": "PAUSE" },
            })
        };

        assert_eq!(plan(variables(false, false)), Some(PlannedLayerPause::default()));
        assert_eq!(plan(variables(false, true)), Some(PlannedLayerPause { next_layer: false, at_layer: Some(42) }));
        assert_eq!(plan(variables(true, false)), Some(PlannedLayerPause { next_layer: true, at_layer: None }));
    }

    #[test]
    fn without_the_variables_the_plan_is_unknown() {
        assert_eq!(read_plan(&ObjectStore::default()), None);
        assert_eq!(plan(serde_json::json!({ "pause_next_layer": { "enable": false } })), None);
        assert_eq!(plan(serde_json::json!({ "pause_next_layer": true, "pause_at_layer": 3 })), None);
    }
}
