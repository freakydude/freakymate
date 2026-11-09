//! The filament sensors of the MMU and the filament path they draw.

use serde_json::Value;

const COMPRESSION_SENSOR: &str = "filament_compression";
const TENSION_SENSOR: &str = "filament_tension";
const PROPORTIONAL_SENSOR: &str = "filament_proportional";
/// The sensors of the sync-feedback buffer.
const BUFFER_SENSORS: [&str; 3] = [COMPRESSION_SENSOR, TENSION_SENSOR, PROPORTIONAL_SENSOR];

/// Happy Hare 4.1 still lists the Happy Hare 3 names of the gate sensors in `mmu.sensors` as aliases.
const LEGACY_SENSOR_NAMES: [&str; 3] = ["mmu_pre_gate", "mmu_gear", "mmu_gate"];

/// A filament sensor of the MMU as reported in `mmu.sensors`; `triggered` is `None` when the sensor is disabled.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MmuSensor {
    pub name: String,
    pub triggered: Option<bool>,
}

/// The sensor where the filament enters the gate from the spool.
const GATE_ENTRY_SENSOR: &str = "mmu_entry";
/// Sensors where the filament leaves the gate, in order of preference: the gate's own exit, then a shared exit.
const GATE_EXIT_SENSORS: [&str; 2] = ["mmu_exit", "mmu_shared_exit"];
const EXTRUDER_PATH_SENSOR: &str = "extruder";
const TOOLHEAD_PATH_SENSOR: &str = "toolhead";

/// A sensor drawn on the filament path.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PathSensor {
    /// Not fitted or disabled: not drawn.
    Absent,
    Free,
    Triggered,
}

impl PathSensor {
    fn of(sensors: &[MmuSensor], name: &str) -> Self {
        match sensors.iter().find(|sensor| sensor.name == name).and_then(|sensor| sensor.triggered) {
            Some(true) => Self::Triggered,
            Some(false) => Self::Free,
            None => Self::Absent,
        }
    }
}

/// The way of the filament from the gate to the nozzle, as Happy Hare's console draws it (`log_visual`): the
/// position on the way (`filament_pos`, -1 unknown, 0 parked in the gate up to 10 loaded to the nozzle, see
/// `FILAMENT_POS_*` in `mmu_constants.py`), the direction and the sensors along it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FilamentPath {
    pub position: i32,
    pub direction: i32,
    pub distance: f64,
    /// 0 to 1 of the bowden while Happy Hare moves through it (`bowden_progress`), `None` otherwise.
    pub bowden_progress: Option<f64>,
    pub gate_entry_sensor: PathSensor,
    pub gate_exit_sensor: PathSensor,
    pub extruder_sensor: PathSensor,
    pub toolhead_sensor: PathSensor,
}

impl FilamentPath {
    pub(super) fn new(
        sensors: &[MmuSensor],
        position: i32,
        direction: i32,
        distance: f64,
        bowden_progress: Option<f64>,
    ) -> Self {
        let gate_exit_sensor = GATE_EXIT_SENSORS
            .into_iter()
            .map(|name| PathSensor::of(sensors, name))
            .find(|sensor| *sensor != PathSensor::Absent)
            .unwrap_or(PathSensor::Absent);
        Self {
            position,
            direction,
            distance,
            bowden_progress,
            gate_entry_sensor: PathSensor::of(sensors, GATE_ENTRY_SENSOR),
            gate_exit_sensor,
            extruder_sensor: PathSensor::of(sensors, EXTRUDER_PATH_SENSOR),
            toolhead_sensor: PathSensor::of(sensors, TOOLHEAD_PATH_SENSOR),
        }
    }
}

/// Names of the sensors the path draws, so they are not listed a second time.
pub(super) fn path_sensor_names(sensors: &[MmuSensor]) -> Vec<&'static str> {
    let exit = GATE_EXIT_SENSORS.into_iter().find(|name| PathSensor::of(sensors, name) != PathSensor::Absent);
    [GATE_ENTRY_SENSOR].into_iter().chain(exit).chain([EXTRUDER_PATH_SENSOR, TOOLHEAD_PATH_SENSOR]).collect()
}

/// The sensors to list on their own: without those the filament path draws, and without the buffer sensors when
/// `has_buffer` says that the buffer (sync feedback) shows them.
pub(super) fn listed_sensors(sensors: &[MmuSensor], has_buffer: bool) -> Vec<&MmuSensor> {
    let on_path = path_sensor_names(sensors);
    sensors
        .iter()
        .filter(|sensor| !(has_buffer && BUFFER_SENSORS.contains(&sensor.name.as_str())))
        .filter(|sensor| !on_path.contains(&sensor.name.as_str()))
        .collect()
}

pub(super) fn has_proportional_sensor(sensors: &[MmuSensor]) -> bool {
    sensors.iter().any(|sensor| sensor.name == PROPORTIONAL_SENSOR && sensor.triggered.is_some())
}

/// A Klipper filament sensor that Happy Hare creates and the MMU view shows: those of the gates and the buffer, and
/// `extruder` and `toolhead`, which the filament path draws. Others (a runout sensor at the spool, a motion sensor)
/// belong to the printer. `object` is the Klipper object name, for example `filament_switch_sensor mmu_entry_0`.
pub fn is_shown_by_mmu(object: &str) -> bool {
    let name = object.split_once(' ').map_or(object, |(_, name)| name);
    let name = name.rsplit_once(':').map_or(name, |(_, name)| name);
    name.starts_with("mmu_")
        || matches!(
            name,
            COMPRESSION_SENSOR | TENSION_SENSOR | PROPORTIONAL_SENSOR | EXTRUDER_PATH_SENSOR | TOOLHEAD_PATH_SENSOR
        )
}

pub(super) fn read_sensors(mmu: &Value) -> Vec<MmuSensor> {
    mmu.get("sensors")
        .and_then(Value::as_object)
        .map(|sensors| {
            sensors
                .iter()
                .filter(|(name, _)| !LEGACY_SENSOR_NAMES.contains(&name.as_str()))
                .map(|(name, state)| MmuSensor { name: name.clone(), triggered: state.as_bool() })
                .collect()
        })
        .unwrap_or_default()
}
