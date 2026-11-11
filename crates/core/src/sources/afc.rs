//! AFC ("Automated Filament Control" of Armored Turtle, the software of the Box Turtle and others): its lanes, the
//! lane in use and, on a tool changer of AFC (`[AFC_Toolchanger]`), its heads.
//!
//! Objects and fields follow AFC-Klipper-Add-On at commit 5ea84ea: `AFC.get_status` (`AFC.py` L2537), a lane's
//! (`AFC_lane.py` L2183; Box Turtle lanes are `AFC_stepper <name>`, other units and the heads of a tool changer
//! `AFC_lane <name>`), `AFC_extruder.get_status` (`AFC_extruder.py` L1065). The Klipper extruder of an
//! `[AFC_extruder <name>]` is only in its configuration (`extruder_name`, the section's own name by default,
//! `AFC_extruder.py` L222); a lane names its `AFC_extruder`, not the Klipper extruder.

use super::fields::{count, flag, text};
use super::subscription::Subscription;
use super::ObjectStore;
use crate::color::{self, Rgba};
use serde_json::Value;

const AFC_OBJECT: &str = "AFC";
const LANE_PREFIXES: [&str; 2] = ["AFC_stepper ", "AFC_lane "];
const EXTRUDER_PREFIX: &str = "AFC_extruder ";
/// Configuration sections are lowercased in `configfile.settings`.
const EXTRUDER_SECTION: &str = "afc_extruder ";
const AFC_FIELDS: [&str; 4] = ["current_load", "current_state", "error_state", "lanes"];
const LANE_FIELDS: [&str; 9] = ["unit", "extruder", "map", "spool_id", "color", "material", "load", "prep", "status"];
const EXTRUDER_FIELDS: [&str; 2] = ["on_shuttle", "lane_loaded"];

/// AFC with its lanes; `None` without the object `AFC`.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct AfcStatus {
    /// `current_state`: `Idle`, `Loading`, `ToolSwap`, … (`AFC.py` L57).
    pub state: String,
    pub error: bool,
    /// The lane loaded into the toolhead (`current_load`).
    pub current_load: Option<String>,
    /// The lanes in the order of `AFC.lanes`.
    pub lanes: Vec<AfcLane>,
    /// The heads of AFC's own tool changer, in the order their lanes come; empty without one.
    pub heads: Vec<AfcHead>,
    /// AFC has more than one unit, so lanes are named with theirs.
    pub several_units: bool,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct AfcLane {
    /// The lane's name (`lane1`), which its commands take as `LANE=`.
    pub name: String,
    /// The unit that carries the lane (`Turtle_1`).
    pub unit: String,
    /// The Klipper extruder the lane feeds.
    pub extruder: String,
    /// The `T<n>` mapped to the lane (`map`).
    pub tool: Option<usize>,
    pub spool_id: Option<u32>,
    pub color: Option<Rgba>,
    pub material: String,
    /// Filament sits in the lane (`prep` or `load`).
    pub filled: bool,
    /// `status`: `None`, `Loaded`, `Tooled`, … (`AFC_lane.py` L65).
    pub status: String,
}

/// A head of AFC's tool changer: an `AFC_extruder` with `toolchanger_unit`.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct AfcHead {
    /// The name of the `AFC_extruder` (`e0`).
    pub name: String,
    pub extruder: String,
    /// The head sits on the carriage (`on_shuttle`).
    pub mounted: bool,
}

pub(super) fn subscribe(subscription: &mut Subscription) {
    subscription.add(AFC_OBJECT, Some(&AFC_FIELDS));
    for prefix in LANE_PREFIXES {
        subscription.add_each(prefix, Some(&LANE_FIELDS));
    }
    subscription.add_each(EXTRUDER_PREFIX, Some(&EXTRUDER_FIELDS));
}

pub(super) fn read_afc(store: &ObjectStore) -> Option<AfcStatus> {
    let afc = store.object(AFC_OBJECT)?;
    let settings = store.object("configfile").and_then(|configfile| configfile.get("settings")).unwrap_or(&Value::Null);
    let names = afc.get("lanes").and_then(Value::as_array).map(Vec::as_slice).unwrap_or_default();
    let lanes: Vec<AfcLane> =
        names.iter().filter_map(Value::as_str).filter_map(|name| read_lane(store, settings, name)).collect();
    let heads = read_heads(store, settings, &lanes);
    let several_units = lanes.iter().any(|lane| lane.unit != lanes[0].unit);
    Some(AfcStatus {
        state: text(afc, "current_state").to_owned(),
        error: flag(afc, "error_state"),
        current_load: Some(text(afc, "current_load")).filter(|lane| !lane.is_empty()).map(str::to_owned),
        lanes,
        heads,
        several_units,
    })
}

/// A lane reports nothing until AFC has finished starting up (`AFC_lane.py` L2183); such a lane is skipped.
fn read_lane(store: &ObjectStore, settings: &Value, name: &str) -> Option<AfcLane> {
    let lane = LANE_PREFIXES.iter().find_map(|prefix| store.object(&format!("{prefix}{name}")))?;
    let afc_extruder = lane.get("extruder").and_then(Value::as_str)?;
    Some(AfcLane {
        name: name.to_owned(),
        unit: text(lane, "unit").to_owned(),
        extruder: klipper_extruder(settings, afc_extruder),
        tool: text(lane, "map").strip_prefix('T').and_then(|number| number.parse().ok()),
        spool_id: count(lane, "spool_id"),
        color: color::parse_hex(text(lane, "color")),
        material: text(lane, "material").to_owned(),
        filled: flag(lane, "prep") || flag(lane, "load"),
        status: text(lane, "status").to_owned(),
    })
}

/// The Klipper extruder of an `AFC_extruder`: `extruder_name` in its section, else its own name.
fn klipper_extruder(settings: &Value, afc_extruder: &str) -> String {
    let section = settings.get(format!("{EXTRUDER_SECTION}{}", afc_extruder.to_lowercase()));
    let configured = section.map(|section| text(section, "extruder_name")).filter(|name| !name.is_empty());
    configured.unwrap_or(afc_extruder).to_owned()
}

/// The `AFC_extruder`s that belong to a tool changer unit (`toolchanger_unit` in their section, `AFC_extruder.py`
/// L270), in the order of the lanes that feed them.
fn read_heads(store: &ObjectStore, settings: &Value, lanes: &[AfcLane]) -> Vec<AfcHead> {
    let mut heads: Vec<AfcHead> = Vec::new();
    let names = store.names().filter_map(|object| object.strip_prefix(EXTRUDER_PREFIX));
    let swappable = |name: &&str| {
        let section = settings.get(format!("{EXTRUDER_SECTION}{}", name.to_lowercase()));
        section.is_some_and(|section| !text(section, "toolchanger_unit").is_empty())
    };
    for name in names.filter(swappable) {
        let object = store.object(&format!("{EXTRUDER_PREFIX}{name}")).unwrap_or(&Value::Null);
        heads.push(AfcHead {
            name: name.to_owned(),
            extruder: klipper_extruder(settings, name),
            mounted: flag(object, "on_shuttle"),
        });
    }
    let position = |head: &AfcHead| lanes.iter().position(|lane| lane.extruder == head.extruder).unwrap_or(usize::MAX);
    heads.sort_by_key(position);
    heads
}

/// `SET_SPOOL_ID` puts a spool into a lane; without `SPOOL_ID` it takes it out (`AFC_spool.py` L325). AFC refuses a
/// spool that another lane holds and only logs it.
pub fn assign_spool(lane: &str, spool_id: Option<u32>) -> String {
    match spool_id {
        Some(id) => format!("SET_SPOOL_ID LANE={lane} SPOOL_ID={id}"),
        None => format!("SET_SPOOL_ID LANE={lane}"),
    }
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

    /// A Box Turtle after `templates/AFC_Turtle_1.cfg`: four `AFC_stepper` lanes on `AFC_extruder extruder`, which has
    /// no `extruder_name`; lane4 has not reported yet.
    #[test]
    fn reads_the_lanes_of_a_box_turtle_with_the_one_in_use() {
        let lane = |map: &str, spool: Value, color: &str| {
            json!({ "unit": "Turtle_1", "extruder": "extruder", "map": map, "spool_id": spool, "color": color,
                    "material": "PLA", "load": true, "prep": true, "status": "Loaded" })
        };
        let store = store(json!({
            "AFC": { "current_load": "lane2", "current_state": "Idle", "error_state": false,
                     "lanes": ["lane1", "lane2", "lane3", "lane4"] },
            "AFC_stepper lane1": lane("T0", json!(3), "#FF0000"),
            "AFC_stepper lane2": lane("T1", json!(null), ""),
            "AFC_stepper lane3": { "unit": "Turtle_1", "extruder": "extruder", "map": null, "spool_id": null,
                                   "color": "", "material": "", "load": false, "prep": false, "status": "None" },
            "AFC_stepper lane4": {},
            "AFC_extruder extruder": { "on_shuttle": false, "lane_loaded": "lane2" },
            "configfile": { "settings": { "afc_extruder extruder": { "tool_stn": 72.0 } } },
        }));

        let afc = read_afc(&store).unwrap();

        assert_eq!(afc.current_load.as_deref(), Some("lane2"));
        assert_eq!(afc.lanes.iter().map(|lane| lane.name.as_str()).collect::<Vec<_>>(), ["lane1", "lane2", "lane3"]);
        assert_eq!(afc.lanes.iter().map(|lane| lane.tool).collect::<Vec<_>>(), [Some(0), Some(1), None]);
        assert_eq!((afc.lanes[0].spool_id, afc.lanes[0].color), (Some(3), Some([255, 0, 0, 255])));
        assert_eq!(afc.lanes[0].extruder, "extruder");
        assert!(!afc.lanes[2].filled);
        assert!(afc.heads.is_empty() && !afc.several_units);
    }

    /// AFC's tool changer after `templates/AFC_Hardware_U1.cfg`: each `AFC_extruder` with `toolchanger_unit` is a head
    /// with its own lane of the same name, on the Klipper extruder its `extruder_name` gives.
    #[test]
    fn the_extruders_of_a_toolchanger_unit_are_heads() {
        let head_lane = |extruder: &str, map: &str| {
            json!({ "unit": "Tools", "extruder": extruder, "map": map, "spool_id": null, "color": "",
                    "material": "", "load": true, "prep": true, "status": "Loaded" })
        };
        let section = |extruder: &str| json!({ "toolchanger_unit": "Tools", "extruder_name": extruder });
        let store = store(json!({
            "AFC": { "current_load": "e1", "current_state": "Idle", "error_state": false, "lanes": ["e0", "e1"] },
            "AFC_lane e0": head_lane("e0", "T0"),
            "AFC_lane e1": head_lane("e1", "T1"),
            "AFC_extruder e1": { "on_shuttle": true, "lane_loaded": "e1" },
            "AFC_extruder e0": { "on_shuttle": false, "lane_loaded": "e0" },
            "configfile": { "settings": { "afc_extruder e0": section("extruder"), "afc_extruder e1": section("extruder1") } },
        }));

        let afc = read_afc(&store).unwrap();

        assert_eq!(
            afc.heads,
            [
                AfcHead { name: "e0".into(), extruder: "extruder".into(), mounted: false },
                AfcHead { name: "e1".into(), extruder: "extruder1".into(), mounted: true },
            ]
        );
        assert_eq!(afc.lanes[1].extruder, "extruder1");
    }

    #[test]
    fn a_spool_goes_into_a_lane_and_comes_out_without_id() {
        assert_eq!(assign_spool("lane2", Some(7)), "SET_SPOOL_ID LANE=lane2 SPOOL_ID=7");
        assert_eq!(assign_spool("lane2", None), "SET_SPOOL_ID LANE=lane2");
    }
}
