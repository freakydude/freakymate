//! Raw state of the subscribed Klipper objects, updated from `notify_status_update`.

use serde_json::{Map, Value};

#[derive(Debug, Clone, Default, PartialEq)]
pub struct ObjectStore {
    objects: Map<String, Value>,
    /// Klipper's `eventtime` of the subscription's snapshot; updates sent before it are part of the snapshot.
    snapshot_eventtime: Option<f64>,
}

impl ObjectStore {
    /// Replaces the entire state.
    pub fn replace(&mut self, status: &Value) {
        self.objects = status.as_object().cloned().unwrap_or_default();
        self.snapshot_eventtime = None;
    }

    /// Replaces the entire state with the response to `printer.objects.subscribe`, which Klipper took at its
    /// `eventtime`.
    pub fn replace_subscribed(&mut self, response: &Value) {
        self.replace(response.get("status").unwrap_or(&Value::Null));
        self.snapshot_eventtime = response.get("eventtime").and_then(Value::as_f64);
    }

    /// Applies the changes of a `notify_status_update` Klipper sent at `eventtime`; false, and nothing changes, when
    /// they are older than the subscription's snapshot. Such an update may still be queued when the snapshot arrives,
    /// and it would undo a change the snapshot already holds: while a client subscribes again, Moonraker stops its
    /// updates until Klipper has answered (`klippy_connection.py`, `_request_subscripton`) and Klipper leaves the
    /// client out of that query's round (`webhooks.py`, `_handle_query`), so the change never comes as an update.
    /// Seen with Happy Hare's `action`, which kept "Selecting" from the bootup until the next gate change.
    pub fn merge_update(&mut self, changes: &Value, eventtime: Option<f64>) -> bool {
        let older = matches!((eventtime, self.snapshot_eventtime), (Some(sent), Some(snapshot)) if sent < snapshot);
        if !older {
            self.merge(changes);
        }
        !older
    }

    /// Applies changed fields; fields not mentioned are kept. Klipper sends a changed field whole (`webhooks.py`,
    /// `_do_query`), so a field's value is replaced, not merged: a key missing from a nested object was removed.
    pub fn merge(&mut self, changes: &Value) {
        let Some(changes) = changes.as_object() else { return };
        for (name, change) in changes {
            match self.objects.get_mut(name) {
                Some(current) => merge_fields(current, change),
                None => {
                    self.objects.insert(name.clone(), change.clone());
                }
            }
        }
    }

    pub fn clear(&mut self) {
        self.objects.clear();
        self.snapshot_eventtime = None;
    }

    pub fn object(&self, name: &str) -> Option<&Value> {
        self.objects.get(name)
    }

    pub fn names(&self) -> impl Iterator<Item = &str> {
        self.objects.keys().map(String::as_str)
    }
}

fn merge_fields(current: &mut Value, change: &Value) {
    match (current, change) {
        (Value::Object(current), Value::Object(change)) => {
            for (field, value) in change {
                current.insert(field.clone(), value.clone());
            }
        }
        (current, change) => *current = change.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn merge_keeps_unchanged_fields_and_replaces_changed_ones() {
        let mut store = ObjectStore::default();
        store.replace(&json!({
            "extruder": { "temperature": 20.0, "target": 0.0 },
            "print_stats": { "info": { "current_layer": 1, "total_layer": 10 } }
        }));

        store.merge(&json!({
            "extruder": { "temperature": 25.5 },
            "print_stats": { "info": { "current_layer": 2, "total_layer": 10 } },
            "fan": { "speed": 0.5 }
        }));

        assert_eq!(store.object("extruder"), Some(&json!({ "temperature": 25.5, "target": 0.0 })));
        assert_eq!(store.object("print_stats"), Some(&json!({ "info": { "current_layer": 2, "total_layer": 10 } })));
        assert_eq!(store.object("fan"), Some(&json!({ "speed": 0.5 })));
    }

    #[test]
    fn keys_missing_from_a_changed_field_are_gone() {
        let mut store = ObjectStore::default();
        store.replace(&json!({ "bed_mesh": { "profile_name": "a", "profiles": { "a": {}, "b": {} } } }));

        store.merge(&json!({ "bed_mesh": { "profiles": { "a": {} } } }));

        assert_eq!(store.object("bed_mesh"), Some(&json!({ "profile_name": "a", "profiles": { "a": {} } })));
    }

    #[test]
    fn arrays_are_replaced_as_a_whole() {
        let mut store = ObjectStore::default();
        store.replace(&json!({ "toolhead": { "position": [1.0, 2.0, 3.0, 4.0] } }));

        store.merge(&json!({ "toolhead": { "position": [5.0, 6.0, 7.0, 8.0] } }));

        assert_eq!(store.object("toolhead"), Some(&json!({ "position": [5.0, 6.0, 7.0, 8.0] })));
    }

    /// The Voron's bootup: "Selecting" was queued while Sailhand subscribed again, the snapshot already says "Idle".
    #[test]
    fn an_update_older_than_the_snapshot_is_ignored() {
        let mut store = ObjectStore::default();
        store.replace_subscribed(&json!({ "eventtime": 30.5, "status": { "mmu": { "action": "Idle" } } }));

        let applied = store.merge_update(&json!({ "mmu": { "action": "Selecting" } }), Some(30.25));

        assert!(!applied);
        assert_eq!(store.object("mmu"), Some(&json!({ "action": "Idle" })));
    }

    #[test]
    fn updates_from_the_snapshot_on_are_applied() {
        let mut store = ObjectStore::default();
        store.replace_subscribed(&json!({ "eventtime": 30.5, "status": { "mmu": { "action": "Selecting" } } }));

        assert!(store.merge_update(&json!({ "mmu": { "action": "Selecting" } }), Some(30.5)));
        assert!(store.merge_update(&json!({ "mmu": { "action": "Idle" } }), Some(30.75)));

        assert_eq!(store.object("mmu"), Some(&json!({ "action": "Idle" })));
    }

    #[test]
    fn without_eventtimes_every_update_is_applied() {
        let mut store = ObjectStore::default();
        store.replace_subscribed(&json!({ "status": { "mmu": { "action": "Idle" } } }));
        assert!(store.merge_update(&json!({ "mmu": { "action": "Selecting" } }), Some(1.0)));

        store.replace_subscribed(&json!({ "eventtime": 30.5, "status": { "mmu": { "action": "Idle" } } }));
        assert!(store.merge_update(&json!({ "mmu": { "action": "Selecting" } }), None));
    }

    #[test]
    fn a_cleared_store_forgets_the_snapshot_time() {
        let mut store = ObjectStore::default();
        store.replace_subscribed(&json!({ "eventtime": 30.5, "status": {} }));
        store.clear();

        assert!(store.merge_update(&json!({ "mmu": { "action": "Idle" } }), Some(1.0)));
    }
}
