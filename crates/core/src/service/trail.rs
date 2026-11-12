//! The course of the printer in the verbose log (the setting `verbose_log`): the changes of the objects that decide what
//! FreakyMate shows and does, the console and Moonraker's notifications, so a bug report shows what happened in which
//! order. Measurements that change all the time (temperatures, positions, speeds) are left out. Nothing here is
//! computed unless the log asks for debug records.

use serde_json::{Map, Value};

/// Longest text of a value in the log; a longer one is cut, so one big answer does not fill the log.
const MAX_VALUE_TEXT: usize = 400;

/// Which fields of an object the verbose log follows.
enum Fields {
    All,
    Only(&'static [&'static str]),
    AllBut(&'static [&'static str]),
}

/// The objects followed in the verbose log; a name ending in a space stands for every object it starts (`tool T0`).
/// The fields of `mmu` and AFC come from Happy Hare's `mmu.py` `get_status` and AFC's `get_status` methods; the left
/// out ones change with every move.
const WATCHED: &[(&str, Fields)] = &[
    ("print_stats", Fields::Only(&["state", "filename", "message", "info"])),
    ("idle_timeout", Fields::Only(&["state"])),
    ("pause_resume", Fields::All),
    ("toolhead", Fields::Only(&["homed_axes", "extruder"])),
    ("virtual_sdcard", Fields::Only(&["is_active", "file_path"])),
    ("configfile", Fields::Only(&["save_config_pending"])),
    ("exclude_object", Fields::Only(&["excluded_objects"])),
    ("bed_mesh", Fields::Only(&["profile_name"])),
    ("manual_probe", Fields::All),
    ("bed_screws", Fields::All),
    ("screws_tilt_adjust", Fields::All),
    ("z_tilt", Fields::All),
    ("quad_gantry_level", Fields::All),
    (
        "mmu",
        Fields::AllBut(&[
            "filament_position",
            "bowden_progress",
            "encoder",
            "flowguard",
            "sync_feedback_bias_modelled",
            "sync_feedback_bias_raw",
            "sync_feedback_flow_rate",
        ]),
    ),
    ("AFC", Fields::All),
    ("AFC_stepper ", Fields::All),
    ("AFC_lane ", Fields::All),
    ("AFC_extruder ", Fields::All),
    ("AFC_hub ", Fields::All),
    ("AFC_buffer ", Fields::AllBut(&["distance_to_fault", "fault_timer"])),
    ("toolchanger", Fields::All),
    ("tool ", Fields::All),
    ("filament_switch_sensor ", Fields::Only(&["filament_detected", "enabled"])),
    ("filament_motion_sensor ", Fields::Only(&["filament_detected", "enabled"])),
    ("led_effect ", Fields::Only(&["enabled"])),
];

/// Moonraker notifications the verbose log notes with their parameters; the frequent ones (status, process
/// statistics, sensors, update output) stay out.
const NOTED_NOTIFICATIONS: &[&str] = &[
    "notify_klippy_ready",
    "notify_klippy_shutdown",
    "notify_klippy_disconnected",
    "notify_active_spool_set",
    "notify_spoolman_status_changed",
    "notify_power_changed",
    "notify_filelist_changed",
    "notify_history_changed",
];

/// Notes the followed fields of a `notify_status_update`; Klipper only sends what changed.
pub(super) fn status_changes(changes: &Value) {
    if !log::log_enabled!(log::Level::Debug) {
        return;
    }
    let Some(objects) = changes.as_object() else { return };
    for (name, fields) in objects {
        if let Some(followed) = followed_fields(name, fields) {
            log::debug!("status {name}: {}", brief(&Value::Object(followed)));
        }
    }
}

/// Notes a notification from Moonraker; every one goes to the trace level.
pub(super) fn notification(method: &str, params: &Value) {
    log::trace!("{method} {params}");
    if NOTED_NOTIFICATIONS.contains(&method) {
        log::debug!("{method} {}", brief(params));
    }
}

/// Notes a line of Klipper's console, as answers and messages of the macros explain what the printer did.
pub(super) fn console_response(message: &str) {
    log::debug!("console: {}", message.trim_end());
}

/// The followed fields of an object's changes; `None` for an object not followed or without such changes.
fn followed_fields(name: &str, changes: &Value) -> Option<Map<String, Value>> {
    let (_, fields) =
        WATCHED
            .iter()
            .find(|(watched, _)| if watched.ends_with(' ') { name.starts_with(watched) } else { name == *watched })?;
    let followed: Map<String, Value> = changes
        .as_object()?
        .iter()
        .filter(|(field, _)| match fields {
            Fields::All => true,
            Fields::Only(names) => names.contains(&field.as_str()),
            Fields::AllBut(names) => !names.contains(&field.as_str()),
        })
        .map(|(field, value)| (field.clone(), value.clone()))
        .collect();
    (!followed.is_empty()).then_some(followed)
}

/// Compact JSON text of a value, cut at [`MAX_VALUE_TEXT`] characters.
pub(super) fn brief(value: &Value) -> String {
    let text = value.to_string();
    match text.char_indices().nth(MAX_VALUE_TEXT) {
        Some((end, _)) => format!("{}…", &text[..end]),
        None => text,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn follows_state_fields_and_leaves_out_measurements() {
        let changes = json!({ "print_stats": { "state": "printing", "print_duration": 12.5 } });
        let followed = followed_fields("print_stats", &changes["print_stats"]).unwrap();
        assert_eq!(Value::Object(followed), json!({ "state": "printing" }));
        let mmu = json!({ "action": "Loading", "filament_position": 312.4 });
        assert_eq!(Value::Object(followed_fields("mmu", &mmu).unwrap()), json!({ "action": "Loading" }));
    }

    #[test]
    fn ignores_objects_not_followed_and_changes_without_followed_fields() {
        assert!(followed_fields("extruder", &json!({ "temperature": 210.0 })).is_none());
        assert!(followed_fields("toolhead", &json!({ "position": [1, 2, 3, 4] })).is_none());
    }

    #[test]
    fn a_name_ending_in_a_space_stands_for_every_object_it_starts() {
        assert!(followed_fields("tool T1", &json!({ "active": true })).is_some());
        assert!(followed_fields("AFC_stepper lane2", &json!({ "status": "Loaded" })).is_some());
        // `toolhead` is not one of the `tool ` objects.
        assert!(followed_fields("toolhead", &json!({ "active": true })).is_none());
    }

    #[test]
    fn long_values_are_cut() {
        let long = json!("x".repeat(MAX_VALUE_TEXT * 2));
        let text = brief(&long);
        assert_eq!(text.chars().count(), MAX_VALUE_TEXT + 1);
        assert!(text.ends_with('…'));
        assert_eq!(brief(&json!({ "a": 1 })), r#"{"a":1}"#);
    }
}
