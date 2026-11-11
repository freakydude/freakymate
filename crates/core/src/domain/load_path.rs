//! The load path of one lane: its measuring points from the spool to the nozzle and where the filament is on it, the
//! same for a gate of Happy Hare and a lane of AFC (`docs/multi-tool.md`, D2). A point that is not fitted, or not tied
//! to this lane by the printer, is [`PathSensor::Absent`] and not drawn.
//!
//! Happy Hare reports the sensors of the selected gate in `mmu.sensors`; those of the other gates only as Klipper
//! objects, named `[<owner>:]<sensor>`: gate sensors without owner and with the global gate number, the shared exit
//! owned by its unit, the extruder and toolhead sensors by the unit's `toolhead`. With one unit Happy Hare drops the
//! owner (`mmu_sensor_utils.py` L363-368 at 7df830f). AFC reports every switch in the status of its lane, hub and
//! `AFC_extruder`.

use super::machine_tree::LaneId;
use crate::sources::afc::{AfcStatus, BufferState};
use crate::sources::happy_hare::{GateAvailability, MmuSensor, MmuStatus, MmuUnit, PathSensor, SyncFeedbackState};
use crate::sources::klipper::FilamentSensor;
use crate::sources::PrinterStatus;

const SWITCH_SENSOR: &str = "filament_switch_sensor ";
const GATE_ENTRY: &str = "mmu_entry";
const GATE_EXIT: &str = "mmu_exit";
const SHARED_EXIT: &str = "mmu_shared_exit";
const EXTRUDER_ENTRY: &str = "extruder";
const TOOLHEAD: &str = "toolhead";

/// Where the filament is, on Happy Hare's scale (`filament_pos`, `mmu_constants.py` L84-95): -1 unknown, 0 parked in
/// the lane, 1 at the hub, 2 to 4 in the bowden, 5 at the extruder sensor, 7 in the extruder entry, 8 at the toolhead
/// sensor, 9 in the extruder, 10 loaded to the nozzle.
pub const UNKNOWN: i32 = -1;
pub const PARKED: i32 = 0;
pub const AT_HUB: i32 = 1;
pub const LOADED: i32 = 10;

/// The side the buffer on the way leans to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BufferSide {
    /// The toolhead pulls (Happy Hare `tension`, AFC `Advancing`: its `expanded` switch).
    Tension,
    Neutral,
    /// The lane pushes (Happy Hare `compressed`, AFC `Trailing`: its `compressed` switch).
    Compression,
    /// The buffer reports no side (idle, disabled).
    Unknown,
}

#[derive(Debug, Clone, PartialEq)]
pub struct LoadPath {
    /// Where the filament enters the lane from the spool (Happy Hare `mmu_entry`, AFC `prep`).
    pub entry: PathSensor,
    /// Where it leaves the lane (Happy Hare `mmu_exit`, AFC `load`).
    pub exit: PathSensor,
    /// Where the lanes of a unit meet (Happy Hare `mmu_shared_exit`, AFC hub).
    pub hub: PathSensor,
    /// Before the extruder gears (Happy Hare `extruder`, AFC `tool_start`).
    pub extruder: PathSensor,
    /// After the extruder gears (Happy Hare `toolhead`, AFC `tool_end`).
    pub toolhead: PathSensor,
    /// The buffer on the way, where the source reports one for this lane.
    pub buffer: Option<BufferSide>,
    /// Where the filament is (see [`LOADED`] for the scale); `None` while the lane holds none.
    pub position: Option<i32>,
    /// 1 loading, -1 unloading, 0 at rest.
    pub direction: i32,
    /// The distance Happy Hare counts while it moves the filament, in mm; `None` where the source has none.
    pub distance: Option<f64>,
    /// 0 to 1 of the bowden while Happy Hare moves through it.
    pub bowden_progress: Option<f64>,
}

impl LoadPath {
    fn empty() -> Self {
        Self {
            entry: PathSensor::Absent,
            exit: PathSensor::Absent,
            hub: PathSensor::Absent,
            extruder: PathSensor::Absent,
            toolhead: PathSensor::Absent,
            buffer: None,
            position: None,
            direction: 0,
            distance: None,
            bowden_progress: None,
        }
    }
}

/// The load path of `lane`; `None` for a lane the sources do not know (the direct feed has no path of its own).
pub fn load_path(status: &PrinterStatus, lane: &LaneId) -> Option<LoadPath> {
    match lane {
        LaneId::Gate(gate) => status.happy_hare().and_then(|mmu| gate_path(mmu, &status.filament_sensors, *gate)),
        LaneId::Bypass => status.happy_hare().map(bypass_path),
        LaneId::Afc(name) => status.afc.as_ref().and_then(|afc| afc_lane_path(afc, name)),
    }
}

/// The part of the path at the head that feeds `extruder`: its sensors before and after the gears and the filament of
/// the lane loaded into it; entry, exit and hub stay absent. Without a tied sensor or a loaded lane all is empty.
pub fn head_path(status: &PrinterStatus, extruder: &str) -> LoadPath {
    let mut path = LoadPath::empty();
    if let Some(mmu) = status.happy_hare() {
        if let Some(toolhead) = toolhead_of(mmu, extruder) {
            path.extruder = owned_sensor(&status.filament_sensors, single_unit(mmu), toolhead, EXTRUDER_ENTRY);
            path.toolhead = owned_sensor(&status.filament_sensors, single_unit(mmu), toolhead, TOOLHEAD);
        }
        let selected = usize::try_from(mmu.gate).ok().and_then(|gate| unit_of(mmu, gate));
        if selected.is_some_and(|unit| unit.extruder == extruder) {
            path.position = Some(mmu.filament_pos);
            path.direction = mmu.filament_direction.signum();
        }
    }
    if let Some(afc) = &status.afc {
        let lanes = afc.lanes.iter().filter(|lane| lane.extruder == extruder);
        for lane in lanes {
            if let Some(own) = afc.extruder(&lane.afc_extruder) {
                path.extruder = PathSensor::of_switch(own.tool_start);
                path.toolhead = PathSensor::of_switch(own.tool_end);
            }
            if lane.in_tool {
                path.position = Some(LOADED);
                path.direction = lane.direction();
            }
        }
    }
    path
}

/// The `[mmu_toolhead]` whose sensors sit at the head of `extruder`: the one the units feeding `extruder` name, when
/// every unit that names it feeds `extruder`. A toolhead shared by units of different heads ties its sensors to none.
pub fn toolhead_of<'a>(mmu: &'a MmuStatus, extruder: &str) -> Option<&'a str> {
    let toolhead = mmu.units.iter().find(|unit| unit.extruder == extruder).map(|unit| unit.toolhead.as_str())?;
    mmu.units.iter().filter(|unit| unit.toolhead == toolhead).all(|unit| unit.extruder == extruder).then_some(toolhead)
}

/// A sensor of Happy Hare's toolhead objects that no head can claim (see [`toolhead_of`]); it belongs to the machine.
pub fn is_untied_toolhead_sensor(mmu: &MmuStatus, object: &str) -> bool {
    // Only several units name an owner, and only several units can feed different heads.
    let Some((owner, sensor)) = object.strip_prefix(SWITCH_SENSOR).and_then(|name| name.rsplit_once(':')) else {
        return false;
    };
    if !matches!(sensor, EXTRUDER_ENTRY | TOOLHEAD) {
        return false;
    }
    let mut extruders = mmu.units.iter().filter(|unit| unit.toolhead == owner).map(|unit| unit.extruder.as_str());
    let first = extruders.next();
    first.is_some_and(|first| extruders.any(|extruder| extruder != first))
}

/// One unit or none reported: Happy Hare names its sensors without owner.
fn single_unit(mmu: &MmuStatus) -> bool {
    mmu.units.len() <= 1
}

fn unit_of(mmu: &MmuStatus, gate: usize) -> Option<&MmuUnit> {
    mmu.units.iter().find(|unit| unit.contains(gate))
}

fn gate_path(mmu: &MmuStatus, sensors: &[FilamentSensor], gate: usize) -> Option<LoadPath> {
    let state = mmu.gates.iter().find(|candidate| candidate.index == gate)?;
    let unit = unit_of(mmu, gate);
    let single = single_unit(mmu);
    let mut path = LoadPath::empty();
    path.entry = sensor(sensors, &format!("{GATE_ENTRY}_{gate}"));
    path.exit = sensor(sensors, &format!("{GATE_EXIT}_{gate}"));
    if let Some(unit) = unit {
        path.hub = owned_sensor(sensors, single, &unit.name, SHARED_EXIT);
        if toolhead_of(mmu, &unit.extruder) == Some(unit.toolhead.as_str()) {
            path.extruder = owned_sensor(sensors, single, &unit.toolhead, EXTRUDER_ENTRY);
            path.toolhead = owned_sensor(sensors, single, &unit.toolhead, TOOLHEAD);
        }
    }
    if usize::try_from(mmu.gate) == Ok(gate) {
        // The selected gate: what `mmu.sensors` lists wins, also sensors Happy Hare keeps out of Klipper's objects.
        let reported = &mmu.sensors;
        path.entry = generic_or(reported, GATE_ENTRY, path.entry);
        path.exit = generic_or(reported, GATE_EXIT, path.exit);
        path.hub = generic_or(reported, SHARED_EXIT, path.hub);
        path.extruder = generic_or(reported, EXTRUDER_ENTRY, path.extruder);
        path.toolhead = generic_or(reported, TOOLHEAD, path.toolhead);
        path.position = Some(mmu.filament_pos);
        path.direction = mmu.filament_direction.signum();
        path.distance = Some(mmu.filament_position);
        path.bowden_progress = mmu.bowden_progress;
        path.buffer = mmu.sync_feedback.as_ref().map(|feedback| happy_hare_side(feedback.state));
        return Some(path);
    }
    path.position = match state.availability {
        GateAvailability::Empty => None,
        GateAvailability::Available | GateAvailability::Buffered => Some(PARKED),
        GateAvailability::Unknown => Some(UNKNOWN),
    };
    Some(path)
}

/// The bypass: filament fed by hand past the gates; `mmu.sensors` holds the bypass's sensors while it is selected.
fn bypass_path(mmu: &MmuStatus) -> LoadPath {
    if !mmu.bypass_selected() {
        return LoadPath::empty();
    }
    let mut path = reported_shared_path(mmu);
    path.position = Some(mmu.filament_pos);
    path
}

/// Happy Hare with no gate selected (after a restart, before homing): no lane of its own, only the points it still
/// reports in `mmu.sensors` (hub, extruder entry, after the gears). The filament is drawn only where it is past the
/// hub, since the lane it came from is not known.
pub fn without_gate_path(mmu: &MmuStatus) -> LoadPath {
    let mut path = reported_shared_path(mmu);
    path.position = Some(mmu.filament_pos).filter(|position| *position > AT_HUB);
    path
}

/// The shared part of the path as `mmu.sensors` reports it for what Happy Hare has selected, without a position.
fn reported_shared_path(mmu: &MmuStatus) -> LoadPath {
    let mut path = LoadPath::empty();
    path.hub = generic(&mmu.sensors, SHARED_EXIT);
    path.extruder = generic(&mmu.sensors, EXTRUDER_ENTRY);
    path.toolhead = generic(&mmu.sensors, TOOLHEAD);
    path.direction = mmu.filament_direction.signum();
    path.distance = Some(mmu.filament_position);
    path.bowden_progress = mmu.bowden_progress;
    path
}

fn afc_lane_path(afc: &AfcStatus, name: &str) -> Option<LoadPath> {
    let lane = afc.lane(name)?;
    let own = afc.extruder(&lane.afc_extruder);
    let position = if lane.in_tool {
        Some(LOADED)
    } else if lane.at_hub {
        Some(AT_HUB)
    } else if lane.filled {
        Some(PARKED)
    } else {
        None
    };
    Some(LoadPath {
        entry: PathSensor::of_switch(lane.prep),
        exit: PathSensor::of_switch(lane.load),
        hub: PathSensor::of_switch(lane.hub.as_deref().and_then(|hub| afc.hub(hub)).and_then(|hub| hub.switch)),
        extruder: PathSensor::of_switch(own.and_then(|own| own.tool_start)),
        toolhead: PathSensor::of_switch(own.and_then(|own| own.tool_end)),
        buffer: lane.buffer.as_deref().and_then(|buffer| afc.buffer(buffer)).map(|buffer| afc_side(buffer.state)),
        position,
        direction: lane.direction(),
        distance: None,
        bowden_progress: None,
    })
}

fn happy_hare_side(state: SyncFeedbackState) -> BufferSide {
    match state {
        SyncFeedbackState::Tension => BufferSide::Tension,
        SyncFeedbackState::Neutral => BufferSide::Neutral,
        SyncFeedbackState::Compressed => BufferSide::Compression,
        SyncFeedbackState::Inactive | SyncFeedbackState::Disabled => BufferSide::Unknown,
    }
}

fn afc_side(state: BufferState) -> BufferSide {
    match state {
        BufferState::Advancing => BufferSide::Tension,
        BufferState::Trailing => BufferSide::Compression,
        BufferState::Unknown => BufferSide::Unknown,
    }
}

/// A sensor of the selected gate by its generic name in `mmu.sensors`.
fn generic(sensors: &[MmuSensor], name: &str) -> PathSensor {
    PathSensor::of_switch(sensors.iter().find(|sensor| sensor.name == name).and_then(|sensor| sensor.triggered))
}

/// [`generic`] where `mmu.sensors` lists the sensor (a disabled one is absent), else `fallback`.
fn generic_or(sensors: &[MmuSensor], name: &str, fallback: PathSensor) -> PathSensor {
    if sensors.iter().any(|sensor| sensor.name == name) {
        generic(sensors, name)
    } else {
        fallback
    }
}

/// A Klipper filament switch by its name; a disabled one counts as absent, as in `mmu.sensors`.
fn sensor(sensors: &[FilamentSensor], name: &str) -> PathSensor {
    let object = format!("{SWITCH_SENSOR}{name}");
    let found = sensors.iter().find(|sensor| sensor.object == object && sensor.enabled);
    PathSensor::of_switch(found.map(|sensor| sensor.detected))
}

/// A sensor owned by `owner`, which Happy Hare names only when it has several units.
fn owned_sensor(sensors: &[FilamentSensor], single_unit: bool, owner: &str, name: &str) -> PathSensor {
    if single_unit {
        sensor(sensors, name)
    } else {
        sensor(sensors, &format!("{owner}:{name}"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sources::afc::{AfcBuffer, AfcExtruder, AfcHub, AfcLane};
    use crate::sources::happy_hare::{MmuGate, SyncFeedback};

    fn switch(name: &str, detected: bool) -> FilamentSensor {
        FilamentSensor {
            object: format!("{SWITCH_SENSOR}{name}"),
            enabled: true,
            detected,
            ..FilamentSensor::default()
        }
    }

    fn unit(name: &str, first_gate: usize, extruder: &str, toolhead: &str) -> MmuUnit {
        MmuUnit {
            name: name.into(),
            first_gate,
            num_gates: 4,
            extruder: extruder.into(),
            toolhead: toolhead.into(),
            ..MmuUnit::default()
        }
    }

    /// Two units on two heads (`toolchanger-mmu`): gate 1 of the first is selected and loaded, gate 5 of the second
    /// holds filament, gate 6 is empty.
    fn two_heads() -> PrinterStatus {
        let gates = (0..8)
            .map(|index| MmuGate {
                index,
                availability: if index == 6 { GateAvailability::Empty } else { GateAvailability::Available },
                ..MmuGate::default()
            })
            .collect();
        let mmu = MmuStatus {
            version: "4.2.0".into(),
            enabled: true,
            gate: 1,
            filament_pos: 10,
            filament_direction: 1,
            filament_position: 512.0,
            sensors: vec![
                MmuSensor { name: "mmu_entry".into(), triggered: Some(true) },
                MmuSensor { name: "extruder".into(), triggered: Some(true) },
                MmuSensor { name: "toolhead".into(), triggered: None },
            ],
            sync_feedback: Some(SyncFeedback { state: SyncFeedbackState::Tension, bias: -1.0 }),
            units: vec![unit("ercf", 0, "extruder", "t0"), unit("tradrack", 4, "extruder1", "t1")],
            gates,
            ..MmuStatus::default()
        };
        let filament_sensors = vec![
            switch("mmu_entry_5", true),
            switch("mmu_exit_5", false),
            switch("tradrack:mmu_shared_exit", false),
            switch("t1:extruder", false),
            switch("t1:toolhead", false),
            switch("t0:extruder", true),
        ];
        PrinterStatus { mmu: Some(mmu), filament_sensors, ..PrinterStatus::default() }
    }

    #[test]
    fn the_selected_gate_takes_its_sensors_and_position_from_happy_hare() {
        let path = load_path(&two_heads(), &LaneId::Gate(1)).unwrap();

        assert_eq!((path.entry, path.exit, path.hub), (PathSensor::Triggered, PathSensor::Absent, PathSensor::Absent));
        assert_eq!((path.extruder, path.toolhead), (PathSensor::Triggered, PathSensor::Absent));
        assert_eq!((path.position, path.direction, path.distance), (Some(10), 1, Some(512.0)));
        assert_eq!(path.buffer, Some(BufferSide::Tension));
    }

    #[test]
    fn another_gate_reads_the_objects_of_its_gate_unit_and_toolhead() {
        let path = load_path(&two_heads(), &LaneId::Gate(5)).unwrap();

        assert_eq!((path.entry, path.exit, path.hub), (PathSensor::Triggered, PathSensor::Free, PathSensor::Free));
        assert_eq!((path.extruder, path.toolhead), (PathSensor::Free, PathSensor::Free));
        assert_eq!((path.position, path.buffer), (Some(PARKED), None));
        assert_eq!(load_path(&two_heads(), &LaneId::Gate(6)).unwrap().position, None);
    }

    #[test]
    fn with_one_unit_the_sensors_have_no_owner() {
        let mut status = two_heads();
        let mmu = status.mmu.as_mut().unwrap();
        mmu.units.truncate(1);
        mmu.units[0].num_gates = 8;
        status.filament_sensors =
            vec![switch("mmu_entry_5", true), switch("extruder", true), switch("toolhead", false)];

        let path = load_path(&status, &LaneId::Gate(5)).unwrap();

        assert_eq!(
            (path.entry, path.extruder, path.toolhead),
            (PathSensor::Triggered, PathSensor::Triggered, PathSensor::Free)
        );
    }

    /// The installer names one toolhead `default` for every unit; when the units feed different heads, its sensors
    /// cannot be placed on one and belong to the machine.
    #[test]
    fn a_toolhead_shared_by_units_of_two_heads_ties_its_sensors_to_none() {
        let mut status = two_heads();
        for unit in &mut status.mmu.as_mut().unwrap().units {
            unit.toolhead = "default".into();
        }
        status.filament_sensors.push(switch("default:toolhead", true));
        let mmu = status.happy_hare().unwrap();

        assert_eq!(load_path(&status, &LaneId::Gate(5)).unwrap().toolhead, PathSensor::Absent);
        assert_eq!(head_path(&status, "extruder1").toolhead, PathSensor::Absent);
        assert!(is_untied_toolhead_sensor(mmu, "filament_switch_sensor default:toolhead"));
        assert!(!is_untied_toolhead_sensor(mmu, "filament_switch_sensor mmu_entry_5"));
    }

    #[test]
    fn the_head_path_has_the_sensors_of_its_toolhead_and_the_filament_of_its_unit() {
        let status = two_heads();

        let first = head_path(&status, "extruder");
        let second = head_path(&status, "extruder1");

        assert_eq!((first.extruder, first.position), (PathSensor::Triggered, Some(10)));
        assert_eq!((second.extruder, second.toolhead, second.position), (PathSensor::Free, PathSensor::Free, None));
        assert_eq!(first.entry, PathSensor::Absent);
    }

    #[test]
    fn without_a_gate_the_path_has_the_reported_head_sensors_and_no_lane() {
        let mut status = two_heads();
        let mmu = status.mmu.as_mut().unwrap();
        mmu.gate = -1;

        let loaded = without_gate_path(mmu);
        mmu.filament_pos = AT_HUB;
        let at_hub = without_gate_path(mmu);

        assert_eq!((loaded.entry, loaded.exit), (PathSensor::Absent, PathSensor::Absent));
        assert_eq!((loaded.extruder, loaded.toolhead), (PathSensor::Triggered, PathSensor::Absent));
        assert_eq!((loaded.position, at_hub.position), (Some(LOADED), None));
    }

    fn box_turtle() -> PrinterStatus {
        let lane = |name: &str, filled: bool, in_tool: bool| AfcLane {
            name: name.into(),
            afc_extruder: "extruder".into(),
            extruder: "extruder".into(),
            hub: Some("Turtle_1".into()),
            buffer: Some("TN".into()),
            filled,
            prep: Some(filled),
            load: Some(filled),
            at_hub: in_tool,
            in_tool,
            status: if in_tool { "Tooled" } else { "Loaded" }.into(),
            ..AfcLane::default()
        };
        let afc = AfcStatus {
            lanes: vec![lane("lane1", true, false), lane("lane2", true, true), lane("lane3", false, false)],
            hubs: vec![AfcHub { name: "Turtle_1".into(), switch: Some(true) }],
            buffers: vec![AfcBuffer { name: "TN".into(), state: BufferState::Advancing }],
            extruders: vec![AfcExtruder { name: "extruder".into(), tool_start: Some(true), tool_end: None }],
            current_load: Some("lane2".into()),
            ..AfcStatus::default()
        };
        PrinterStatus { afc: Some(afc), ..PrinterStatus::default() }
    }

    #[test]
    fn an_afc_lane_reads_its_switches_hub_buffer_and_toolhead() {
        let status = box_turtle();

        let loaded = load_path(&status, &LaneId::Afc("lane2".into())).unwrap();
        let parked = load_path(&status, &LaneId::Afc("lane1".into())).unwrap();
        let empty = load_path(&status, &LaneId::Afc("lane3".into())).unwrap();

        assert_eq!(
            (loaded.entry, loaded.exit, loaded.hub),
            (PathSensor::Triggered, PathSensor::Triggered, PathSensor::Triggered)
        );
        assert_eq!((loaded.extruder, loaded.toolhead), (PathSensor::Triggered, PathSensor::Absent));
        assert_eq!((loaded.position, loaded.buffer), (Some(LOADED), Some(BufferSide::Tension)));
        assert_eq!(parked.position, Some(PARKED));
        assert_eq!((empty.entry, empty.position), (PathSensor::Free, None));
        assert!(load_path(&status, &LaneId::Afc("lane9".into())).is_none());
    }

    #[test]
    fn the_head_path_of_afc_shows_the_lane_in_the_tool() {
        let path = head_path(&box_turtle(), "extruder");

        assert_eq!((path.extruder, path.position), (PathSensor::Triggered, Some(LOADED)));
    }
}
