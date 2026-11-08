//! Workaround: Happy Hare loses some gate map changes on their way to Moonraker.
//!
//! Klipper sends a field only when it differs from the value it sent last (`webhooks.py`, `_do_query`), and keeps
//! that value by reference. Happy Hare returns its gate map lists from `get_status` and, when a gate becomes empty
//! through `set_gate_status` or `validate_gate_status` (`mmu_gate_maps.py`, up to at least v4.1.0-17), copies only
//! `gate_status` before `clear_gate_attributes` clears the other lists in place. Klipper then compares a list with
//! itself and never sends the cleared name, material, colour or spool; `gate_color_rgb` is rebuilt as a new list and
//! arrives. Seen on a Voron after an unload that left the filament behind the entry sensor: the gate turned black
//! but kept its material, name and spool.
//!
//! So when `gate_status` or `gate_color_rgb` changes, the fields Happy Hare clears in place are queried once more.
//! Remove this module and its one call in `service.rs` once Happy Hare copies every list before changing it.

use serde_json::{json, Value};

/// Fields that trigger the query; at least one of them arrives whenever Happy Hare empties a gate.
const TRIGGERS: [&str; 2] = ["gate_status", "gate_color_rgb"];

/// The gate map fields `clear_gate_attributes` resets in place (`_gate_map_vars` without `gate_status`).
const CLEARED_IN_PLACE: [&str; 10] = [
    "gate_filament_name",
    "gate_material",
    "gate_vendor",
    "gate_td",
    "gate_td1_color",
    "gate_color",
    "gate_temperature",
    "gate_spool_id",
    "gate_speed_override",
    "gate_spool_rfid",
];

/// Parameters for `printer.objects.query` when the changes of a `notify_status_update` may hide gate map fields
/// that Happy Hare changed in place; `None` otherwise.
pub fn query_after(changes: &Value) -> Option<Value> {
    let mmu = changes.get("mmu")?;
    TRIGGERS.iter().any(|field| mmu.get(field).is_some()).then(|| json!({ "objects": { "mmu": CLEARED_IN_PLACE } }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sources::happy_hare::read_mmu;
    use crate::sources::ObjectStore;

    #[test]
    fn a_change_of_the_gate_status_or_colour_asks_again() {
        assert!(query_after(&json!({ "mmu": { "gate_status": [0] } })).is_some());
        assert!(query_after(&json!({ "mmu": { "gate_color_rgb": [[0.0, 0.0, 0.0]] } })).is_some());
        assert_eq!(query_after(&json!({ "mmu": { "filament_pos": 0 } })), None);
        assert_eq!(query_after(&json!({ "extruder": { "gate_status": [0] } })), None);
    }

    /// The unload on the Voron: the entry sensor empties gate 0, only `gate_status` and `gate_color_rgb` arrive.
    #[test]
    fn the_query_clears_what_the_update_left_behind() {
        let mut store = ObjectStore::default();
        store.replace(&json!({ "mmu": {
            "num_gates": 1, "gate_status": [1], "gate_filament_name": ["Red ABS+"], "gate_material": ["ABS+"],
            "gate_color": ["e31f2b"], "gate_color_rgb": [[0.89, 0.12, 0.17]], "gate_spool_id": [5],
            "gate_temperature": [250],
        }}));
        let changes = json!({ "mmu": { "gate_status": [0], "gate_color_rgb": [[0.0, 0.0, 0.0]] } });
        store.merge(&changes);
        let stale = &read_mmu(&store).unwrap().gates[0];
        assert_eq!((stale.color, stale.material.as_str(), stale.spool_id), (Some([0, 0, 0, 255]), "ABS+", Some(5)));

        let query = query_after(&changes).unwrap();
        assert_eq!(query["objects"]["mmu"].as_array().unwrap().len(), CLEARED_IN_PLACE.len());
        store.merge(&json!({ "mmu": {
            "gate_filament_name": [""], "gate_material": [""], "gate_color": [""], "gate_spool_id": [-1],
            "gate_temperature": [200],
        }}));
        let gate = &read_mmu(&store).unwrap().gates[0];
        assert_eq!((gate.color, gate.material.as_str(), gate.name.as_str(), gate.spool_id), (None, "", "", None));
    }
}
