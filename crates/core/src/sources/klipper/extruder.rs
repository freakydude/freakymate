//! The extruder beside its heater: pressure advance, whether it may move, and its limits from the configuration.

use crate::sources::fields::{flag, number};
use serde_json::Value;
use std::collections::BTreeMap;

/// The extruder Klipper starts with, and the one a printer without a reported active extruder has.
const MAIN_EXTRUDER: &str = "extruder";

/// Klipper's default of `max_extrude_only_distance` in mm.
const DEFAULT_MAX_EXTRUDE_ONLY_MM: f64 = 50.0;

#[derive(Debug, Clone, PartialEq, Default)]
pub struct ExtruderState {
    pub pressure_advance: f64,
    pub smooth_time: f64,
    pub can_extrude: bool,
    /// Configured `filament_diameter` in mm; `None` until the configuration is known.
    pub filament_diameter: Option<f64>,
    /// Configured `filament_diameter` in mm of every extruder section (`extruder`, `extruder1`, ...).
    pub filament_diameters: BTreeMap<String, f64>,
    /// The extruder the toolhead moves (`toolhead.extruder`); `None` until reported.
    pub active: Option<String>,
    /// Configured `max_extrude_only_distance` in mm: Klipper refuses longer moves of the extruder alone.
    pub max_extrude_only_mm: Option<f64>,
}

impl ExtruderState {
    /// The name of the extruder in use: the reported one, the main one while none is reported.
    pub fn active_name(&self) -> &str {
        self.active.as_deref().unwrap_or(MAIN_EXTRUDER)
    }

    /// Configured `filament_diameter` in mm of the active extruder.
    ///
    /// Klipper's `motion_report.live_extruder_velocity` follows the active extruder: `motion_report.py` `get_status`
    /// takes the velocity of `toolhead.get_extra_axes()` entry 3 (`if ea_index == 3: evelocity = velocity`), and
    /// `toolhead.py` `set_extruder` puts the active extruder at `extra_axes[0]`, which is axis index 3.
    pub fn active_filament_diameter(&self) -> Option<f64> {
        self.filament_diameters.get(self.active_name()).copied()
    }

    /// Klipper's limit for extruding alone in mm (its default while the configuration is unknown).
    pub fn extrude_limit(&self) -> f64 {
        self.max_extrude_only_mm.unwrap_or(DEFAULT_MAX_EXTRUDE_ONLY_MM)
    }

    /// The configured lengths, all of them: those above [`Self::extrude_limit`] are for the UI to show as disabled.
    /// When none fits the limit, the limit itself comes first as an extra length.
    pub fn extrude_lengths(&self, configured: &[f64]) -> Vec<f64> {
        let limit = self.extrude_limit();
        if configured.iter().any(|length| *length <= limit) {
            return configured.to_vec();
        }
        std::iter::once(limit).chain(configured.iter().copied()).collect()
    }
}

/// `toolhead` names the active extruder, `settings` is `configfile.settings` (all configured sections).
pub(super) fn read_extruder(extruder: &Value, toolhead: &Value, settings: Option<&Value>) -> ExtruderState {
    let configuration = settings.and_then(|settings| settings.get(MAIN_EXTRUDER));
    let filament_diameters = settings
        .and_then(Value::as_object)
        .into_iter()
        .flatten()
        .filter(|(name, _)| is_extruder_section(name))
        .filter_map(|(name, section)| Some((name.clone(), section.get("filament_diameter")?.as_f64()?)))
        .collect();
    ExtruderState {
        pressure_advance: number(extruder, "pressure_advance"),
        smooth_time: number(extruder, "smooth_time"),
        can_extrude: flag(extruder, "can_extrude"),
        filament_diameter: configuration.and_then(|section| section.get("filament_diameter")).and_then(Value::as_f64),
        filament_diameters,
        active: toolhead.get("extruder").and_then(Value::as_str).map(str::to_owned),
        max_extrude_only_mm: configuration
            .and_then(|section| section.get("max_extrude_only_distance"))
            .and_then(Value::as_f64),
    }
}

/// `extruder` or `extruder<n>`.
fn is_extruder_section(name: &str) -> bool {
    name.strip_prefix(MAIN_EXTRUDER).is_some_and(|rest| rest.bytes().all(|byte| byte.is_ascii_digit()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sources::{ObjectStore, PrinterStatus};
    use serde_json::json;

    fn status(objects: Value) -> PrinterStatus {
        let mut store = ObjectStore::default();
        store.replace(&objects);
        PrinterStatus::from_store(&store)
    }

    #[test]
    fn all_configured_lengths_are_offered_and_the_limit_joins_when_none_fits() {
        let configured = |limit: f64| ExtruderState { max_extrude_only_mm: Some(limit), ..ExtruderState::default() };
        let lengths = [5.0, 10.0, 25.0, 50.0, 100.0];

        assert_eq!(ExtruderState::default().extrude_limit(), 50.0);
        assert_eq!(ExtruderState::default().extrude_lengths(&lengths), lengths);
        assert_eq!(configured(30.0).extrude_lengths(&lengths), lengths);
        assert_eq!(configured(5.0).extrude_lengths(&lengths), lengths);
        assert_eq!(configured(2.0).extrude_lengths(&lengths), [2.0, 5.0, 10.0, 25.0, 50.0, 100.0]);
        let status = status(json!({
            "extruder": { "temperature": 20.0 },
            "configfile": { "settings": { "extruder": { "max_extrude_only_distance": 100.0 } } }
        }));
        assert_eq!(status.extruder.max_extrude_only_mm, Some(100.0));
    }

    #[test]
    fn reads_the_filament_diameter_from_the_configuration() {
        let configured = status(json!({
            "extruder": { "temperature": 20.0 },
            "configfile": { "settings": { "extruder": { "filament_diameter": 2.85 } } }
        }));

        assert_eq!(configured.extruder.filament_diameter, Some(2.85));
        assert_eq!(status(json!({})).extruder.filament_diameter, None);
    }

    #[test]
    fn reads_the_diameter_of_the_active_extruder() {
        let status = status(json!({
            "toolhead": { "extruder": "extruder1" },
            "configfile": { "settings": {
                "extruder": { "filament_diameter": 1.75 },
                "extruder1": { "filament_diameter": 2.85 },
                "extruder_stepper x": { "filament_diameter": 9.0 }
            } }
        }));

        assert_eq!(status.extruder.active.as_deref(), Some("extruder1"));
        assert_eq!(status.extruder.filament_diameters.len(), 2);
        assert_eq!(status.extruder.active_filament_diameter(), Some(2.85));
        assert_eq!(status.extruder.filament_diameter, Some(1.75));
    }

    #[test]
    fn the_main_extruder_counts_while_none_is_reported() {
        let status = status(json!({ "configfile": { "settings": { "extruder": { "filament_diameter": 1.75 } } } }));

        assert_eq!(status.extruder.active_filament_diameter(), Some(1.75));
    }
}
