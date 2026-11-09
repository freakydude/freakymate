//! The extruder beside its heater: pressure advance, whether it may move, and its limits from the configuration.

use crate::sources::fields::{flag, number};
use serde_json::Value;

/// Lengths in mm the Filament page offers for extruding by hand.
pub const EXTRUDE_LENGTHS_MM: [f64; 5] = [5.0, 10.0, 25.0, 50.0, 100.0];
/// Klipper's default of `max_extrude_only_distance` in mm.
const DEFAULT_MAX_EXTRUDE_ONLY_MM: f64 = 50.0;

#[derive(Debug, Clone, PartialEq, Default)]
pub struct ExtruderState {
    pub pressure_advance: f64,
    pub smooth_time: f64,
    pub can_extrude: bool,
    /// Configured `filament_diameter` in mm; `None` until the configuration is known.
    pub filament_diameter: Option<f64>,
    /// Configured `max_extrude_only_distance` in mm: Klipper refuses longer moves of the extruder alone.
    pub max_extrude_only_mm: Option<f64>,
}

impl ExtruderState {
    /// The lengths of [`EXTRUDE_LENGTHS_MM`] Klipper accepts (Klipper's default limit while the configuration is
    /// unknown); the limit itself when it is shorter than all of them.
    pub fn extrude_lengths(&self) -> Vec<f64> {
        let limit = self.max_extrude_only_mm.unwrap_or(DEFAULT_MAX_EXTRUDE_ONLY_MM);
        let lengths: Vec<f64> = EXTRUDE_LENGTHS_MM.into_iter().filter(|length| *length <= limit).collect();
        if lengths.is_empty() {
            vec![limit]
        } else {
            lengths
        }
    }
}

pub(super) fn read_extruder(extruder: &Value, configuration: Option<&Value>) -> ExtruderState {
    ExtruderState {
        pressure_advance: number(extruder, "pressure_advance"),
        smooth_time: number(extruder, "smooth_time"),
        can_extrude: flag(extruder, "can_extrude"),
        filament_diameter: configuration.and_then(|section| section.get("filament_diameter")).and_then(Value::as_f64),
        max_extrude_only_mm: configuration
            .and_then(|section| section.get("max_extrude_only_distance"))
            .and_then(Value::as_f64),
    }
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
    fn only_lengths_klipper_accepts_are_offered_for_extruding() {
        let configured = |limit: f64| ExtruderState { max_extrude_only_mm: Some(limit), ..ExtruderState::default() };

        assert_eq!(ExtruderState::default().extrude_lengths(), [5.0, 10.0, 25.0, 50.0]);
        assert_eq!(configured(100.0).extrude_lengths(), EXTRUDE_LENGTHS_MM);
        assert_eq!(configured(30.0).extrude_lengths(), [5.0, 10.0, 25.0]);
        assert_eq!(configured(2.0).extrude_lengths(), [2.0]);
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
}
