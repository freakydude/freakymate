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
const HUB_PREFIX: &str = "AFC_hub ";
const BUFFER_PREFIX: &str = "AFC_buffer ";
/// Section prefixes of lanes in `configfile.settings` (lowercased like every section there).
const LANE_SECTIONS: [&str; 2] = ["afc_stepper ", "afc_lane "];
const HUB_SECTION: &str = "afc_hub ";
const AFC_FIELDS: [&str; 4] = ["current_load", "current_state", "error_state", "lanes"];
const LANE_FIELDS: [&str; 13] = [
    "unit",
    "extruder",
    "hub",
    "buffer",
    "map",
    "spool_id",
    "color",
    "material",
    "load",
    "prep",
    "loaded_to_hub",
    "tool_loaded",
    "status",
];
const EXTRUDER_FIELDS: [&str; 6] =
    ["on_shuttle", "lane_loaded", "tool_start", "tool_start_status", "tool_end", "tool_end_status"];
const HUB_FIELDS: [&str; 1] = ["state"];
const BUFFER_FIELDS: [&str; 1] = ["state"];
/// The hub names that stand for no hub: the lane loads straight to the toolhead (`AFC_lane.py` L63).
const DIRECT_HUBS: [&str; 2] = ["direct", "direct_load"];
/// `pin_tool_start: buffer` lets the buffer stand in for the switch (`AFC_extruder.py` L291-301).
const BUFFER_AS_TOOL_START: &str = "buffer";

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
    /// The hubs the lanes name, with their switch.
    pub hubs: Vec<AfcHub>,
    /// The buffers the lanes name.
    pub buffers: Vec<AfcBuffer>,
    /// Every `AFC_extruder` the lanes name, with the sensors at the toolhead.
    pub extruders: Vec<AfcExtruder>,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct AfcLane {
    /// The lane's name (`lane1`), which its commands take as `LANE=`.
    pub name: String,
    /// The unit that carries the lane (`Turtle_1`).
    pub unit: String,
    /// The `AFC_extruder` the lane feeds (`extruder` in its status), which carries the toolhead sensors.
    pub afc_extruder: String,
    /// The Klipper extruder the lane feeds.
    pub extruder: String,
    /// The hub the lane runs through; `None` for a lane that loads straight to the toolhead (`hub: direct`).
    pub hub: Option<String>,
    /// The buffer on the lane's way (the lane's own, else its unit's; `AFC_lane.py` L628-641).
    pub buffer: Option<String>,
    /// The `T<n>` mapped to the lane (`map`).
    pub tool: Option<usize>,
    pub spool_id: Option<u32>,
    pub color: Option<Rgba>,
    pub material: String,
    /// Filament sits in the lane (`prep` or `load`).
    pub filled: bool,
    /// The prep switch where the spool's filament enters; `None` without one (`prep:` in the section).
    pub prep: Option<bool>,
    /// The load switch at the lane's exit; `None` without one (`load:`).
    pub load: Option<bool>,
    /// The filament reaches the hub (`loaded_to_hub`).
    pub at_hub: bool,
    /// The filament is loaded into the toolhead (`tool_loaded`).
    pub in_tool: bool,
    /// `status`: `None`, `Loaded`, `Tooled`, … (`AFC_lane.py` L65).
    pub status: String,
}

impl AfcLane {
    /// The direction the lane moves its filament from `status` (`AFC_lane.py` L65-75): 1 towards the toolhead, -1 back,
    /// 0 at rest.
    pub fn direction(&self) -> i32 {
        match self.status.as_str() {
            "Tool Loading" | "HUB Loading" => 1,
            "Tool Unloading" | "Ejecting" => -1,
            _ => 0,
        }
    }
}

/// A hub of AFC; `switch` is `None` for a virtual hub, which has no switch of its own (`switch_pin: virtual`,
/// `AFC_hub.py` L58-66).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct AfcHub {
    pub name: String,
    pub switch: Option<bool>,
}

/// The state AFC reports for a buffer (`AFC_buffer.py` L34-35): `Advancing` while its expanded switch is triggered
/// (the toolhead pulls, the lane feeds faster), `Trailing` while the compressed one is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum BufferState {
    Advancing,
    Trailing,
    #[default]
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct AfcBuffer {
    pub name: String,
    pub state: BufferState,
}

/// An `AFC_extruder` with its switches at the toolhead; `None` where none is fitted. A `tool_start` of `buffer` is the
/// buffer standing in for the sensor, not a switch (`AFC_extruder.py` L291-301).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct AfcExtruder {
    pub name: String,
    /// The switch before the extruder gears (`pin_tool_start`, `tool_start_status`).
    pub tool_start: Option<bool>,
    /// The switch after the extruder gears (`pin_tool_end`, `tool_end_status`).
    pub tool_end: Option<bool>,
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
    subscription.add_each(HUB_PREFIX, Some(&HUB_FIELDS));
    subscription.add_each(BUFFER_PREFIX, Some(&BUFFER_FIELDS));
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
        hubs: distinct(lanes.iter().filter_map(|lane| lane.hub.as_deref()))
            .map(|hub| read_hub(store, settings, hub))
            .collect(),
        buffers: distinct(lanes.iter().filter_map(|lane| lane.buffer.as_deref()))
            .map(|buffer| read_buffer(store, buffer))
            .collect(),
        extruders: distinct(lanes.iter().map(|lane| lane.afc_extruder.as_str()))
            .map(|extruder| read_extruder(store, extruder))
            .collect(),
        lanes,
        heads,
        several_units,
    })
}

/// `names` without repeats, in the order they first come.
fn distinct<'a>(names: impl Iterator<Item = &'a str>) -> impl Iterator<Item = &'a str> {
    let mut seen: Vec<&str> = Vec::new();
    names.filter(move |name| {
        let new = !seen.contains(name);
        if new {
            seen.push(name);
        }
        new
    })
}

/// A lane reports nothing until AFC has finished starting up (`AFC_lane.py` L2183); such a lane is skipped.
fn read_lane(store: &ObjectStore, settings: &Value, name: &str) -> Option<AfcLane> {
    let lane = LANE_PREFIXES.iter().find_map(|prefix| store.object(&format!("{prefix}{name}")))?;
    let afc_extruder = lane.get("extruder").and_then(Value::as_str)?;
    let section = LANE_SECTIONS.iter().find_map(|prefix| settings.get(format!("{prefix}{}", name.to_lowercase())));
    // A switch exists where its pin is configured (`AFC_lane.py` L224-232); the lanes AFC creates for the heads of its
    // tool changer have no section and no switches of their own (`AFC_extruder.py` L327-332).
    let switch = |key: &str| section.filter(|section| !text(section, key).is_empty()).map(|_| flag(lane, key));
    Some(AfcLane {
        name: name.to_owned(),
        unit: text(lane, "unit").to_owned(),
        afc_extruder: afc_extruder.to_owned(),
        extruder: klipper_extruder(settings, afc_extruder),
        hub: Some(text(lane, "hub")).filter(|hub| !hub.is_empty() && !DIRECT_HUBS.contains(hub)).map(str::to_owned),
        buffer: Some(text(lane, "buffer")).filter(|buffer| !buffer.is_empty()).map(str::to_owned),
        tool: text(lane, "map").strip_prefix('T').and_then(|number| number.parse().ok()),
        spool_id: count(lane, "spool_id"),
        color: color::parse_hex(text(lane, "color")),
        material: text(lane, "material").to_owned(),
        filled: flag(lane, "prep") || flag(lane, "load"),
        prep: switch("prep"),
        load: switch("load"),
        at_hub: flag(lane, "loaded_to_hub"),
        in_tool: flag(lane, "tool_loaded"),
        status: text(lane, "status").to_owned(),
    })
}

/// A hub with its switch; a virtual hub (`switch_pin: virtual`) or one whose section is unknown has none.
fn read_hub(store: &ObjectStore, settings: &Value, name: &str) -> AfcHub {
    let pin = settings.get(format!("{HUB_SECTION}{}", name.to_lowercase())).map(|section| text(section, "switch_pin"));
    let object = store.object(&format!("{HUB_PREFIX}{name}")).unwrap_or(&Value::Null);
    let switch = pin.filter(|pin| !pin.is_empty() && *pin != "virtual").map(|_| flag(object, "state"));
    AfcHub { name: name.to_owned(), switch }
}

fn read_buffer(store: &ObjectStore, name: &str) -> AfcBuffer {
    let object = store.object(&format!("{BUFFER_PREFIX}{name}")).unwrap_or(&Value::Null);
    let state = match text(object, "state") {
        "Advancing" => BufferState::Advancing,
        "Trailing" => BufferState::Trailing,
        _ => BufferState::Unknown,
    };
    AfcBuffer { name: name.to_owned(), state }
}

/// The switches at the toolhead from the status: `tool_start` and `tool_end` hold the pin, `null` without one.
fn read_extruder(store: &ObjectStore, name: &str) -> AfcExtruder {
    let object = store.object(&format!("{EXTRUDER_PREFIX}{name}")).unwrap_or(&Value::Null);
    let switch = |pin: &str, state: &str| {
        let pin = text(object, pin);
        (!pin.is_empty() && pin != BUFFER_AS_TOOL_START).then(|| flag(object, state))
    };
    AfcExtruder {
        name: name.to_owned(),
        tool_start: switch("tool_start", "tool_start_status"),
        tool_end: switch("tool_end", "tool_end_status"),
    }
}

/// A slicer tool and the lane AFC feeds it from (`map` of the lane).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolLane {
    pub tool: usize,
    pub lane: String,
}

/// A lane whose filament AFC can pull back until its hub is clear (`AFC_LANE_RESET`), and why not right now.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HubReturn {
    pub lane: String,
    /// The unit of the lane where AFC has several, else `None`.
    pub unit: Option<String>,
    /// `None` while AFC would carry it out.
    pub blocked: Option<HubReturnBlock>,
}

/// Why AFC does not pull a lane back to its hub now (`AFC_functions.py` L1685-1692 at 5ea84ea).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HubReturnBlock {
    /// The named lane is loaded in the toolhead; AFC wants it unloaded first ("Toolhead is loaded with '…', unload or
    /// check sensor before resetting lane"; AFC only logs it and goes on, so it is held back here).
    ///
    /// AFC-BUG-WORKAROUND: this block deliberately deviates from the rule "hints, not locks". Its message says the
    /// reset must stop, but unlike the hub check right above it, `AFC_functions.py` L1690-1692 has no `return`, so AFC
    /// pulls filament back out of a loaded head (verified at 5ea84ea, 2026-11-12). We treat this as a technical bug in
    /// AFC, not as behaviour to rely on. Once AFC fixes it (issue: see the backlog item "AFC issue"), remove this
    /// variant, let `hub_returns` ignore `current_load` and show only a hint.
    ToolLoaded(String),
    /// The hub's switch sees no filament; AFC refuses ("Hub is already clear while trying to reset '…'").
    HubClear,
}

impl AfcStatus {
    pub fn lane(&self, name: &str) -> Option<&AfcLane> {
        self.lanes.iter().find(|lane| lane.name == name)
    }

    pub fn hub(&self, name: &str) -> Option<&AfcHub> {
        self.hubs.iter().find(|hub| hub.name == name)
    }

    pub fn buffer(&self, name: &str) -> Option<&AfcBuffer> {
        self.buffers.iter().find(|buffer| buffer.name == name)
    }

    pub fn extruder(&self, name: &str) -> Option<&AfcExtruder> {
        self.extruders.iter().find(|extruder| extruder.name == name)
    }

    /// The tools AFC maps to its lanes, by tool number; a lane without `map` has none. `SET_MAP` swaps, so a tool has
    /// one lane (`AFC_spool.py` L147-185).
    pub fn tool_lanes(&self) -> Vec<ToolLane> {
        let mut tools: Vec<ToolLane> = self
            .lanes
            .iter()
            .filter_map(|lane| lane.tool.map(|tool| ToolLane { tool, lane: lane.name.clone() }))
            .collect();
        tools.sort_by_key(|entry| entry.tool);
        tools
    }

    /// The lanes `AFC_LANE_RESET` can pull back, as AFC's own prompt `AFC_RESET` offers them: filament at the load
    /// switch (`AFC_functions.py` L1617-1626; without a load pin AFC counts the lane as loaded, `AFC_lane.py` L229-233).
    /// A lane without a hub (`hub: direct`) is left out: its stand-in hub is always clear, so AFC always refuses
    /// (`AFC_lane.py` L611-615). With a virtual hub (`switch_pin: virtual`) the status `load` is `loaded_to_hub`
    /// instead of the switch (`AFC_lane.py` L1093-1103).
    pub fn hub_returns(&self) -> Vec<HubReturn> {
        let blocked = |hub: &str| match &self.current_load {
            Some(loaded) => Some(HubReturnBlock::ToolLoaded(loaded.clone())),
            None if self.hub(hub).and_then(|hub| hub.switch) == Some(false) => Some(HubReturnBlock::HubClear),
            None => None,
        };
        self.lanes
            .iter()
            .filter(|lane| lane.load.unwrap_or(true))
            .filter_map(|lane| {
                let hub = lane.hub.as_deref()?;
                Some(HubReturn {
                    lane: lane.name.clone(),
                    unit: self.several_units.then(|| lane.unit.clone()),
                    blocked: blocked(hub),
                })
            })
            .collect()
    }

    /// The names of the switch sensors AFC creates for its lanes, hubs, buffers and toolheads (`AFC_utils.py` L42-90,
    /// `<lane>_prep`, `<hub>_Hub`, …); they belong to its view, not to the printer's list of sensors.
    pub fn owns_sensor(&self, name: &str) -> bool {
        let of = |owner: &str, suffixes: &[&str]| {
            name.strip_prefix(owner)
                .and_then(|rest| rest.strip_prefix('_'))
                .is_some_and(|rest| suffixes.contains(&rest))
        };
        self.lanes.iter().any(|lane| of(&lane.name, &["prep", "load"]))
            || self.hubs.iter().any(|hub| of(&hub.name, &["Hub"]))
            || self.buffers.iter().any(|buffer| of(&buffer.name, &["expanded", "compressed"]))
            || self.extruders.iter().any(|extruder| of(&extruder.name, &["tool_start", "tool_end"]))
    }
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

/// Unloads the lane in the toolhead and loads `lane` (`CHANGE_TOOL`, `AFC.py` L2293-2330); runs in a print, too.
pub fn load_lane(lane: &str) -> String {
    format!("CHANGE_TOOL LANE={lane}")
}

/// Unloads `lane` from the toolhead back to its hub (`TOOL_UNLOAD`, `AFC.py` L1828-1850).
pub fn unload_lane(lane: &str) -> String {
    format!("TOOL_UNLOAD LANE={lane}")
}

/// Maps `T<tool>` to `lane`; the lane that had it gets the old tool of `lane` (`SET_MAP`, `AFC_spool.py` L147-185).
/// Moves nothing and is not refused while printing (AFC runs it itself on a runout).
pub fn map_tool(lane: &str, tool: usize) -> String {
    format!("SET_MAP LANE={lane} MAP=T{tool}")
}

/// Back to the tool map of the configuration; keeps the runout lanes (`RESET_AFC_MAPPING`, `AFC_spool.py` L527-566).
pub const RESET_TOOL_MAP: &str = "RESET_AFC_MAPPING RUNOUT=no";

/// Clears the error state and sets AFC idle; no resume (`RESET_FAILURE`, `AFC_error.py` L172-195).
pub const RESET_FAILURE: &str = "RESET_FAILURE";

/// Pulls the filament of `lane` back until the hub is clear (`AFC_LANE_RESET`, `AFC_functions.py` L1640-1750).
pub fn reset_lane(lane: &str) -> String {
    format!("AFC_LANE_RESET LANE={lane}")
}

/// Docks the head on the shuttle of AFC's tool changer through its `custom_unselect`, else viesturz `UNSELECT_TOOL`
/// (`AFC_Toolchanger.py` L51-52, L153-190). Registered only with an `[AFC_Toolchanger]`; with `show_macros` (the
/// default, `AFC.py` L318) the name is a macro that calls `_AFC_UNSELECT_TOOL` (`AFC_functions.py` L869-893), so this
/// name works either way.
pub const DROP_HEAD: &str = "AFC_UNSELECT_TOOL";

/// Ejects the filament of `lane` out of the unit (`LANE_UNLOAD`, `AFC.py` L1321-1338); AFC refuses it while printing.
pub fn eject_lane(lane: &str) -> String {
    format!("LANE_UNLOAD LANE={lane}")
}

/// Material and colour of a lane (`SET_MATERIAL`, `SET_COLOR` with a hex code, `AFC_spool.py` L205-299), one line
/// each; an empty material is not sent.
pub fn set_filament(lane: &str, material: Option<&str>, color: Option<[u8; 3]>) -> String {
    let material = material
        .filter(|material| !material.is_empty())
        .map(|material| format!("SET_MATERIAL LANE={lane} MATERIAL={material}"));
    let color = color.map(|[red, green, blue]| format!("SET_COLOR LANE={lane} COLOR={red:02X}{green:02X}{blue:02X}"));
    material.into_iter().chain(color).collect::<Vec<_>>().join("\n")
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
