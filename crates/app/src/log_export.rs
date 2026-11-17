//! The log as one file for a bug report: the current log file and the rotated old ones, oldest first, each part
//! behind a separator line. Only Android hands it to the user (`android::log_export`); the assembly here is plain
//! and tested on the desktop.

use crate::logging;
use chrono::NaiveDateTime;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

/// The log as it is handed out.
#[derive(Debug, PartialEq, Eq)]
pub struct LogExport {
    pub file_name: String,
    pub content: Vec<u8>,
}

/// Reads the log of the running program; fails when it writes no file.
#[cfg_attr(not(target_os = "android"), allow(dead_code))]
pub fn read_current(now: NaiveDateTime) -> io::Result<LogExport> {
    let path = logging::file_path().ok_or_else(|| io::Error::other("the log is not written to a file"))?;
    // Lines still waiting for the writer thread belong in the export.
    log::logger().flush();
    read(path, now)
}

fn read(current: &Path, now: NaiveDateTime) -> io::Result<LogExport> {
    let mut parts = Vec::new();
    for path in files_oldest_first(current) {
        let name = path.file_name().map(|name| name.to_string_lossy().into_owned()).unwrap_or_default();
        parts.push((name, fs::read(&path)?));
    }
    Ok(LogExport { file_name: file_name(now), content: combine(&parts) })
}

/// `sailhand-log-20261117-153000.txt`
fn file_name(now: NaiveDateTime) -> String {
    format!("sailhand-log-{}.txt", now.format("%Y%m%d-%H%M%S"))
}

/// The rotated files by day, then the current one; a missing current file (nothing written yet) is left out.
fn files_oldest_first(current: &Path) -> Vec<PathBuf> {
    let mut old = logging::old_files(current);
    old.sort();
    let mut files: Vec<PathBuf> = old.into_iter().map(|(_, path)| path).collect();
    if current.is_file() {
        files.push(current.to_owned());
    }
    files
}

/// Each part behind a `===== <name> =====` line; a part without a final line break gets one.
fn combine(parts: &[(String, Vec<u8>)]) -> Vec<u8> {
    let mut text = Vec::new();
    for (name, content) in parts {
        text.extend_from_slice(format!("===== {name} =====\n").as_bytes());
        text.extend_from_slice(content);
        if !content.ends_with(b"\n") {
            text.push(b'\n');
        }
    }
    text
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::NaiveDate;

    fn moment() -> NaiveDateTime {
        NaiveDate::from_ymd_opt(2026, 11, 17).unwrap().and_hms_opt(15, 30, 0).unwrap()
    }

    #[test]
    fn the_name_carries_the_time() {
        assert_eq!(file_name(moment()), "sailhand-log-20261117-153000.txt");
    }

    #[test]
    fn parts_get_a_separator_and_a_final_line_break() {
        let parts = [("a.log.2026-11-16".to_owned(), b"one\n".to_vec()), ("a.log".to_owned(), b"two".to_vec())];
        let text = String::from_utf8(combine(&parts)).unwrap();
        assert_eq!(text, "===== a.log.2026-11-16 =====\none\n===== a.log =====\ntwo\n");
    }

    #[test]
    fn old_files_come_first_in_the_order_of_their_days() {
        let directory = tempfile::tempdir().unwrap();
        let current = directory.path().join("sailhand.log");
        fs::write(&current, "now\n").unwrap();
        fs::write(directory.path().join("sailhand.log.2026-11-16"), "later\n").unwrap();
        fs::write(directory.path().join("sailhand.log.2026-11-09"), "earlier\n").unwrap();
        fs::write(directory.path().join("other.log"), "unrelated\n").unwrap();
        let export = read(&current, moment()).unwrap();
        let text = String::from_utf8(export.content).unwrap();
        assert_eq!(
            text,
            "===== sailhand.log.2026-11-09 =====\nearlier\n===== sailhand.log.2026-11-16 =====\nlater\n\
             ===== sailhand.log =====\nnow\n"
        );
        assert_eq!(export.file_name, "sailhand-log-20261117-153000.txt");
    }

    #[test]
    fn a_missing_current_file_is_left_out() {
        let directory = tempfile::tempdir().unwrap();
        let current = directory.path().join("sailhand.log");
        assert!(files_oldest_first(&current).is_empty());
    }
}
