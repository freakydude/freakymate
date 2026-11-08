//! Klipper's filament sensors (`filament_switch_sensor`, `filament_motion_sensor`). Which of them belong to the
//! printer and which to an MMU is decided in [`crate::domain::filament`].

use super::status::label;
use crate::sources::fields::flag;
use crate::sources::objects::ObjectStore;

#[derive(Debug, Clone, PartialEq, Default)]
pub struct FilamentSensor {
    pub object: String,
    pub label: String,
    pub enabled: bool,
    pub detected: bool,
}

pub(super) fn is_filament_sensor(kind: &str) -> bool {
    matches!(kind, "filament_switch_sensor" | "filament_motion_sensor")
}

pub(super) fn read_filament_sensors(store: &ObjectStore) -> Vec<FilamentSensor> {
    store
        .names()
        .filter(|name| name.split_once(' ').is_some_and(|(kind, _)| is_filament_sensor(kind)))
        .filter_map(|name| {
            store.object(name).map(|value| FilamentSensor {
                object: name.to_owned(),
                label: label(name),
                enabled: flag(value, "enabled"),
                detected: flag(value, "filament_detected"),
            })
        })
        .collect()
}
