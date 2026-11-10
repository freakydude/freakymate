//! Tool changers that swap whole heads: viesturz's `klipper-toolchanger` and its fork `klipper-toolchanger-easy`
//! report the same objects, `toolchanger` and one `tool <name>` per head. Fields that only one of them has are not
//! read.

use super::fields::{count, flag, text};
use super::subscription::Subscription;
use super::ObjectStore;
use serde_json::Value;

const TOOLCHANGER_OBJECT: &str = "toolchanger";
const TOOL_PREFIX: &str = "tool ";
const TOOLCHANGER_FIELDS: [&str; 3] = ["status", "tool_number", "tool_names"];
const TOOL_FIELDS: [&str; 3] = ["tool_number", "extruder", "active"];

/// The tool changer and its heads.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct ToolchangerStatus {
    /// `uninitialized`, `initializing`, `ready`, `changing` or `error`.
    pub status: String,
    pub heads: Vec<ToolHead>,
}

/// One head of the tool changer.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ToolHead {
    /// The name of the `[tool <name>]` section, such as `T0`.
    pub name: String,
    /// The `T<n>` the head answers to; `None` while unassigned.
    pub number: Option<usize>,
    /// The Klipper extruder of the head; `None` for a head without one.
    pub extruder: Option<String>,
    /// The head is the one in use.
    pub active: bool,
}

pub(super) fn subscribe(subscription: &mut Subscription) {
    subscription.add(TOOLCHANGER_OBJECT, Some(&TOOLCHANGER_FIELDS));
    subscription.add_each(TOOL_PREFIX, Some(&TOOL_FIELDS));
}

/// `None` without a tool changer; the heads in the order of `tool_names` (sorted by number).
pub(super) fn read_toolchanger(store: &ObjectStore) -> Option<ToolchangerStatus> {
    let toolchanger = store.object(TOOLCHANGER_OBJECT)?;
    let names = toolchanger.get("tool_names").and_then(Value::as_array).map(Vec::as_slice).unwrap_or_default();
    let heads = names.iter().filter_map(Value::as_str).filter_map(|object| read_head(store, object)).collect();
    Some(ToolchangerStatus { status: text(toolchanger, "status").to_owned(), heads })
}

fn read_head(store: &ObjectStore, object: &str) -> Option<ToolHead> {
    let tool = store.object(object)?;
    let extruder = text(tool, "extruder");
    Some(ToolHead {
        name: object.strip_prefix(TOOL_PREFIX).unwrap_or(object).to_owned(),
        number: count(tool, "tool_number"),
        extruder: (!extruder.is_empty()).then(|| extruder.to_owned()),
        active: flag(tool, "active"),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn store(status: Value) -> ObjectStore {
        let mut store = ObjectStore::default();
        store.replace(&status);
        store
    }

    #[test]
    fn reads_the_heads_in_the_order_of_the_tool_changer() {
        let store = store(json!({
            "toolchanger": { "status": "ready", "tool_number": 1, "tool_names": ["tool T0", "tool T1"] },
            "tool T1": { "tool_number": 1, "extruder": "extruder1", "active": true },
            "tool T0": { "tool_number": 0, "extruder": "extruder", "active": false },
        }));

        let toolchanger = read_toolchanger(&store).unwrap();

        assert_eq!(toolchanger.status, "ready");
        assert_eq!(
            toolchanger.heads,
            [
                ToolHead { name: "T0".into(), number: Some(0), extruder: Some("extruder".into()), active: false },
                ToolHead { name: "T1".into(), number: Some(1), extruder: Some("extruder1".into()), active: true },
            ]
        );
    }

    #[test]
    fn an_unassigned_head_without_extruder_has_neither() {
        let store = store(json!({
            "toolchanger": { "status": "ready", "tool_names": ["tool probe"] },
            "tool probe": { "tool_number": -1, "extruder": null, "active": false },
        }));

        let head = &read_toolchanger(&store).unwrap().heads[0];

        assert_eq!((head.number, head.extruder.as_deref()), (None, None));
    }

    #[test]
    fn without_the_object_there_is_no_tool_changer() {
        assert_eq!(read_toolchanger(&store(json!({ "extruder": {} }))), None);
    }
}
