//! The print job: `print_stats`, with the progress of `display_status` or `virtual_sdcard`.

use crate::sources::fields::{count, number, text};
use serde_json::Value;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum PrintState {
    #[default]
    Standby,
    Printing,
    Paused,
    Complete,
    Cancelled,
    Error,
}

impl PrintState {
    pub fn parse(text: &str) -> Self {
        match text {
            "printing" => Self::Printing,
            "paused" => Self::Paused,
            "complete" => Self::Complete,
            "cancelled" => Self::Cancelled,
            "error" => Self::Error,
            _ => Self::Standby,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Standby => "standby",
            Self::Printing => "printing",
            Self::Paused => "paused",
            Self::Complete => "complete",
            Self::Cancelled => "cancelled",
            Self::Error => "error",
        }
    }

    /// A job is running or paused; manual moves and extrusion are locked then.
    pub fn is_active(self) -> bool {
        matches!(self, Self::Printing | Self::Paused)
    }

    /// The job has ended, well or not.
    pub fn is_finished(self) -> bool {
        matches!(self, Self::Complete | Self::Cancelled | Self::Error)
    }
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct PrintJob {
    pub state: PrintState,
    pub filename: String,
    pub message: String,
    /// What `M117` / `SET_DISPLAY_TEXT` show (`display_status.message`); empty when none or cleared.
    pub display_message: String,
    pub progress: f64,
    pub print_duration: f64,
    pub total_duration: f64,
    pub filament_used: f64,
    pub current_layer: Option<u32>,
    pub total_layer: Option<u32>,
    /// Height of the layer being printed, or the one an ended job reached, see [`super::HeightWatch`].
    pub height: Option<f64>,
    /// When the pause began (Unix seconds), if it was seen beginning, see [`super::JobWatch`].
    pub paused_since: Option<f64>,
}

pub(super) fn read_job(print_stats: &Value, display_status: &Value, virtual_sdcard: &Value) -> PrintJob {
    let info = print_stats.get("info").unwrap_or(&Value::Null);
    let progress = display_status
        .get("progress")
        .or_else(|| virtual_sdcard.get("progress"))
        .and_then(Value::as_f64)
        .unwrap_or_default();
    PrintJob {
        state: PrintState::parse(text(print_stats, "state")),
        filename: text(print_stats, "filename").to_owned(),
        message: text(print_stats, "message").to_owned(),
        display_message: text(display_status, "message").to_owned(),
        progress: progress.clamp(0.0, 1.0),
        print_duration: number(print_stats, "print_duration"),
        total_duration: number(print_stats, "total_duration"),
        filament_used: number(print_stats, "filament_used"),
        current_layer: count(info, "current_layer"),
        total_layer: count(info, "total_layer"),
        height: None,
        paused_since: None,
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn display_message_is_read_and_null_clears_it() {
        let none = json!({});
        let set = json!({ "progress": 0.5, "message": "Part 2 of 3" });
        assert_eq!(read_job(&none, &set, &none).display_message, "Part 2 of 3");
        let cleared = json!({ "progress": 0.5, "message": null });
        assert_eq!(read_job(&none, &cleared, &none).display_message, "");
    }
}
