//! Subscription of a connection to printer objects (`printer.objects.subscribe`).

use serde_json::{Map, Value};
use std::collections::BTreeMap;

#[derive(Debug, Default)]
pub struct Subscription {
    wanted: BTreeMap<String, Option<Vec<String>>>,
    last_sent: Map<String, Value>,
}

impl Subscription {
    /// Drops all subscribed objects, as Moonraker does when the Klipper connection closes.
    pub fn clear(&mut self) {
        self.wanted.clear();
        self.last_sent.clear();
    }

    pub fn replace_with(&mut self, objects: &Value) -> Result<(), String> {
        let requested = objects.as_object().ok_or("'objects' must be an object")?;
        let mut wanted = BTreeMap::new();
        for (name, fields) in requested {
            wanted.insert(name.clone(), parse_fields(fields)?);
        }
        self.wanted = wanted;
        self.last_sent.clear();
        Ok(())
    }

    /// Full state of all subscribed fields; serves as the baseline for later changes.
    pub fn snapshot(&mut self, status: &Value) -> Value {
        self.last_sent = self.select(status);
        Value::Object(self.last_sent.clone())
    }

    /// Only the fields changed since the last call, `None` if nothing is new.
    pub fn changes(&mut self, status: &Value) -> Option<Value> {
        let current = self.select(status);
        let mut changed = Map::new();
        for (name, value) in &current {
            let Value::Object(fields) = value else { continue };
            let previous = self.last_sent.get(name).and_then(Value::as_object);
            let differing: Map<String, Value> = fields
                .iter()
                .filter(|(field, value)| previous.and_then(|old| old.get(field.as_str())) != Some(*value))
                .map(|(field, value)| (field.clone(), value.clone()))
                .collect();
            if !differing.is_empty() {
                changed.insert(name.clone(), Value::Object(differing));
            }
        }
        self.last_sent = current;
        (!changed.is_empty()).then_some(Value::Object(changed))
    }

    fn select(&self, status: &Value) -> Map<String, Value> {
        let mut selected = Map::new();
        for (name, fields) in &self.wanted {
            let Some(Value::Object(object)) = status.get(name) else { continue };
            let chosen = match fields {
                None => object.clone(),
                Some(names) => object
                    .iter()
                    .filter(|(field, _)| names.contains(field))
                    .map(|(field, value)| (field.clone(), value.clone()))
                    .collect(),
            };
            selected.insert(name.clone(), Value::Object(chosen));
        }
        selected
    }
}

fn parse_fields(fields: &Value) -> Result<Option<Vec<String>>, String> {
    match fields {
        Value::Null => Ok(None),
        Value::Array(items) => items
            .iter()
            .map(|item| item.as_str().map(str::to_owned).ok_or_else(|| "field names must be strings".to_owned()))
            .collect::<Result<Vec<_>, _>>()
            .map(Some),
        _ => Err("fields must be null or an array of names".to_owned()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn subscribed(objects: Value) -> Subscription {
        let mut subscription = Subscription::default();
        subscription.replace_with(&objects).unwrap();
        subscription
    }

    #[test]
    fn snapshot_contains_only_requested_fields() {
        let mut subscription = subscribed(json!({ "extruder": ["target"], "fan": null }));
        let status = json!({
            "extruder": { "temperature": 24.0, "target": 0.0 },
            "fan": { "speed": 0.0, "rpm": null },
            "toolhead": { "homed_axes": "" },
        });

        let snapshot = subscription.snapshot(&status);

        assert_eq!(snapshot, json!({ "extruder": { "target": 0.0 }, "fan": { "speed": 0.0, "rpm": null } }));
    }

    #[test]
    fn changes_report_only_modified_fields() {
        let mut subscription = subscribed(json!({ "extruder": null }));
        subscription.snapshot(&json!({ "extruder": { "temperature": 24.0, "target": 0.0 } }));

        let changes = subscription.changes(&json!({ "extruder": { "temperature": 30.0, "target": 0.0 } }));

        assert_eq!(changes, Some(json!({ "extruder": { "temperature": 30.0 } })));
    }

    #[test]
    fn unchanged_status_yields_no_changes() {
        let mut subscription = subscribed(json!({ "extruder": null }));
        let status = json!({ "extruder": { "temperature": 24.0 } });
        subscription.snapshot(&status);

        assert_eq!(subscription.changes(&status), None);
    }

    #[test]
    fn unknown_objects_are_ignored() {
        let mut subscription = subscribed(json!({ "mmu": null }));

        assert_eq!(subscription.snapshot(&json!({ "extruder": {} })), json!({}));
    }

    #[test]
    fn replacing_the_subscription_resets_the_baseline() {
        let mut subscription = subscribed(json!({ "extruder": null }));
        let status = json!({ "extruder": { "temperature": 24.0 }, "fan": { "speed": 0.0 } });
        subscription.snapshot(&status);

        subscription.replace_with(&json!({ "fan": null })).unwrap();

        assert_eq!(subscription.changes(&status), Some(json!({ "fan": { "speed": 0.0 } })));
    }

    #[test]
    fn rejects_malformed_subscriptions() {
        let mut subscription = Subscription::default();

        assert!(subscription.replace_with(&json!([])).is_err());
        assert!(subscription.replace_with(&json!({ "extruder": "target" })).is_err());
        assert!(subscription.replace_with(&json!({ "extruder": [1] })).is_err());
    }
}
