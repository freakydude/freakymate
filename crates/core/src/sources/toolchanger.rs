//! Tool changers that swap whole heads: viesturz's `klipper-toolchanger` and its fork `klipper-toolchanger-easy`
//! report the same objects, `toolchanger` and one `tool <name>` per head. Fields that only one of them has are not
//! read.

use super::fields::{count, flag, text};
use super::subscription::Subscription;
use super::ObjectStore;
use serde_json::Value;

const TOOLCHANGER_OBJECT: &str = "toolchanger";
/// The value of `initialize_on` that makes `select_tool` initialise an `uninitialized` plugin (viesturz
/// `toolchanger.py` at 94756df, Z.109-110 and 406-411).
const INITIALIZE_ON_FIRST_USE: &str = "first-use";
const TOOL_PREFIX: &str = "tool ";
const TOOLCHANGER_FIELDS: [&str; 5] = ["status", "tool_number", "detected_tool_number", "has_detection", "tool_names"];
const TOOL_FIELDS: [&str; 3] = ["tool_number", "extruder", "active"];

/// The tool changer and its heads.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct ToolchangerStatus {
    /// `uninitialized`, `initializing`, `ready`, `changing` or `error`.
    pub status: String,
    pub heads: Vec<ToolHead>,
    /// The `T<n>` the changer selected; `None` with no head selected.
    pub selected: Option<usize>,
    /// The head the detection pins find on the carriage; `None` without detection, with none or with several.
    pub detected: Option<usize>,
    /// `initialize_on` of the `[toolchanger]` section (`first-use`, `home` or `manual`); empty while the
    /// configuration has not arrived.
    pub initialize_on: String,
}

impl ToolchangerStatus {
    /// The changer can change heads now.
    pub fn is_ready(&self) -> bool {
        self.status == "ready"
    }

    /// A tool change goes through now: the changer is ready, or it initialises itself at the selection.
    pub fn accepts_tool_change(&self) -> bool {
        self.is_ready() || (self.status == "uninitialized" && self.initializes_on_first_use())
    }

    /// The next tool selection initialises the changer by itself: viesturz's default `first-use`. Assumed while the
    /// configuration is unknown, so no deviation is claimed without a fact.
    pub fn initializes_on_first_use(&self) -> bool {
        self.initialize_on.is_empty() || self.initialize_on == INITIALIZE_ON_FIRST_USE
    }
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

/// Commands of the plugin; the heads are the `[tool <name>]` sections.
pub mod commands {
    /// Picks up a head; viesturz and KTC-easy.
    pub fn pick_up(head: &str) -> Option<String> {
        (!head.is_empty() && !head.contains(char::is_whitespace)).then(|| format!("SELECT_TOOL TOOL={head}"))
    }

    /// Parks the mounted head; only exists with `require_tool_present` off.
    pub const DROP: &str = "UNSELECT_TOOL";

    pub fn initialize(recover: bool) -> String {
        if recover {
            "INITIALIZE_TOOLCHANGER RECOVER=1".to_owned()
        } else {
            "INITIALIZE_TOOLCHANGER".to_owned()
        }
    }
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
    let has_detection = flag(toolchanger, "has_detection");
    Some(ToolchangerStatus {
        status: text(toolchanger, "status").to_owned(),
        heads,
        selected: tool_number(toolchanger, "tool_number"),
        detected: tool_number(toolchanger, "detected_tool_number").filter(|_| has_detection),
        initialize_on: configured_initialize_on(store),
    })
}

/// `initialize_on` from `configfile.settings.toolchanger`, which lists the default too; empty if unknown.
fn configured_initialize_on(store: &ObjectStore) -> String {
    let settings = store.object("configfile").and_then(|configfile| configfile.get("settings"));
    settings
        .and_then(|settings| settings.get(TOOLCHANGER_OBJECT))
        .map_or("", |section| text(section, "initialize_on"))
        .to_owned()
}

/// A tool number; Klipper reports -1 for none.
fn tool_number(object: &Value, key: &str) -> Option<usize> {
    object.get(key).and_then(Value::as_i64).and_then(|number| usize::try_from(number).ok())
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
        assert_eq!((toolchanger.selected, toolchanger.detected), (Some(1), None));
        assert_eq!(
            toolchanger.heads,
            [
                ToolHead { name: "T0".into(), number: Some(0), extruder: Some("extruder".into()), active: false },
                ToolHead { name: "T1".into(), number: Some(1), extruder: Some("extruder1".into()), active: true },
            ]
        );
    }

    #[test]
    fn detection_is_only_read_where_the_changer_has_it() {
        let status = |has_detection| json!({ "toolchanger": { "status": "ready", "tool_number": 0, "detected_tool_number": 0, "has_detection": has_detection, "tool_names": [] } });

        assert_eq!(read_toolchanger(&store(status(true))).unwrap().detected, Some(0));
        assert_eq!(read_toolchanger(&store(status(false))).unwrap().detected, None);
    }

    #[test]
    fn no_head_selected_is_minus_one() {
        let store = store(json!({ "toolchanger": { "status": "ready", "tool_number": -1, "tool_names": [] } }));

        assert_eq!(read_toolchanger(&store).unwrap().selected, None);
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
    fn initialize_on_comes_from_the_configuration_and_defaults_to_first_use() {
        let with = |settings: Value| {
            let mut status = json!({ "toolchanger": { "status": "uninitialized", "tool_names": [] } });
            status["configfile"] = json!({ "settings": settings });
            read_toolchanger(&store(status)).unwrap()
        };

        assert!(with(json!({ "toolchanger": { "initialize_on": "first-use" } })).initializes_on_first_use());
        let home = with(json!({ "toolchanger": { "initialize_on": "home" } }));
        assert_eq!(home.initialize_on, "home");
        assert!(!home.initializes_on_first_use());
        assert!(!with(json!({ "toolchanger": { "initialize_on": "manual" } })).initializes_on_first_use());
        // Not arrived: no claim of a deviation.
        assert!(with(json!({})).initializes_on_first_use());
        // `uninitialized` takes a tool change only where the selection initializes by itself.
        assert!(with(json!({ "toolchanger": { "initialize_on": "first-use" } })).accepts_tool_change());
        assert!(!home.accepts_tool_change());
    }

    #[test]
    fn without_the_object_there_is_no_tool_changer() {
        assert_eq!(read_toolchanger(&store(json!({ "extruder": {} }))), None);
    }
}
