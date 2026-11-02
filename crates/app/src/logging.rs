//! The log: a file next to the logs of Klipper and Moonraker (when asked for with `--logfile`) and stderr.
//!
//! Like Moonraker's log the file rotates at midnight, keeps a few old files named `<file>.YYYY-MM-DD` and starts each
//! file with a header that says which build and which machine wrote it. Writing happens on its own thread, so a slow
//! SD card never stalls the UI. Records with the target [`HEADER_TARGET`] are remembered and repeated in the header
//! after a rotation.

use chrono::{Local, NaiveDate};
use freakymate_core::HEADER_TARGET;
use log::{Level, LevelFilter, Log, Metadata, Record};
use std::fs::{self, File, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, SyncSender};
use std::sync::{Mutex, OnceLock};
use std::thread;
use std::time::Duration;

/// Old files kept next to the current one.
const OLD_FILES_KEPT: usize = 3;
/// Records waiting for the writer; more than this are dropped rather than blocking the caller.
const QUEUE_LENGTH: usize = 4096;
const STAMP_FORMAT: &str = "%Y-%m-%d %H:%M:%S,%3f";
const FLUSH_TIMEOUT: Duration = Duration::from_secs(2);

/// What [`init`] needs to know.
#[derive(Debug, Default)]
pub struct Config {
    /// The log file; without it the log goes to stderr only.
    pub file: Option<PathBuf>,
    /// Facts about this run for the header, as `(name, value)` in the order to show them.
    pub header: Vec<(String, String)>,
}

/// Installs the logger. `RUST_LOG` decides what is logged when set; otherwise the program logs at info, and
/// [`set_verbose`] raises its own modules to debug.
pub fn init(config: Config) -> io::Result<()> {
    let writer = config.file.map(|path| spawn_writer(path, config.header)).transpose()?;
    let logger = Logger::new(writer, std::env::var("RUST_LOG").ok().as_deref());
    log::set_max_level(logger.max_level());
    let logger = LOGGER.get_or_init(|| logger);
    log::set_logger(logger).map_err(io::Error::other)?;
    std::panic::set_hook(Box::new(|info| {
        log::error!("panic: {info}");
        log::logger().flush();
    }));
    Ok(())
}

/// Debug level for the program's own modules (the setting `verbose_log`); `RUST_LOG` overrides it.
pub fn set_verbose(verbose: bool) {
    if let Some(logger) = LOGGER.get() {
        logger.verbose.store(verbose, Ordering::Relaxed);
        log::set_max_level(logger.max_level());
    }
}

/// Notes a fact for the header of this and every later file, and logs it.
pub fn header(name: &str, value: &str) {
    log::info!(target: HEADER_TARGET, "{name}: {value}");
}

static LOGGER: OnceLock<Logger> = OnceLock::new();

struct Logger {
    /// From `RUST_LOG`.
    filter: Option<env_filter::Filter>,
    verbose: AtomicBool,
    file: Option<SyncSender<Message>>,
    /// Several threads write to stderr; one line at a time.
    stderr: Mutex<()>,
}

enum Message {
    /// `fact` is set for a header record: its name and value.
    Line {
        day: NaiveDate,
        stamp: String,
        text: String,
        fact: Option<(String, String)>,
    },
    Flush(SyncSender<()>),
}

impl Logger {
    fn new(file: Option<SyncSender<Message>>, rust_log: Option<&str>) -> Self {
        let filter =
            rust_log.filter(|spec| !spec.trim().is_empty()).map(|spec| env_filter::Builder::new().parse(spec).build());
        Self { filter, verbose: AtomicBool::new(false), file, stderr: Mutex::new(()) }
    }

    fn max_level(&self) -> LevelFilter {
        match &self.filter {
            Some(filter) => filter.filter(),
            None if self.verbose.load(Ordering::Relaxed) => LevelFilter::Debug,
            None => LevelFilter::Info,
        }
    }

    fn wanted(&self, metadata: &Metadata) -> bool {
        match &self.filter {
            Some(filter) => filter.enabled(metadata),
            None => {
                let verbose = self.verbose.load(Ordering::Relaxed) && metadata.target().starts_with("freakymate");
                metadata.level() <= if verbose { Level::Debug } else { Level::Info }
            }
        }
    }
}

impl Log for Logger {
    fn enabled(&self, metadata: &Metadata) -> bool {
        self.wanted(metadata)
    }

    fn log(&self, record: &Record) {
        let matches = match &self.filter {
            Some(filter) => filter.matches(record),
            None => self.wanted(record.metadata()),
        };
        if !matches {
            return;
        }
        let now = Local::now();
        let stamp = now.format(STAMP_FORMAT).to_string();
        let text = format_line(&stamp, record.level(), record.target(), &record.args().to_string());
        {
            let _line = self.stderr.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
            // Nowhere to report a failing stderr.
            let _ = io::stderr().write_all(text.as_bytes());
        }
        if let Some(file) = &self.file {
            // A full queue means a stuck disk: losing lines is better than freezing the UI.
            let fact = (record.target() == HEADER_TARGET).then(|| split_fact(&record.args().to_string())).flatten();
            let _ = file.try_send(Message::Line { day: now.date_naive(), stamp, text, fact });
        }
    }

    fn flush(&self) {
        if let Some(file) = &self.file {
            let (done, wait) = mpsc::sync_channel(1);
            if file.send(Message::Flush(done)).is_ok() {
                let _ = wait.recv_timeout(FLUSH_TIMEOUT);
            }
        }
    }
}

/// `2026-11-02 14:03:12,345 WARN [module] - text`, one line per record (further lines of the text are indented).
fn format_line(stamp: &str, level: Level, target: &str, text: &str) -> String {
    let module = if target == HEADER_TARGET { "header" } else { target };
    let level = match level {
        Level::Error => "ERROR",
        Level::Warn => "WARN",
        Level::Info => "INFO",
        Level::Debug => "DEBUG",
        Level::Trace => "TRACE",
    };
    let mut lines = text.lines();
    let mut line = format!("{stamp} {level} [{module}] - {}\n", lines.next().unwrap_or_default());
    for continued in lines {
        line.push_str("    ");
        line.push_str(continued);
        line.push('\n');
    }
    line
}

fn spawn_writer(path: PathBuf, header: Vec<(String, String)>) -> io::Result<SyncSender<Message>> {
    let today = Local::now().date_naive();
    let mut file = RotatingFile::open(path, today, OLD_FILES_KEPT)?;
    file.write_banner("FreakyMate log started", &Local::now().format(STAMP_FORMAT).to_string(), &header)?;
    let (sender, receiver) = mpsc::sync_channel(QUEUE_LENGTH);
    thread::Builder::new().name("log-writer".to_owned()).spawn(move || write_all(file, header, receiver))?;
    Ok(sender)
}

/// Writes until every sender is gone. A file that cannot be written is given up: the log continues on stderr.
fn write_all(mut file: RotatingFile, mut header: Vec<(String, String)>, receiver: Receiver<Message>) {
    for message in receiver {
        match message {
            Message::Line { day, stamp, text, fact } => {
                if let Some((name, value)) = fact {
                    remember_fact(&mut header, name, value);
                }
                if let Err(error) = file.write_line(day, &stamp, &text, &header) {
                    eprintln!("cannot write the log file {}: {error}", file.path.display());
                    return;
                }
            }
            Message::Flush(done) => {
                let _ = file.flush();
                let _ = done.send(());
            }
        }
    }
    let _ = file.flush();
}

fn split_fact(text: &str) -> Option<(String, String)> {
    let (name, value) = text.split_once(": ")?;
    Some((name.to_owned(), value.to_owned()))
}

/// Keeps a fact for the header, replacing an earlier value of the same name.
fn remember_fact(header: &mut Vec<(String, String)>, name: String, value: String) {
    match header.iter_mut().find(|(known, _)| *known == name) {
        Some(entry) => entry.1 = value,
        None => header.push((name, value)),
    }
}

/// The current log file plus the rotation by day.
struct RotatingFile {
    path: PathBuf,
    file: File,
    /// Day of the lines written so far.
    day: NaiveDate,
    old_files_kept: usize,
}

impl RotatingFile {
    /// Appends to an existing file; one from an earlier day is rotated first.
    fn open(path: PathBuf, today: NaiveDate, old_files_kept: usize) -> io::Result<Self> {
        if let Some(parent) = path.parent().filter(|parent| !parent.as_os_str().is_empty()) {
            fs::create_dir_all(parent)?;
        }
        if let Some(day) = modified_day(&path).filter(|day| *day < today) {
            rotate(&path, day, old_files_kept)?;
        }
        let file = open_append(&path)?;
        Ok(Self { path, file, day: today, old_files_kept })
    }

    fn write_line(&mut self, day: NaiveDate, stamp: &str, text: &str, header: &[(String, String)]) -> io::Result<()> {
        if day > self.day {
            rotate(&self.path, self.day, self.old_files_kept)?;
            self.file = open_append(&self.path)?;
            self.day = day;
            self.write_banner("FreakyMate log continued", stamp, header)?;
        }
        self.file.write_all(text.as_bytes())
    }

    fn write_banner(&mut self, title: &str, stamp: &str, header: &[(String, String)]) -> io::Result<()> {
        let mut banner = format_line(stamp, Level::Info, HEADER_TARGET, &format!("---- {title} ----"));
        for (name, value) in header {
            banner.push_str(&format_line(stamp, Level::Info, HEADER_TARGET, &format!("{name}: {value}")));
        }
        self.file.write_all(banner.as_bytes())
    }

    fn flush(&mut self) -> io::Result<()> {
        self.file.flush()
    }
}

fn open_append(path: &Path) -> io::Result<File> {
    OpenOptions::new().create(true).append(true).open(path)
}

fn modified_day(path: &Path) -> Option<NaiveDate> {
    let modified = fs::metadata(path).ok()?.modified().ok()?;
    Some(chrono::DateTime::<Local>::from(modified).date_naive())
}

/// Renames the file to `<file>.<day>` and deletes old files beyond the number to keep.
fn rotate(path: &Path, day: NaiveDate, old_files_kept: usize) -> io::Result<()> {
    if !path.exists() {
        return Ok(());
    }
    let target = dated_name(path, day);
    // The target exists after a clock that went back; renaming over it fails on Windows.
    let _ = fs::remove_file(&target);
    fs::rename(path, target)?;
    let mut old: Vec<(NaiveDate, PathBuf)> = old_files(path);
    old.sort();
    let surplus = old.len().saturating_sub(old_files_kept);
    for (_, path) in old.into_iter().take(surplus) {
        let _ = fs::remove_file(path);
    }
    Ok(())
}

fn dated_name(path: &Path, day: NaiveDate) -> PathBuf {
    let mut name = path.as_os_str().to_owned();
    name.push(format!(".{day}"));
    PathBuf::from(name)
}

/// The rotated files of this log with the day in their name.
fn old_files(path: &Path) -> Vec<(NaiveDate, PathBuf)> {
    let (Some(directory), Some(name)) = (path.parent(), path.file_name().and_then(|name| name.to_str())) else {
        return Vec::new();
    };
    let directory = if directory.as_os_str().is_empty() { Path::new(".") } else { directory };
    let Ok(entries) = fs::read_dir(directory) else { return Vec::new() };
    entries
        .flatten()
        .filter_map(|entry| {
            let file_name = entry.file_name().into_string().ok()?;
            let day = file_name.strip_prefix(name)?.strip_prefix('.')?.parse::<NaiveDate>().ok()?;
            Some((day, entry.path()))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn day(text: &str) -> NaiveDate {
        text.parse().unwrap()
    }

    fn names(directory: &Path) -> Vec<String> {
        let mut names: Vec<String> =
            fs::read_dir(directory).unwrap().flatten().map(|entry| entry.file_name().into_string().unwrap()).collect();
        names.sort();
        names
    }

    #[test]
    fn formats_like_moonraker_with_the_level() {
        let line = format_line("2026-11-02 14:03:12,345", Level::Warn, "freakymate_core::service", "lost");
        assert_eq!(line, "2026-11-02 14:03:12,345 WARN [freakymate_core::service] - lost\n");
    }

    #[test]
    fn indents_further_lines_of_a_record() {
        let line = format_line("t", Level::Error, "m", "first\nsecond");
        assert_eq!(line, "t ERROR [m] - first\n    second\n");
    }

    #[test]
    fn names_header_records_header() {
        let line = format_line("t", Level::Info, HEADER_TARGET, "Version: 1");
        assert_eq!(line, "t INFO [header] - Version: 1\n");
    }

    #[test]
    fn starts_with_the_header_and_appends_after_a_restart() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("logs").join("freakymate.log");
        let header = vec![("Version".to_owned(), "1.0".to_owned())];
        for _ in 0..2 {
            let mut file = RotatingFile::open(path.clone(), day("2026-11-02"), 3).unwrap();
            file.write_banner("FreakyMate log started", "t", &header).unwrap();
        }
        let text = fs::read_to_string(&path).unwrap();
        assert_eq!(text.matches("---- FreakyMate log started ----").count(), 2);
        assert_eq!(text.matches("[header] - Version: 1.0").count(), 2);
    }

    #[test]
    fn rotates_at_midnight_and_repeats_the_header() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("freakymate.log");
        let header = vec![("Version".to_owned(), "1.0".to_owned())];
        let mut file = RotatingFile::open(path.clone(), day("2026-11-02"), 3).unwrap();
        file.write_line(day("2026-11-02"), "t", "evening\n", &header).unwrap();
        file.write_line(day("2026-11-03"), "t", "morning\n", &header).unwrap();
        let old = fs::read_to_string(directory.path().join("freakymate.log.2026-11-02")).unwrap();
        assert_eq!(old, "evening\n");
        let current = fs::read_to_string(&path).unwrap();
        assert!(current.contains("---- FreakyMate log continued ----"));
        assert!(current.contains("Version: 1.0"));
        assert!(current.ends_with("morning\n"));
    }

    #[test]
    fn rotates_a_file_from_an_earlier_day_when_opening() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("freakymate.log");
        fs::write(&path, "old\n").unwrap();
        let tomorrow = Local::now().date_naive().succ_opt().unwrap();
        RotatingFile::open(path.clone(), tomorrow, 3).unwrap();
        let today = Local::now().date_naive();
        assert_eq!(fs::read_to_string(dated_name(&path, today)).unwrap(), "old\n");
        assert_eq!(fs::read_to_string(&path).unwrap(), "");
    }

    #[test]
    fn keeps_only_the_newest_old_files() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("freakymate.log");
        for old in ["2026-10-29", "2026-10-30", "2026-10-31", "2026-11-01"] {
            fs::write(dated_name(&path, day(old)), old).unwrap();
        }
        fs::write(directory.path().join("other.log.2026-10-01"), "x").unwrap();
        fs::write(&path, "current").unwrap();
        rotate(&path, day("2026-11-02"), 3).unwrap();
        assert_eq!(
            names(directory.path()),
            [
                "freakymate.log.2026-10-31",
                "freakymate.log.2026-11-01",
                "freakymate.log.2026-11-02",
                "other.log.2026-10-01"
            ]
        );
    }

    #[test]
    fn remembers_header_facts_by_name() {
        let mut header = vec![("Version".to_owned(), "1".to_owned())];
        for text in ["Moonraker: v0.9", "Moonraker: v0.10"] {
            let (name, value) = split_fact(text).unwrap();
            remember_fact(&mut header, name, value);
        }
        assert_eq!(header, [("Version".to_owned(), "1".to_owned()), ("Moonraker".to_owned(), "v0.10".to_owned())]);
        assert_eq!(split_fact("no separator"), None);
    }

    #[test]
    fn verbose_raises_only_the_programs_modules() {
        let logger = Logger::new(None, None);
        let metadata = |target| Metadata::builder().level(Level::Debug).target(target).build();
        assert!(!logger.wanted(&metadata("freakymate_core::service")));
        logger.verbose.store(true, Ordering::Relaxed);
        assert!(logger.wanted(&metadata("freakymate_core::service")));
        assert!(!logger.wanted(&metadata("hyper::client")));
        assert_eq!(logger.max_level(), LevelFilter::Debug);
    }

    #[test]
    fn rust_log_has_the_say() {
        let logger = Logger::new(None, Some("warn,freakymate_core=trace"));
        logger.verbose.store(false, Ordering::Relaxed);
        let metadata = |target| Metadata::builder().level(Level::Trace).target(target).build();
        assert!(logger.wanted(&metadata("freakymate_core")));
        assert!(!logger.wanted(&metadata("hyper")));
    }
}
