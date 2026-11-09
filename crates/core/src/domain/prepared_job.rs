//! The file prepared as the job of the print page. Only the app knows it: Moonraker and other clients see nothing, and
//! a restart forgets it, so Klipper's `print_stats.filename` is the job again. A print that runs, started from
//! anywhere, takes its place; a file that is no longer listed drops it.

use crate::sources::klipper::{PrintJob, PrintState};

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PreparedJob {
    path: Option<String>,
}

impl PreparedJob {
    /// Prepares the file at `path`; an empty path drops the prepared file.
    pub fn prepare(&mut self, path: &str) {
        self.path = Some(path.to_owned()).filter(|path| !path.is_empty());
    }

    pub fn path(&self) -> Option<&str> {
        self.path.as_deref()
    }

    /// Drops the prepared file while a print runs or is paused: the running file is the job then, and after it ends the
    /// printer's last file is (reprint and prepare are one way). True when that changed something.
    pub fn follow_job(&mut self, job: &PrintJob) -> bool {
        let running = matches!(job.state, PrintState::Printing | PrintState::Paused);
        running && self.path.take().is_some()
    }

    /// Drops the prepared file when the file list no longer has it (deleted here or elsewhere). True when that changed
    /// something.
    pub fn follow_files<'a>(&mut self, mut listed: impl Iterator<Item = &'a str>) -> bool {
        let gone = self.path.as_deref().is_some_and(|path| !listed.any(|listed| listed == path));
        if gone {
            self.path = None;
        }
        gone
    }

    /// The file of the print page's job: the prepared one, else the printer's last or running file; `None` for none.
    pub fn job_file<'a>(&'a self, job: &'a PrintJob) -> Option<&'a str> {
        self.path().or_else(|| Some(job.filename.as_str()).filter(|filename| !filename.is_empty()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn job(state: PrintState, filename: &str) -> PrintJob {
        PrintJob { state, filename: filename.into(), ..PrintJob::default() }
    }

    fn prepared(path: &str) -> PreparedJob {
        let mut prepared = PreparedJob::default();
        prepared.prepare(path);
        prepared
    }

    #[test]
    fn a_prepared_file_is_the_job_until_a_print_runs() {
        let mut prepared = prepared("cube.gcode");
        let finished = job(PrintState::Complete, "benchy.gcode");
        assert!(!prepared.follow_job(&finished));
        assert_eq!(prepared.job_file(&finished), Some("cube.gcode"));

        let running = job(PrintState::Printing, "cube.gcode");
        assert!(prepared.follow_job(&running));
        assert_eq!(prepared.path(), None);
        assert_eq!(prepared.job_file(&running), Some("cube.gcode"));
        assert!(!prepared.follow_job(&running), "nothing left to drop");
    }

    #[test]
    fn a_print_started_elsewhere_replaces_the_prepared_file() {
        let mut prepared = prepared("cube.gcode");
        assert!(prepared.follow_job(&job(PrintState::Paused, "benchy.gcode")));
        assert_eq!(prepared.job_file(&job(PrintState::Complete, "benchy.gcode")), Some("benchy.gcode"));
    }

    #[test]
    fn without_a_prepared_file_the_printer_has_the_job() {
        let prepared = PreparedJob::default();
        assert_eq!(prepared.job_file(&job(PrintState::Complete, "benchy.gcode")), Some("benchy.gcode"));
        assert_eq!(prepared.job_file(&job(PrintState::Standby, "")), None);
    }

    #[test]
    fn an_empty_path_drops_the_prepared_file() {
        let mut prepared = prepared("cube.gcode");
        prepared.prepare("");
        assert_eq!(prepared.path(), None);
    }

    #[test]
    fn a_file_no_longer_listed_drops_it() {
        let mut prepared = prepared("cube.gcode");
        assert!(!prepared.follow_files(["benchy.gcode", "cube.gcode"].into_iter()));
        assert_eq!(prepared.path(), Some("cube.gcode"));
        assert!(prepared.follow_files(["benchy.gcode"].into_iter()));
        assert_eq!(prepared.path(), None);
    }
}
