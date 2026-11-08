//! Moonraker's `history` component: the jobs printed so far, the last completed print per file and the newest job.

use crate::moonraker::Call;
use serde::Deserialize;
use serde_json::{json, Value};
use std::collections::HashMap;

/// What the last completed print of a file measured, from the Moonraker job history.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct LastPrint {
    /// Seconds spent printing, without paused and idle time.
    pub print_duration: f64,
    /// Filament length in millimeters.
    pub filament_used: f64,
    /// Spoolman spools the job used (`auxiliary_data`); empty without Spoolman.
    pub spool_ids: Vec<u32>,
}

#[derive(Deserialize)]
struct AuxiliaryEntry {
    #[serde(default)]
    name: String,
    #[serde(default)]
    value: Value,
}

/// The newest job of the history once it has ended, for the end of the print the print page shows (see
/// [`crate::domain::job_end`]).
#[derive(Debug, Clone, PartialEq, Default)]
pub struct FinishedJob {
    pub filename: String,
    /// Moonraker's status: `completed`, `cancelled`, `error`, `klippy_shutdown`, `klippy_disconnect`, ...
    pub status: String,
    /// Unix seconds.
    pub end_time: f64,
}

/// The job history as far as the UI uses it.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct PrintHistory {
    pub last_prints: HashMap<String, LastPrint>,
    pub newest: Option<FinishedJob>,
}

/// Jobs the history asks for.
const PRINT_HISTORY_LIMIT: u32 = 300;

/// `server.history.list`.
pub fn list_call() -> Call {
    Call::with("server.history.list", json!({ "limit": PRINT_HISTORY_LIMIT }))
}

pub fn parse_print_history(result: &Value) -> PrintHistory {
    let jobs: Vec<HistoryJob> =
        result.get("jobs").and_then(|jobs| serde_json::from_value(jobs.clone()).ok()).unwrap_or_default();
    PrintHistory { last_prints: last_completed_prints(&jobs), newest: jobs.first().and_then(HistoryJob::finished) }
}

#[derive(Deserialize)]
struct HistoryJob {
    #[serde(default)]
    filename: String,
    #[serde(default)]
    status: String,
    #[serde(default)]
    end_time: Option<f64>,
    #[serde(default)]
    print_duration: f64,
    #[serde(default)]
    filament_used: f64,
    #[serde(default)]
    auxiliary_data: Vec<AuxiliaryEntry>,
}

impl HistoryJob {
    fn spool_ids(&self) -> Vec<u32> {
        self.auxiliary_data
            .iter()
            .filter(|entry| entry.name == "spool_ids")
            .filter_map(|entry| entry.value.as_array())
            .flatten()
            .filter_map(|id| id.as_u64().and_then(|id| u32::try_from(id).ok()))
            .collect()
    }

    /// A job that is still running has no end time yet (`null`).
    fn finished(&self) -> Option<FinishedJob> {
        let end_time = self.end_time.filter(|_| self.status != "in_progress")?;
        Some(FinishedJob { filename: self.filename.clone(), status: self.status.clone(), end_time })
    }
}

/// The newest completed job per file; `server.history.list` returns the newest job first. Cancelled and failed
/// jobs are left out because their duration and filament say nothing about a whole print.
fn last_completed_prints(jobs: &[HistoryJob]) -> HashMap<String, LastPrint> {
    let mut last_prints = HashMap::new();
    for job in jobs.iter().filter(|job| job.status == "completed" && job.print_duration > 0.0) {
        last_prints.entry(job.filename.clone()).or_insert(LastPrint {
            print_duration: job.print_duration,
            filament_used: job.filament_used,
            spool_ids: job.spool_ids(),
        });
    }
    last_prints
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn last_completed_prints_of(result: Value) -> HashMap<String, LastPrint> {
        parse_print_history(&result).last_prints
    }

    #[test]
    fn the_newest_job_is_the_first_of_the_list() {
        let history = parse_print_history(&json!({ "jobs": [
            { "filename": "a.gcode", "status": "cancelled", "end_time": 1_700_000_600.0 },
            { "filename": "a.gcode", "status": "completed", "end_time": 1_600_000_000.0 }
        ] }));

        assert_eq!(
            history.newest,
            Some(FinishedJob { filename: "a.gcode".into(), status: "cancelled".into(), end_time: 1_700_000_600.0 })
        );
    }

    #[test]
    fn a_running_job_has_not_ended() {
        let history = parse_print_history(&json!({ "jobs": [
            { "filename": "a.gcode", "status": "in_progress", "end_time": null }
        ] }));

        assert_eq!(history.newest, None);
    }

    #[test]
    fn last_completed_print_is_the_newest_one_that_finished() {
        let last_prints = last_completed_prints_of(json!({
            "count": 4,
            "jobs": [
                { "filename": "a.gcode", "status": "cancelled", "print_duration": 50.0, "filament_used": 100.0 },
                { "filename": "a.gcode", "status": "completed", "print_duration": 900.0, "filament_used": 2000.0 },
                { "filename": "a.gcode", "status": "completed", "print_duration": 1000.0, "filament_used": 2100.0 },
                { "filename": "b.gcode", "status": "error", "print_duration": 10.0, "filament_used": 5.0 }
            ]
        }));

        assert_eq!(last_prints.len(), 1);
        assert_eq!(
            last_prints["a.gcode"],
            LastPrint { print_duration: 900.0, filament_used: 2000.0, ..LastPrint::default() }
        );
    }

    #[test]
    fn the_spools_of_a_job_come_from_the_auxiliary_data() {
        let last_prints = last_completed_prints_of(json!({ "jobs": [{
            "filename": "a.gcode", "status": "completed", "print_duration": 900.0, "filament_used": 2000.0,
            "auxiliary_data": [
                { "provider": "other", "name": "other", "value": [9] },
                { "provider": "spoolman", "name": "spool_ids", "value": [18, 21], "units": null }
            ]
        }] }));

        assert_eq!(last_prints["a.gcode"].spool_ids, [18, 21]);
    }

    #[test]
    fn history_without_jobs_is_empty() {
        assert_eq!(parse_print_history(&json!({})), PrintHistory::default());
    }
}
