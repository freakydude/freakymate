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
/// Where the filament leaves the gate, and where the gates of a unit meet.
const GATE_EXIT_SENSOR: &str = "mmu_exit";
const SHARED_EXIT_SENSOR: &str = "mmu_shared_exit";
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
    /// A switch that is triggered or free; `None` for one that is not fitted or disabled.
    pub fn of_switch(triggered: Option<bool>) -> Self {
        match triggered {
            Some(true) => Self::Triggered,
            Some(false) => Self::Free,
            None => Self::Absent,
        }
    }
}

/// Names of the sensors the path draws, so they are not listed a second time.
/// The load path (`domain::load_path`) draws the gate's entry and exit, the shared exit as the hub, and the sensors
/// before and after the extruder gears.
const PATH_SENSORS: [&str; 5] =
    [GATE_ENTRY_SENSOR, GATE_EXIT_SENSOR, SHARED_EXIT_SENSOR, EXTRUDER_PATH_SENSOR, TOOLHEAD_PATH_SENSOR];

/// The sensors to list on their own: without those the filament path draws, and without the buffer sensors when
/// `has_buffer` says that the buffer (sync feedback) shows them.
pub(super) fn listed_sensors(sensors: &[MmuSensor], has_buffer: bool) -> Vec<&MmuSensor> {
    sensors
        .iter()
        .filter(|sensor| !(has_buffer && BUFFER_SENSORS.contains(&sensor.name.as_str())))
        .filter(|sensor| !PATH_SENSORS.contains(&sensor.name.as_str()))
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
