//! Moonraker's `update_manager` component: versions of the installed software and the system packages.

use crate::moonraker::Call;
use serde_json::{json, Value};

/// The entry of the system packages (apt), next to the software repositories.
const SYSTEM: &str = "system";

/// State of a software package in the update manager.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct UpdateInfo {
    pub name: String,
    pub version: String,
    pub remote_version: String,
    /// Number of system packages, or 0/1 for software with a newer version.
    pub pending: u32,
    pub valid: bool,
}

impl UpdateInfo {
    /// The system packages: `pending` counts packages, there is no version.
    pub fn is_system(&self) -> bool {
        self.name == SYSTEM
    }

    pub fn has_update(&self) -> bool {
        self.pending > 0
    }

    /// Known to be current: nothing pending, a valid repository and a known remote version (Moonraker reports `?`
    /// while it has not looked yet).
    pub fn is_up_to_date(&self) -> bool {
        if self.has_update() || !self.valid {
            return false;
        }
        self.is_system() || (!self.remote_version.is_empty() && self.remote_version != "?")
    }
}

/// `machine.update.status`.
pub fn status_call() -> Call {
    Call::new("machine.update.status")
}

pub fn parse_update_status(result: &Value) -> Vec<UpdateInfo> {
    let Some(version_info) = result.get("version_info").and_then(Value::as_object) else {
        return Vec::new();
    };
    let mut updates: Vec<UpdateInfo> = version_info.iter().map(|(name, info)| update_info(name, info)).collect();
    updates.sort_by(|left, right| (!left.is_system(), &left.name).cmp(&(!right.is_system(), &right.name)));
    updates
}

/// `busy` of `machine.update.status`: an update runs (from any client).
pub fn is_busy(status: &Value) -> bool {
    status.get("busy").and_then(Value::as_bool).unwrap_or_default()
}

/// `notify_update_response` carries the output of a running update; its last message has `complete: true`.
pub fn is_response_complete(response: &Value) -> bool {
    response.get("complete").and_then(Value::as_bool).unwrap_or_default()
}

/// Asks the update manager to look for new versions.
pub fn refresh() -> Call {
    Call::new("machine.update.refresh")
}

pub fn upgrade(name: &str) -> Call {
    Call::with("machine.update.upgrade", json!({ "name": name }))
}

fn update_info(name: &str, info: &Value) -> UpdateInfo {
    let text = |field: &str| info.get(field).and_then(Value::as_str).unwrap_or_default().to_owned();
    if name == SYSTEM {
        let count = info.get("package_count").and_then(Value::as_u64).unwrap_or_default();
        return UpdateInfo {
            name: name.to_owned(),
            version: String::new(),
            remote_version: String::new(),
            pending: u32::try_from(count).unwrap_or(u32::MAX),
            valid: true,
        };
    }
    let version = text("version");
    let remote_version = text("remote_version");
    let commits_behind = info.get("commits_behind_count").and_then(Value::as_u64).unwrap_or_default();
    let newer =
        (!remote_version.is_empty() && remote_version != "?" && remote_version != version) || commits_behind > 0;
    UpdateInfo {
        name: name.to_owned(),
        version,
        remote_version,
        pending: u32::from(newer),
        valid: info.get("is_valid").and_then(Value::as_bool).unwrap_or(true),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn update_status_lists_system_first_and_detects_new_versions() {
        let updates = parse_update_status(&json!({ "version_info": {
            "klipper": { "version": "v0.12.0-1", "remote_version": "v0.12.0-5", "is_valid": true },
            "moonraker": { "version": "v0.9.3", "remote_version": "v0.9.3" },
            "system": { "package_count": 4, "package_list": ["a", "b", "c", "d"] }
        }}));

        let names: Vec<&str> = updates.iter().map(|update| update.name.as_str()).collect();
        assert_eq!(names, ["system", "klipper", "moonraker"]);
        assert_eq!(updates.iter().map(UpdateInfo::is_system).collect::<Vec<_>>(), [true, false, false]);
        assert_eq!(updates[0].pending, 4);
        assert!(updates[1].has_update());
        assert!(!updates[2].has_update());
    }

    #[test]
    fn up_to_date_only_with_a_known_remote_version_and_a_valid_repository() {
        let updates = parse_update_status(&json!({ "version_info": {
            "klipper": { "version": "v0.12.0-1", "remote_version": "v0.12.0-5" },
            "moonraker": { "version": "v0.9.3", "remote_version": "v0.9.3" },
            "fluidd": { "version": "v1.30.0", "remote_version": "?" },
            "happy_hare": { "version": "v3.0.0", "remote_version": "v3.0.0", "is_valid": false },
            "system": { "package_count": 0 }
        }}));

        let current: Vec<&str> =
            updates.iter().filter(|update| update.is_up_to_date()).map(|update| update.name.as_str()).collect();
        assert_eq!(current, ["system", "moonraker"]);
    }

    #[test]
    fn busy_from_the_status_and_the_end_of_an_update_response() {
        assert!(is_busy(&json!({ "busy": true, "version_info": {} })));
        assert!(!is_busy(&json!({ "version_info": {} })));
        assert!(!is_response_complete(&json!({ "application": "klipper", "message": "Updating", "complete": false })));
        assert!(is_response_complete(&json!({ "application": "klipper", "message": "Done", "complete": true })));
    }
}
