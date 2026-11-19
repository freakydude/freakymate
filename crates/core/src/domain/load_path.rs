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
use crate::sources::PrinterStatus;
use crate::sources::afc::{AfcStatus, BufferState};
use crate::sources::happy_hare::{GateAvailability, MmuSensor, MmuStatus, MmuUnit, PathSensor, SyncFeedbackState};
use crate::sources::klipper::FilamentSensor;

const SWITCH_SENSOR: &str = "filament_switch_sensor ";
const GATE_ENTRY: &str = "mmu_entry";
const GATE_EXIT: &str = "mmu_exit";
const SHARED_EXIT: &str = "mmu_shared_exit";
const EXTRUDER_ENTRY: &str = "extruder";
const TOOLHEAD: &str = "toolhead";
const ENCODER: &str = "encoder";

/// Where the filament is, on Happy Hare's scale (`filament_pos`, `mmu_constants.py` L84-95): -1 unknown, 0 parked in
/// the lane, 1 at the hub, 2 to 4 in the bowden, 5 at the extruder sensor, 7 in the extruder entry, 8 at the toolhead
/// sensor, 9 in the extruder, 10 loaded to the nozzle.
pub const UNKNOWN: i32 = -1;
pub const PARKED: i32 = 0;
pub const AT_HUB: i32 = 1;
pub const LOADED: i32 = 10;
const BOWDEN_START: i32 = 2;
const BOWDEN_END: i32 = 4;
const AT_GEARS: i32 = 6;
const IN_GEARS: i32 = 7;
const AT_TOOLHEAD_SENSOR: i32 = 8;
const PAST_TOOLHEAD_SENSOR: i32 = 9;

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
    /// Happy Hare's encoder at the unit, before the bowden (`gate_endstop_to_encoder` in `mmu_parameters.cfg`; one
    /// per unit, reported for the selected gate). It has no presence switch: triggered means it counts movement
    /// (`unit/mmu_encoder.py` L425-468 at c176ecc).
    pub encoder: PathSensor,
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
    /// A way without sensors and without filament.
    pub fn empty() -> Self {
        Self {
            entry: PathSensor::Absent,
            exit: PathSensor::Absent,
            hub: PathSensor::Absent,
            encoder: PathSensor::Absent,
            extruder: PathSensor::Absent,
            toolhead: PathSensor::Absent,
            buffer: None,
            position: None,
            direction: 0,
            distance: None,
            bowden_progress: None,
        }
    }

    /// The filament is loaded to the nozzle: nothing about the way changes until the next unload. The direction does
    /// not count, as Happy Hare keeps the one of the last move.
    pub fn is_settled(&self) -> bool {
        self.position == Some(LOADED)
    }

    /// What is drawn of the way, in the order the filament passes it: the fitted sensors, the stretches between them
    /// and how far the filament fills each, the buffer, the extruder gears and the nozzle.
    pub fn parts(&self, scope: PathScope) -> Vec<PathPart> {
        let position = self.position.unwrap_or(UNKNOWN);
        let tube = |stretch: Stretch| PathPart::Tube { stretch, fill: stretch.fill(position, self.bowden_progress) };
        let sensor = |sensor: PathSensor| match sensor {
            PathSensor::Absent => None,
            PathSensor::Free => Some(PathPart::Sensor { triggered: false }),
            PathSensor::Triggered => Some(PathPart::Sensor { triggered: true }),
        };
        let mut parts = Vec::new();
        if scope == PathScope::Whole {
            parts.extend(sensor(self.entry));
            parts.push(tube(Stretch::Lane));
            parts.extend(sensor(self.exit));
            // Where the lanes of a unit meet, when the printer has a sensor there.
            if self.hub != PathSensor::Absent {
                parts.push(tube(Stretch::Hub));
                parts.extend(sensor(self.hub));
            }
            match self.encoder {
                PathSensor::Absent => {}
                PathSensor::Free => parts.push(PathPart::Encoder { moving: false }),
                PathSensor::Triggered => parts.push(PathPart::Encoder { moving: true }),
            }
            parts.extend(self.buffer.map(PathPart::Buffer));
            parts.push(tube(Stretch::Bowden));
        }
        // Between two points lies one stretch. Without an extruder sensor the bowden runs up to the gears (the head
        // alone keeps the piece that leads to them); without a toolhead sensor one stretch runs from the gears to the
        // nozzle.
        parts.extend(sensor(self.extruder));
        if self.extruder != PathSensor::Absent || scope == PathScope::HeadOnly {
            parts.push(tube(Stretch::ExtruderEntry));
        }
        parts.push(PathPart::Gears { reached: position >= AT_GEARS });
        if self.toolhead == PathSensor::Absent {
            parts.push(tube(Stretch::GearsToNozzle));
        } else {
            parts.push(tube(Stretch::Extruder));
            parts.extend(sensor(self.toolhead));
            parts.push(tube(Stretch::Hotend));
        }
        parts.push(PathPart::Nozzle { reached: position >= LOADED });
        parts
    }
}

/// How much of the way is drawn.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PathScope {
    /// From the lane to the nozzle.
    Whole,
    /// The part at the head only: its sensors, the extruder gears and the nozzle.
    HeadOnly,
}

/// One drawn piece of the way.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum PathPart {
    Sensor {
        triggered: bool,
    },
    /// A stretch of tube, filled from the lane's side.
    Tube {
        stretch: Stretch,
        fill: Fill,
    },
    /// The encoder at the unit; `moving` while it counts filament passing.
    Encoder {
        moving: bool,
    },
    Buffer(BufferSide),
    /// The extruder gears; `reached` once the filament is at them.
    Gears {
        reached: bool,
    },
    Nozzle {
        reached: bool,
    },
}

/// How far a stretch of tube is filled. The scale has no measure inside a stretch, so a position between two points
/// is only a stage ("somewhere in it"); how much of the drawing a stage fills is for the display to decide.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Fill {
    Empty,
    Full,
    Stage(Stage),
    /// How far a move has come, 0 to 1, as Happy Hare reports it in the bowden.
    Measured(f64),
}

/// How far into a stretch the filament is, from the first reach to nearly through.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Stage {
    Started,
    Early,
    Halfway,
    Late,
}

/// The stretches of tube between the measuring points.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Stretch {
    /// Inside the lane, up to its exit.
    Lane,
    /// From the lane's exit to where the lanes meet.
    Hub,
    Bowden,
    /// From the extruder sensor (the end of the bowden) to the extruder gears.
    ExtruderEntry,
    /// From the gears to the toolhead sensor.
    Extruder,
    /// From the toolhead sensor to the nozzle.
    Hotend,
    /// From the gears to the nozzle, where no toolhead sensor divides the way.
    GearsToNozzle,
}

impl Stretch {
    /// The fill at `position`: a position between two points is a stage of the stretch; in the bowden Happy Hare
    /// reports how far a move has come.
    fn fill(self, position: i32, bowden_progress: Option<f64>) -> Fill {
        if position < PARKED {
            return Fill::Empty;
        }
        match self {
            Self::Lane if position >= AT_HUB => Fill::Full,
            Self::Lane => Fill::Stage(Stage::Late),
            Self::Hub if position >= AT_HUB => Fill::Full,
            Self::Bowden if position >= BOWDEN_END => Fill::Full,
            Self::Bowden if position >= BOWDEN_START => match bowden_progress {
                Some(progress) => Fill::Measured(progress.clamp(0.0, 1.0)),
                None if position == BOWDEN_START => Fill::Stage(Stage::Started),
                None => Fill::Stage(Stage::Halfway),
            },
            Self::ExtruderEntry if position >= AT_GEARS => Fill::Full,
            Self::Extruder if position >= AT_TOOLHEAD_SENSOR => Fill::Full,
            // The gears have pulled the filament in.
            Self::Extruder if position == IN_GEARS => Fill::Stage(Stage::Early),
            Self::Hotend if position >= LOADED => Fill::Full,
            Self::Hotend if position == PAST_TOOLHEAD_SENSOR => Fill::Stage(Stage::Halfway),
            Self::GearsToNozzle if position >= LOADED => Fill::Full,
            Self::GearsToNozzle if position >= AT_TOOLHEAD_SENSOR => Fill::Stage(Stage::Late),
            Self::GearsToNozzle if position == IN_GEARS => Fill::Stage(Stage::Early),
            _ => Fill::Empty,
        }
    }
}

/// The gate's own sensors contradict what Happy Hare's `gate_status` says, by the rule Happy Hare applies itself
/// (`validate_gate_status`, `extras/mmu/mmu_gate_maps.py` L287-307 @ c4c767a): a sensor detects filament in an empty
/// gate, or the entry sensor sees none in a gate with filament, whether from the spool (1) or parked in the gate after
/// an unload (2, "from buffer"). Only the entry sensor can tell "no filament": the exit sensor is free while filament
/// waits in the gate. Happy Hare corrects this only at start-up and before a load, so it can persist meanwhile.
/// A disabled or missing sensor and an unknown status contradict nothing.
pub fn sensor_contradicts_gate(path: &LoadPath, availability: GateAvailability) -> bool {
    match availability {
        GateAvailability::Empty => path.entry == PathSensor::Triggered || path.exit == PathSensor::Triggered,
        GateAvailability::Available | GateAvailability::Buffered => path.entry == PathSensor::Free,
        GateAvailability::Unknown => false,
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
        path.encoder = generic(reported, ENCODER);
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
        encoder: PathSensor::Absent,
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
    if sensors.iter().any(|sensor| sensor.name == name) { generic(sensors, name) } else { fallback }
}

/// A Klipper filament switch by its name; a disabled one counts as absent, as in `mmu.sensors`.
fn sensor(sensors: &[FilamentSensor], name: &str) -> PathSensor {
    let object = format!("{SWITCH_SENSOR}{name}");
    let found = sensors.iter().find(|sensor| sensor.object == object && sensor.enabled);
    PathSensor::of_switch(found.map(|sensor| sensor.detected))
}

/// A sensor owned by `owner`, which Happy Hare names only when it has several units.
fn owned_sensor(sensors: &[FilamentSensor], single_unit: bool, owner: &str, name: &str) -> PathSensor {
    if single_unit { sensor(sensors, name) } else { sensor(sensors, &format!("{owner}:{name}")) }
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
            sync_feedback: Some(SyncFeedback {
                state: SyncFeedbackState::Tension,
                bias: -1.0,
                ..SyncFeedback::default()
            }),
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
    fn a_gate_sensor_against_the_gate_status_is_a_contradiction() {
        let path = |entry, exit| LoadPath { entry, exit, ..LoadPath::empty() };
        let (free, triggered, absent) = (PathSensor::Free, PathSensor::Triggered, PathSensor::Absent);

        assert!(sensor_contradicts_gate(&path(triggered, absent), GateAvailability::Empty));
        assert!(sensor_contradicts_gate(&path(free, triggered), GateAvailability::Empty));
        assert!(sensor_contradicts_gate(&path(free, free), GateAvailability::Available));
        assert!(
            sensor_contradicts_gate(&path(free, absent), GateAvailability::Buffered),
            "filament parked in the gate"
        );
        assert!(!sensor_contradicts_gate(&path(triggered, free), GateAvailability::Buffered), "exit free after unload");
        assert!(!sensor_contradicts_gate(&path(free, free), GateAvailability::Empty));
        assert!(!sensor_contradicts_gate(&path(triggered, free), GateAvailability::Available), "exit may be free");
        assert!(!sensor_contradicts_gate(&path(absent, free), GateAvailability::Available), "only entry says none");
        assert!(!sensor_contradicts_gate(&path(triggered, triggered), GateAvailability::Unknown));
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

    fn path_at(position: i32) -> LoadPath {
        LoadPath { entry: PathSensor::Triggered, exit: PathSensor::Free, position: Some(position), ..LoadPath::empty() }
    }

    fn with_head_sensors(mut path: LoadPath) -> LoadPath {
        path.extruder = PathSensor::Free;
        path.toolhead = PathSensor::Free;
        path
    }

    fn fill_of(path: &LoadPath, wanted: Stretch) -> Fill {
        path.parts(PathScope::Whole)
            .into_iter()
            .find_map(|part| match part {
                PathPart::Tube { stretch, fill } if stretch == wanted => Some(fill),
                _ => None,
            })
            .unwrap()
    }

    #[test]
    fn the_parts_follow_the_way_and_leave_out_what_is_not_fitted() {
        let mut path = path_at(LOADED);
        path.toolhead = PathSensor::Triggered;

        let parts = path.parts(PathScope::Whole);

        assert_eq!(parts.first(), Some(&PathPart::Sensor { triggered: true }));
        assert_eq!(parts.get(2), Some(&PathPart::Sensor { triggered: false }));
        assert_eq!(parts.last(), Some(&PathPart::Nozzle { reached: true }));
        assert!(!parts.iter().any(|part| matches!(part, PathPart::Tube { stretch: Stretch::Hub, .. })));
        assert!(!parts.iter().any(|part| matches!(part, PathPart::Buffer(_))));
        assert_eq!(parts.iter().filter(|part| matches!(part, PathPart::Sensor { .. })).count(), 3);
    }

    #[test]
    fn hub_and_buffer_are_parts_where_the_lane_has_them() {
        let mut path = path_at(PARKED);
        path.hub = PathSensor::Free;
        path.buffer = Some(BufferSide::Neutral);

        let parts = path.parts(PathScope::Whole);
        let hub = parts.iter().position(|part| matches!(part, PathPart::Tube { stretch: Stretch::Hub, .. })).unwrap();
        let buffer = parts.iter().position(|part| *part == PathPart::Buffer(BufferSide::Neutral)).unwrap();
        let bowden =
            parts.iter().position(|part| matches!(part, PathPart::Tube { stretch: Stretch::Bowden, .. })).unwrap();

        assert!(hub < buffer && buffer < bowden);
    }

    #[test]
    fn the_encoder_stands_between_the_lane_and_the_buffer() {
        let mut path = path_at(LOADED);
        path.encoder = PathSensor::Triggered;
        path.buffer = Some(BufferSide::Neutral);

        let parts = path.parts(PathScope::Whole);
        let encoder = parts.iter().position(|part| *part == PathPart::Encoder { moving: true }).unwrap();
        let buffer = parts.iter().position(|part| matches!(part, PathPart::Buffer(_))).unwrap();

        assert!(encoder < buffer);
        assert!(!path.parts(PathScope::HeadOnly).iter().any(|part| matches!(part, PathPart::Encoder { .. })));
    }

    #[test]
    fn the_head_only_scope_starts_at_the_extruder_sensor() {
        let mut path = path_at(LOADED);
        path.extruder = PathSensor::Triggered;

        let parts = path.parts(PathScope::HeadOnly);

        assert_eq!(parts.first(), Some(&PathPart::Sensor { triggered: true }));
        assert!(matches!(parts.get(1), Some(PathPart::Tube { stretch: Stretch::ExtruderEntry, .. })));
        assert_eq!(parts.get(2), Some(&PathPart::Gears { reached: true }));
        assert!(!parts.iter().any(|part| matches!(part, PathPart::Tube { stretch: Stretch::Bowden, .. })));
    }

    #[test]
    fn stretches_merge_where_no_sensor_divides_them() {
        let path = path_at(IN_GEARS);
        let parts = path.parts(PathScope::Whole);
        let tubes: Vec<Stretch> = parts
            .iter()
            .filter_map(|part| match part {
                PathPart::Tube { stretch, .. } => Some(*stretch),
                _ => None,
            })
            .collect();

        assert_eq!(tubes, [Stretch::Lane, Stretch::Bowden, Stretch::GearsToNozzle]);
        assert_eq!(fill_of(&path, Stretch::GearsToNozzle), Fill::Stage(Stage::Early));
        assert_eq!(fill_of(&path_at(LOADED), Stretch::GearsToNozzle), Fill::Full);
        assert!(matches!(
            path.parts(PathScope::HeadOnly).first(),
            Some(PathPart::Tube { stretch: Stretch::ExtruderEntry, .. })
        ));
    }

    #[test]
    fn the_stretches_fill_with_the_position() {
        assert_eq!(fill_of(&path_at(PARKED), Stretch::Lane), Fill::Stage(Stage::Late));
        assert_eq!(fill_of(&path_at(PARKED), Stretch::Bowden), Fill::Empty);
        assert_eq!(fill_of(&path_at(AT_HUB), Stretch::Lane), Fill::Full);
        assert_eq!(fill_of(&path_at(BOWDEN_START), Stretch::Bowden), Fill::Stage(Stage::Started));
        assert_eq!(fill_of(&path_at(BOWDEN_START + 1), Stretch::Bowden), Fill::Stage(Stage::Halfway));
        assert_eq!(fill_of(&path_at(BOWDEN_END), Stretch::Bowden), Fill::Full);
        assert_eq!(fill_of(&with_head_sensors(path_at(AT_GEARS - 1)), Stretch::ExtruderEntry), Fill::Empty);
        assert_eq!(fill_of(&with_head_sensors(path_at(AT_GEARS)), Stretch::ExtruderEntry), Fill::Full);
        assert_eq!(fill_of(&with_head_sensors(path_at(IN_GEARS)), Stretch::Extruder), Fill::Stage(Stage::Early));
        assert_eq!(
            fill_of(&with_head_sensors(path_at(PAST_TOOLHEAD_SENSOR)), Stretch::Hotend),
            Fill::Stage(Stage::Halfway)
        );
        assert_eq!(fill_of(&with_head_sensors(path_at(LOADED)), Stretch::Hotend), Fill::Full);
    }

    #[test]
    fn the_bowden_fills_as_far_as_happy_hare_reports_the_move() {
        let mut path = path_at(BOWDEN_START + 1);
        path.bowden_progress = Some(0.8);

        assert_eq!(fill_of(&path, Stretch::Bowden), Fill::Measured(0.8));
    }

    #[test]
    fn nothing_is_filled_and_reached_without_a_position() {
        let path = LoadPath::empty();

        assert_eq!(fill_of(&path, Stretch::Lane), Fill::Empty);
        assert!(path.parts(PathScope::Whole).contains(&PathPart::Gears { reached: false }));
        assert!(!path.is_settled());
    }

    #[test]
    fn a_path_is_settled_once_loaded_whatever_the_last_direction() {
        let mut path = path_at(LOADED);
        path.direction = 1;

        assert!(path.is_settled());
        assert!(!path_at(PAST_TOOLHEAD_SENSOR).is_settled());
        assert!(!path_at(BOWDEN_END).is_settled());
    }
}
