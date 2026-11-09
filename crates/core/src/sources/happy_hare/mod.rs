//! Happy Hare state from the Klipper objects `mmu` and `mmu_machine`.
//!
//! Field names and value ranges follow Happy Hare 4.1 (`mmu_sync_feedback.py`, `mmu_sensor_manager.py`) and were
//! checked against a real single-gate machine with a proportional sync-feedback buffer and no encoder.

pub mod commands;
mod gates;
mod sensors;
pub mod stale_gate_map;
mod sync_feedback;

pub use gates::{GateAvailability, LoadedFilament, MmuGate, BYPASS};
pub use sensors::{is_shown_by_mmu, FilamentPath, MmuSensor, PathSensor};
pub use sync_feedback::{
    FlowGuard, FlowGuardState, SyncFeedback, SyncFeedbackState, ORIGIN_ENCODER, ORIGIN_SYNC_FEEDBACK,
};

use super::fields::{count, flag, integer, text};
use super::objects::ObjectStore;
use super::subscription::Subscription;
use crate::sourced::Sourced;
use gates::{read_gates, read_tool_to_gate};
use sensors::{listed_sensors, read_sensors};
use serde_json::Value;
use sync_feedback::{read_flow_rate, read_flowguard, read_sync_feedback};

/// Oldest Happy Hare the project supports; older versions report different fields.
const MIN_VERSION: (u32, u32) = (4, 1);

/// Position of the selector servo (`mmu.selector.servo`, `mmu_linear_servo_selector.py`): up releases the filament,
/// down grips it, move holds it while the selector moves.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ServoPosition {
    Up,
    Down,
    Move,
    Unknown,
}

impl ServoPosition {
    fn parse(text: &str) -> Self {
        match text {
            "Up" => Self::Up,
            "Down" => Self::Down,
            "Move" => Self::Move,
            _ => Self::Unknown,
        }
    }
}

/// Happy Hare's `filament_pos` when it does not know (`FILAMENT_POS_UNKNOWN` in `mmu_constants.py`).
const FILAMENT_POS_UNKNOWN: i32 = -1;
#[derive(Debug, Clone, PartialEq, Default)]
pub struct MmuStatus {
    /// `mmu_machine.happy_hare_version`, for example `4.1.0`; empty when not reported.
    pub version: String,
    pub enabled: bool,
    pub num_gates: usize,
    pub num_units: usize,
    /// Selected gate; -1 unknown, -2 bypass.
    pub gate: i32,
    /// Selected tool; -1 unknown, -2 bypass.
    pub tool: i32,
    pub action: String,
    pub operation: String,
    pub print_state: String,
    pub reason_for_pause: String,
    /// What to do about the pause (`pause_hint`, Happy Hare 4.2 and newer); empty before or outside a pause.
    pub pause_hint: String,
    pub filament: LoadedFilament,
    /// Where the filament of the selected gate is, see [`FilamentPath`].
    pub filament_pos: i32,
    /// 1 loading, -1 unloading, 0 unknown.
    pub filament_direction: i32,
    /// Distance the filament has moved in mm, as Happy Hare counts it.
    pub filament_position: f64,
    /// 0 to 1 of the bowden while Happy Hare moves through it; `None` (Happy Hare: -1) otherwise.
    pub bowden_progress: Option<f64>,
    pub homed: bool,
    pub paused: bool,
    pub locked: bool,
    /// Only selectors with a servo report it; `None` for the others.
    pub servo: Option<ServoPosition>,
    pub gates: Vec<MmuGate>,
    /// Gate of each tool (`ttg_map`).
    pub tool_to_gate: Vec<usize>,
    /// EndlessSpool is on (`endless_spool_enabled`); the groups are in [`MmuGate::endless_spool_group`].
    pub endless_spool: bool,
    /// `off`, `readonly`, `push` or `pull`; empty if not reported.
    pub spoolman_support: String,
    pub sensors: Vec<MmuSensor>,
    pub sync_feedback: Option<SyncFeedback>,
    pub flowguard: Option<FlowGuard>,
    /// Flow rate in percent: measured by the encoder, else estimated by the sync-feedback controller with a
    /// proportional sensor. `None` if neither exists.
    pub flow_rate: Option<Sourced<f64>>,
}

impl MmuStatus {
    pub fn filament_path(&self) -> FilamentPath {
        FilamentPath::new(
            &self.sensors,
            self.filament_pos,
            self.filament_direction.signum(),
            self.filament_position,
            self.bowden_progress,
        )
    }

    /// The sensors to list on their own: neither the filament path draws them nor the buffer (sync feedback).
    pub fn listed_sensors(&self) -> Vec<&MmuSensor> {
        listed_sensors(&self.sensors, self.sync_feedback.is_some())
    }

    /// Happy Hare 4.1 or newer; the rest of this model relies on its field names.
    pub fn is_supported(&self) -> bool {
        parse_version(&self.version).is_some_and(|version| version >= MIN_VERSION)
    }

    /// Happy Hare has the say (loading, the reason of a pause, the lanes, its own sensors) only while it is enabled
    /// and supported; otherwise the printer counts as one without it.
    pub fn is_in_charge(&self) -> bool {
        self.enabled && self.is_supported()
    }

    /// Enabled, but older than the oldest supported version: worth a notice instead of its view.
    pub fn is_too_old(&self) -> bool {
        self.enabled && !self.is_supported()
    }

    pub fn bypass_selected(&self) -> bool {
        self.gate == BYPASS
    }

    /// The EndlessSpool groups of all gates with `gate` moved to `group`; `None` if Happy Hare reports no groups.
    pub fn endless_spool_groups_with(&self, gate: usize, group: u32) -> Option<Vec<u32>> {
        self.gates
            .iter()
            .map(|other| if other.index == gate { Some(group) } else { other.endless_spool_group })
            .collect()
    }

    /// Material and colour of a gate can be set in Happy Hare, unless it takes them from Spoolman
    /// (`spoolman_support: pull`; `mmu_gate_map.py` refuses them then).
    pub fn gate_map_is_local(&self) -> bool {
        self.spoolman_support != "pull"
    }
}

/// All fields: the filament view needs gates, sensors, buffer and FlowGuard, and Happy Hare adds fields over time.
pub(super) fn subscribe(subscription: &mut Subscription) {
    subscription.add("mmu", None);
    subscription.add("mmu_machine", None);
}

/// `None` without Happy Hare (no `mmu` object).
pub(super) fn read_mmu(store: &ObjectStore) -> Option<MmuStatus> {
    let mmu = store.object("mmu")?;
    let machine = store.object("mmu_machine").unwrap_or(&Value::Null);
    let num_gates = count::<usize>(mmu, "num_gates").max(count(machine, "num_gates")).unwrap_or_default();
    let sensors = read_sensors(mmu);
    let tool_to_gate = read_tool_to_gate(mmu);
    Some(MmuStatus {
        version: text(machine, "happy_hare_version").to_owned(),
        enabled: flag(mmu, "enabled"),
        num_gates,
        num_units: count::<usize>(machine, "num_units").unwrap_or_default().max(1),
        gate: integer(mmu, "gate", -1),
        tool: integer(mmu, "tool", -1),
        action: text(mmu, "action").to_owned(),
        operation: text(mmu, "operation").to_owned(),
        print_state: text(mmu, "print_state").to_owned(),
        reason_for_pause: text(mmu, "reason_for_pause").to_owned(),
        pause_hint: text(mmu, "pause_hint").to_owned(),
        filament: LoadedFilament::parse(text(mmu, "filament")),
        filament_pos: integer(mmu, "filament_pos", FILAMENT_POS_UNKNOWN),
        filament_direction: integer(mmu, "filament_direction", 0),
        filament_position: mmu.get("filament_position").and_then(Value::as_f64).unwrap_or_default(),
        bowden_progress: mmu
            .get("bowden_progress")
            .and_then(Value::as_f64)
            .filter(|percent| *percent >= 0.0)
            .map(|percent| (percent / 100.0).min(1.0)),
        homed: flag(mmu, "is_homed"),
        paused: flag(mmu, "is_paused"),
        locked: flag(mmu, "is_locked"),
        servo: mmu
            .get("selector")
            .and_then(|selector| selector.get("servo"))
            .and_then(Value::as_str)
            .map(ServoPosition::parse),
        gates: read_gates(mmu, num_gates, &tool_to_gate),
        tool_to_gate,
        endless_spool: flag(mmu, "endless_spool_enabled"),
        spoolman_support: text(mmu, "spoolman_support").to_owned(),
        flow_rate: read_flow_rate(mmu, &sensors),
        flowguard: read_flowguard(mmu, &sensors),
        sensors,
        sync_feedback: read_sync_feedback(mmu),
    })
}
/// `major.minor` of a version such as `4.1.0` or `v4.10`.
fn parse_version(text: &str) -> Option<(u32, u32)> {
    let mut parts = text.trim().trim_start_matches('v').split('.');
    Some((parts.next()?.parse().ok()?, parts.next()?.parse().ok()?))
}

#[cfg(test)]
mod tests {
    use super::gates::gate_color;
    use super::sensors::path_sensor_names;
    use super::*;
    use serde_json::json;

    fn store(status: Value) -> ObjectStore {
        let mut store = ObjectStore::default();
        store.replace(&status);
        store
    }

    /// `mmu` and `mmu_machine` of a real single-gate machine with a proportional buffer and no encoder (Happy Hare 4.1.0).
    fn single_gate_machine() -> ObjectStore {
        store(json!({
            "mmu": {
                "enabled": true, "num_gates": 1, "gate": 0, "tool": 0, "action": "Idle", "operation": "",
                "print_state": "initialized", "reason_for_pause": "", "filament": "Unloaded", "is_homed": true,
                "is_paused": false, "is_locked": false,
                "gate_status": [0], "gate_color": [""], "gate_color_rgb": [[0.0, 0.0, 0.0]],
                "gate_material": [""], "gate_filament_name": [""], "ttg_map": [0],
                "sensors": {
                    "mmu_entry": false, "mmu_exit": false, "filament_compression": false, "filament_tension": true,
                    "filament_proportional": false, "extruder": false, "toolhead": false,
                    "mmu_pre_gate": false, "mmu_gear": false
                },
                "sync_feedback_state": "inactive", "sync_feedback_enabled": 1,
                "sync_feedback_bias_raw": -1.0, "sync_feedback_bias_modelled": -1.0,
                "flowguard": { "trigger": "", "reason": "", "level": 0.0, "max_clog": 0.0, "max_tangle": 0.0,
                               "active": false, "enabled": true },
                "encoder": null
            },
            "mmu_machine": { "happy_hare_version": "4.1.0", "num_units": 1, "num_gates": 1 }
        }))
    }

    #[test]
    fn no_happy_hare_without_the_mmu_object() {
        assert_eq!(read_mmu(&store(json!({ "extruder": {} }))), None);
    }

    #[test]
    fn reads_a_single_gate_machine_with_a_buffer() {
        let mmu = read_mmu(&single_gate_machine()).unwrap();

        assert!(mmu.enabled && mmu.homed && !mmu.paused);
        assert_eq!((mmu.num_gates, mmu.num_units, mmu.gate, mmu.tool), (1, 1, 0, 0));
        assert_eq!(mmu.filament, LoadedFilament::Unloaded);
        assert_eq!(mmu.flow_rate, None);
        assert_eq!(
            mmu.gates,
            vec![MmuGate { index: 0, availability: GateAvailability::Empty, tools: vec![0], ..MmuGate::default() }]
        );
        assert_eq!(mmu.sync_feedback, Some(SyncFeedback { state: SyncFeedbackState::Inactive, bias: -1.0 }));
        let flowguard = mmu.flowguard.unwrap();
        assert!(flowguard.enabled && !flowguard.active);
        assert_eq!(mmu.servo, None);
    }

    #[test]
    fn the_pause_hint_is_empty_before_happy_hare_4_2_and_read_after() {
        assert_eq!(read_mmu(&single_gate_machine()).unwrap().pause_hint, "");

        let with_hint = store(json!({
            "mmu": { "enabled": true, "pause_hint": "Check the nozzle, then RESUME" },
            "mmu_machine": { "happy_hare_version": "4.2.0" }
        }));
        assert_eq!(read_mmu(&with_hint).unwrap().pause_hint, "Check the nozzle, then RESUME");
    }

    #[test]
    fn a_servo_selector_reports_its_position() {
        // `mmu.selector` of an ERCF with `LinearServoSelector` (Happy Hare 4.1.0, 2026-10-28).
        let servo = |selector: Value| read_mmu(&store(json!({ "mmu": { "selector": selector } }))).unwrap().servo;

        assert_eq!(servo(json!({ "has_bypass": true, "servo": "Up" })), Some(ServoPosition::Up));
        assert_eq!(servo(json!({ "servo": "Down" })), Some(ServoPosition::Down));
        assert_eq!(servo(json!({ "servo": "Move" })), Some(ServoPosition::Move));
        assert_eq!(servo(json!({ "servo": "Unknown" })), Some(ServoPosition::Unknown));
        assert_eq!(servo(json!({ "has_bypass": false })), None);
    }

    #[test]
    fn only_happy_hare_4_1_or_newer_is_supported() {
        let with_version = |version: &str| {
            read_mmu(&store(json!({ "mmu": {}, "mmu_machine": { "happy_hare_version": version } }))).unwrap()
        };

        assert!(read_mmu(&single_gate_machine()).unwrap().is_supported());
        assert!(with_version("4.1.0").is_supported() && with_version("v4.10").is_supported());
        assert!(with_version("5.0").is_supported());
        assert!(!with_version("4.0.2").is_supported() && !with_version("3.4").is_supported());
        assert!(!with_version("").is_supported() && !with_version("beta").is_supported());
    }

    #[test]
    fn sensors_skip_the_legacy_names_and_keep_disabled_ones() {
        let sensors = read_mmu(&single_gate_machine()).unwrap().sensors;
        let names: Vec<&str> = sensors.iter().map(|sensor| sensor.name.as_str()).collect();

        assert!(!names.contains(&"mmu_pre_gate") && !names.contains(&"mmu_gear"));
        assert!(names.contains(&"mmu_entry") && names.contains(&"filament_proportional"));
        let tension = sensors.iter().find(|sensor| sensor.name == "filament_tension").unwrap();
        assert_eq!(tension.triggered, Some(true));

        let disabled = store(json!({ "mmu": { "sensors": { "toolhead": null } } }));
        assert_eq!(read_mmu(&disabled).unwrap().sensors, vec![MmuSensor { name: "toolhead".into(), triggered: None }]);
    }

    #[test]
    fn buffer_sensors_are_only_listed_without_sync_feedback_and_path_sensors_never() {
        let sensors = ["mmu_entry", "mmu_exit", "filament_tension", "filament_proportional", "toolhead"]
            .map(|name| MmuSensor { name: name.into(), triggered: Some(false) })
            .to_vec();
        let names = |sync_feedback| -> Vec<String> {
            let mmu = MmuStatus { sensors: sensors.clone(), sync_feedback, ..MmuStatus::default() };
            mmu.listed_sensors().iter().map(|sensor| sensor.name.clone()).collect()
        };

        assert!(names(Some(SyncFeedback::default())).is_empty(), "entry, exit and toolhead are on the filament path");
        assert_eq!(names(None), ["filament_tension", "filament_proportional"]);
    }

    #[test]
    fn a_gate_colour_keeps_the_alpha_of_gate_color() {
        let rgb = json!([[0.89, 0.12, 0.17]]);
        assert_eq!(gate_color("e31f2b80", rgb.get(0)), Some([227, 31, 43, 0x80]));
        assert_eq!(gate_color("e31f2b", rgb.get(0)), Some([227, 31, 43, 0xff]));
        assert_eq!(gate_color("red", rgb.get(0)), Some([227, 31, 43, 0xff]));
        assert_eq!(gate_color("", rgb.get(0)), None);
    }

    #[test]
    fn reads_gates_with_colours_and_tool_mapping() {
        let store = store(json!({
            "mmu": {
                "num_gates": 3, "gate": 1,
                "gate_status": [1, 2, -1], "gate_filament_name": ["Red PLA", "", "Blue"],
                "gate_material": ["PLA", "", "PETG"], "gate_color": ["red", "", "blue"],
                "gate_color_rgb": [[1.0, 0.0, 0.0], [0.0, 0.0, 0.0], [0.0, 0.0, 1.0]],
                "ttg_map": [2, 0, 1]
            }
        }));

        let mmu = read_mmu(&store).unwrap();

        assert_eq!(mmu.gate, 1);
        assert_eq!(mmu.gates[0].availability, GateAvailability::Available);
        assert_eq!(mmu.gates[0].color, Some([255, 0, 0, 255]));
        assert_eq!(mmu.gates[0].tools, [1]);
        assert_eq!(mmu.gates[1].availability, GateAvailability::Buffered);
        assert_eq!(mmu.gates[1].color, None);
        assert_eq!(mmu.gates[2].availability, GateAvailability::Unknown);
        assert_eq!((mmu.gates[2].name.as_str(), mmu.gates[2].material.as_str()), ("Blue", "PETG"));
        assert_eq!(mmu.gates[2].tools, [0]);
    }

    #[test]
    fn bypass_is_reported_as_gate_minus_two() {
        let bypass = read_mmu(&store(json!({ "mmu": { "num_gates": 2, "gate": -2 } }))).unwrap();
        assert!(bypass.bypass_selected());
    }

    #[test]
    fn a_gate_knows_all_its_tools_and_its_endless_spool_group() {
        let mmu = read_mmu(&store(json!({ "mmu": {
            "num_gates": 3, "ttg_map": [0, 2, 0], "endless_spool_enabled": 1, "endless_spool_groups": [0, 1, 0]
        } })))
        .unwrap();

        let tools: Vec<Vec<usize>> = mmu.gates.iter().map(|gate| gate.tools.clone()).collect();
        assert_eq!(tools, [vec![0, 2], vec![], vec![1]]);
        assert!(mmu.endless_spool);
        assert_eq!(mmu.endless_spool_groups_with(1, 0), Some(vec![0, 0, 0]));
        let without_groups = read_mmu(&store(json!({ "mmu": { "num_gates": 2 } }))).unwrap();
        assert_eq!(without_groups.endless_spool_groups_with(0, 1), None);
    }

    #[test]
    fn buffer_states_follow_the_happy_hare_strings() {
        let state = |text: &str| {
            read_mmu(&store(json!({ "mmu": { "sync_feedback_state": text, "sync_feedback_bias_raw": 0.4 } })))
                .unwrap()
                .sync_feedback
                .unwrap()
        };

        assert_eq!(state("compressed").state, SyncFeedbackState::Compressed);
        assert_eq!(state("tension").state, SyncFeedbackState::Tension);
        assert_eq!(state("neutral").state, SyncFeedbackState::Neutral);
        assert_eq!(state("something else").state, SyncFeedbackState::Disabled);
        assert_eq!(state("neutral").bias, 0.4);
        assert_eq!(read_mmu(&store(json!({ "mmu": { "sync_feedback_state": null } }))).unwrap().sync_feedback, None);
    }

    #[test]
    fn flow_rate_comes_from_the_encoder_dictionary() {
        let store = store(json!({ "mmu": { "encoder": { "flow_rate": 97.5, "encoder_pos": 12.0 } } }));
        assert_eq!(read_mmu(&store).unwrap().flow_rate, Some(Sourced::measured(97.5, ORIGIN_ENCODER)));
    }

    #[test]
    fn flow_rate_without_encoder_is_estimated_by_the_proportional_sensor() {
        let store = store(json!({ "mmu": {
            "encoder": null, "sensors": { "filament_proportional": false }, "sync_feedback_flow_rate": 93.4
        } }));
        assert_eq!(read_mmu(&store).unwrap().flow_rate, Some(Sourced::estimated(93.4, ORIGIN_SYNC_FEEDBACK)));
    }

    #[test]
    fn encoder_flow_rate_wins_over_the_estimate() {
        let store = store(json!({ "mmu": {
            "encoder": { "flow_rate": 97.5 }, "sensors": { "filament_proportional": true },
            "sync_feedback_flow_rate": 93.4
        } }));
        assert_eq!(read_mmu(&store).unwrap().flow_rate, Some(Sourced::measured(97.5, ORIGIN_ENCODER)));
    }

    #[test]
    fn buffer_without_proportional_sensor_has_no_flow_rate() {
        // Happy Hare reports the initial 100 for tension/compression buffers, which is no estimate.
        let store = store(json!({ "mmu": {
            "sensors": { "filament_tension": true }, "sync_feedback_flow_rate": 100.0
        } }));
        assert_eq!(read_mmu(&store).unwrap().flow_rate, None);
    }

    #[test]
    fn disabled_proportional_sensor_has_no_flow_rate() {
        let store = store(json!({ "mmu": {
            "sensors": { "filament_proportional": null }, "sync_feedback_flow_rate": 100.0
        } }));
        assert_eq!(read_mmu(&store).unwrap().flow_rate, None);
    }

    #[test]
    fn flow_rate_is_missing_without_buffer() {
        let store = store(json!({ "mmu": {
            "sensors": { "filament_proportional": false }, "sync_feedback_flow_rate": null
        } }));
        assert_eq!(read_mmu(&store).unwrap().flow_rate, None);
    }

    #[test]
    fn the_sensors_of_happy_hare_are_told_apart_from_other_klipper_sensors() {
        assert!(is_shown_by_mmu("filament_switch_sensor mmu_entry_0"));
        assert!(is_shown_by_mmu("filament_switch_sensor filament_tension"));
        assert!(is_shown_by_mmu("filament_switch_sensor unit0:mmu_exit_1"));
        assert!(is_shown_by_mmu("filament_switch_sensor toolhead"));
        assert!(is_shown_by_mmu("filament_switch_sensor extruder"));
        assert!(!is_shown_by_mmu("filament_switch_sensor spool_runout"));
        assert!(!is_shown_by_mmu("filament_motion_sensor extruder_encoder"));
    }

    #[test]
    fn flowguard_state_is_one_word_where_a_trigger_wins() {
        let flowguard = |enabled, active, trigger: &str| FlowGuard {
            enabled,
            active,
            trigger: trigger.into(),
            ..FlowGuard::default()
        };

        assert_eq!(flowguard(false, true, "clog").state(), FlowGuardState::Off);
        assert_eq!(flowguard(true, true, "clog").state(), FlowGuardState::Triggered);
        assert_eq!(flowguard(true, true, "").state(), FlowGuardState::Monitoring);
        assert_eq!(flowguard(true, false, "").state(), FlowGuardState::Ready);
    }

    #[test]
    fn boosting_tangle_prevention_shows_between_trigger_and_monitoring() {
        let boosting = |trigger: &str| FlowGuard {
            enabled: true,
            active: true,
            boosting: true,
            trigger: trigger.into(),
            ..FlowGuard::default()
        };

        assert_eq!(boosting("").state(), FlowGuardState::Boosting);
        assert_eq!(boosting("tangle").state(), FlowGuardState::Triggered);
        assert_eq!(FlowGuard { boosting: true, ..FlowGuard::default() }.state(), FlowGuardState::Off);
    }

    #[test]
    fn the_boost_needs_the_switch_and_a_proportional_sensor() {
        let boosting = |sensors: Value, enabled: bool| {
            read_mmu(&store(json!({ "mmu": {
                "sensors": sensors,
                "flowguard": { "enabled": true, "active": true },
                "tangle_prevention": { "enabled": enabled, "active": true, "boosted": true, "threshold": 0.3 }
            } })))
            .unwrap()
            .flowguard
            .unwrap()
            .boosting
        };

        assert!(boosting(json!({ "filament_proportional": false }), true));
        assert!(!boosting(json!({ "filament_proportional": false }), false));
        assert!(!boosting(json!({ "filament_tension": true }), true));
    }

    #[test]
    fn the_path_has_the_gate_entry_and_exit_and_skips_disabled_sensors() {
        let path = |sensors: Value, progress: f64| {
            read_mmu(&store(json!({ "mmu": {
                "filament_pos": 3, "filament_direction": 1, "filament_position": 412.6, "sensors": sensors,
                "bowden_progress": progress
            } })))
            .unwrap()
            .filament_path()
        };

        let both = path(json!({ "mmu_entry": true, "mmu_exit": false, "extruder": null, "toolhead": true }), 35.0);
        assert_eq!((both.position, both.direction, both.distance, both.bowden_progress), (3, 1, 412.6, Some(0.35)));
        assert_eq!((both.gate_entry_sensor, both.gate_exit_sensor), (PathSensor::Triggered, PathSensor::Free));
        assert_eq!(both.extruder_sensor, PathSensor::Absent, "disabled");
        assert_eq!(both.toolhead_sensor, PathSensor::Triggered);
        let shared = path(json!({ "mmu_shared_exit": true }), -1.0);
        assert_eq!((shared.gate_exit_sensor, shared.bowden_progress), (PathSensor::Triggered, None));
        assert_eq!(read_mmu(&store(json!({ "mmu": {} }))).unwrap().filament_path().position, -1);
    }

    #[test]
    fn the_sensors_on_the_path_are_named_once() {
        let sensors = |names: &[(&str, bool)]| -> Vec<MmuSensor> {
            names.iter().map(|(name, on)| MmuSensor { name: (*name).into(), triggered: Some(*on) }).collect()
        };
        assert_eq!(
            path_sensor_names(&sensors(&[("mmu_entry", true), ("mmu_exit", false)])),
            ["mmu_entry", "mmu_exit", "extruder", "toolhead"]
        );
        assert_eq!(path_sensor_names(&sensors(&[("mmu_entry", true)])), ["mmu_entry", "extruder", "toolhead"]);
    }

    #[test]
    fn every_sensor_of_the_path_is_optional() {
        let path = |sensors: Value| {
            read_mmu(&store(json!({ "mmu": { "filament_pos": 10, "sensors": sensors } }))).unwrap().filament_path()
        };
        let all_absent = |path: FilamentPath| {
            [path.gate_entry_sensor, path.gate_exit_sensor, path.extruder_sensor, path.toolhead_sensor]
                .iter()
                .all(|sensor| *sensor == PathSensor::Absent)
        };

        assert!(all_absent(path(json!({}))), "no sensors at all: the path is drawn without marks");
        assert!(all_absent(read_mmu(&store(json!({ "mmu": {} }))).unwrap().filament_path()), "no sensor list");
        let only_toolhead = path(json!({ "toolhead": true }));
        assert_eq!(only_toolhead.toolhead_sensor, PathSensor::Triggered);
        assert_eq!(
            (only_toolhead.gate_entry_sensor, only_toolhead.gate_exit_sensor, only_toolhead.extruder_sensor),
            (PathSensor::Absent, PathSensor::Absent, PathSensor::Absent)
        );
        assert_eq!(path(json!({})).position, 10, "the position does not depend on sensors");
    }
}
