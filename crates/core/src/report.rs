//! The diagnostic report for a bug report (`freakymate --report`): what Moonraker and Klipper tell about this printer
//! and what FreakyMate detects in it, as one JSON file. It is the state at one moment; the verbose log tells what led
//! there.
//!
//! The report only reads. It leaves out what is secret or personal: Moonraker's configuration (API keys, passwords),
//! network addresses, the host name, serial numbers, the commit history of the update manager and the stored access
//! of this display.

use crate::credentials::CredentialStore;
use crate::domain::machine_tree::MachineTree;
use crate::moonraker::{Connection, Endpoint, InvalidEndpoint};
use crate::service::{introduce, Access};
use crate::settings::{Settings, SettingsStore};
use crate::sources::moonraker_server::{server_info, ServerComponents};
use crate::sources::{ObjectStore, PrinterStatus, Sources};
use serde_json::{json, Map, Value};
use std::future::Future;
use std::path::{Path, PathBuf};
use std::time::Duration;
use thiserror::Error;

/// Name of the report file.
pub const FILE_NAME: &str = "freakymate-report.json";
/// Raised when the layout of the report changes, so a reader knows what to expect.
const FORMAT_VERSION: u32 = 1;
/// The settings as the installer places them, relative to the home folder; the service is started with this path.
const INSTALLED_SETTINGS: &str = "printer_data/config/freakymate.conf";
const CALL_TIMEOUT: Duration = Duration::from_secs(10);
/// Lines of Klipper's console in the report, the newest ones.
const CONSOLE_LINES: u32 = 100;

/// Fields of a `machine.update.status` entry that tell the installed version; the commit history (authors, messages)
/// stays out.
const UPDATE_FIELDS: &[&str] = &[
    "name",
    "configured_type",
    "detected_type",
    "channel",
    "version",
    "remote_version",
    "full_version_string",
    "rollback_version",
    "branch",
    "owner",
    "repo_name",
    "is_valid",
    "is_dirty",
    "corrupt",
    "detached",
    "pristine",
    "warnings",
    "anomalies",
];
/// Fields of `machine.system_info.cpu_info` without the serial number.
const CPU_FIELDS: &[&str] =
    &["cpu_count", "bits", "processor", "cpu_desc", "hardware_desc", "model", "total_memory", "memory_units"];

/// What `freakymate --report` was asked for.
#[derive(Debug, Default)]
pub struct ReportRequest {
    /// Moonraker's address; the saved setting otherwise.
    pub moonraker: Option<String>,
    /// The settings file; the installer's place or the default location otherwise.
    pub settings_file: Option<PathBuf>,
    /// File or folder to write to; see [`output_path`].
    pub output: Option<PathBuf>,
    /// Facts about this program as the log header has them, as `(name, value)`.
    pub facts: Vec<(String, String)>,
}

#[derive(Debug, Error)]
pub enum ReportError {
    #[error(transparent)]
    Endpoint(#[from] InvalidEndpoint),
    #[error("cannot connect to {url}: {reason}")]
    Connect { url: String, reason: String },
    #[error("Moonraker does not let this display in: {0}")]
    Access(String),
    #[error("cannot write {path}: {source}")]
    Write { path: PathBuf, source: std::io::Error },
}

/// Collects the report and writes it; returns the file written.
pub async fn write(request: ReportRequest) -> Result<PathBuf, ReportError> {
    let store = settings_store(request.settings_file);
    let settings = store.load().unwrap_or_else(|error| {
        log::warn!("{error}; using defaults");
        Settings::default()
    });
    let endpoint = Endpoint::parse(request.moonraker.as_deref().unwrap_or(&settings.moonraker))?;
    let freakymate = freakymate_section(&request.facts, &store, &settings);
    let report = collect(&endpoint, &CredentialStore::default_location(), freakymate).await?;
    let path = output_path(request.output.as_deref(), store.path(), Path::new("."));
    let text = serde_json::to_string_pretty(&report).unwrap_or_default();
    std::fs::write(&path, text).map_err(|source| ReportError::Write { path: path.clone(), source })?;
    Ok(path)
}

/// The report of the printer behind `endpoint`, with `freakymate` as what this program says about itself.
pub async fn collect(
    endpoint: &Endpoint,
    credentials: &CredentialStore,
    freakymate: Value,
) -> Result<Value, ReportError> {
    let url = endpoint.websocket_url();
    let connect_failed = |reason: String| ReportError::Connect { url: url.clone(), reason };
    let (connection, _notifications) = tokio::time::timeout(CALL_TIMEOUT, Connection::open(&url))
        .await
        .map_err(|_| connect_failed("no answer".to_owned()))?
        .map_err(|error| connect_failed(error.to_string()))?;
    let access =
        introduce(&connection, credentials, endpoint).await.map_err(|error| ReportError::Access(error.to_string()))?;
    let moonraker = Asker(&connection);

    let server = moonraker.ask("server.info", None).await;
    let components = server.as_ref().map(|info| server_info::parse_server_info(info).components).unwrap_or_default();
    let spoolman = if components.iter().any(|name| name == "spoolman") {
        shown(moonraker.ask("server.spoolman.status", None).await, Value::clone)
    } else {
        json!("no spoolman component")
    };
    let objects = moonraker.ask("printer.objects.list", None).await;
    let names = objects.as_ref().map(object_names).unwrap_or_default();
    let status = if names.is_empty() {
        Err("no objects: Klipper is not ready".to_owned())
    } else {
        moonraker
            .ask("printer.objects.query", Some(all_objects(&names)))
            .await
            .map(|answer| without_raw_config(&answer))
    };
    let help = moonraker.ask("printer.gcode.help", None).await;
    let detected = match &status {
        Ok(status) => detected(&names, &components, status, help.as_ref().ok()),
        Err(error) => json!({ "error": error }),
    };

    Ok(json!({
        "format": FORMAT_VERSION,
        "freakymate": freakymate,
        "moonraker": {
            "address": endpoint.display(),
            "access": access_kind(&access),
            "server.info": shown(server, Value::clone),
            "printer.info": shown(moonraker.ask("printer.info", None).await, |info| without(info, &["hostname"])),
            "machine.system_info": shown(moonraker.ask("machine.system_info", None).await, system_info),
            "machine.update.status": shown(moonraker.ask("machine.update.status", None).await, update_status),
            "server.spoolman.status": spoolman,
            "server.gcode_store": shown(
                moonraker.ask("server.gcode_store", Some(json!({ "count": CONSOLE_LINES }))).await,
                Value::clone,
            ),
        },
        "klipper": {
            "printer.objects.list": shown(objects, Value::clone),
            "printer.objects.query": shown(status, Value::clone),
            "printer.gcode.help": shown(help, Value::clone),
        },
        "detected": detected,
    }))
}

/// Where the report goes: `output` (a folder gets the [`FILE_NAME`]), otherwise the `logs` folder of the
/// `printer_data` that holds the settings, so Mainsail and Fluidd offer it for download like the log, otherwise
/// `current`.
pub fn output_path(output: Option<&Path>, settings_file: &Path, current: &Path) -> PathBuf {
    match output {
        Some(path) if path.is_dir() => path.join(FILE_NAME),
        Some(path) => path.to_owned(),
        None => logs_folder(settings_file).unwrap_or_else(|| current.to_owned()).join(FILE_NAME),
    }
}

/// The `logs` folder next to the `config` folder holding the settings file, if there is one.
fn logs_folder(settings_file: &Path) -> Option<PathBuf> {
    let config = settings_file.parent()?;
    if config.file_name()? != "config" {
        return None;
    }
    let logs = config.parent()?.join("logs");
    logs.is_dir().then_some(logs)
}

/// The settings the service uses: the given file, the installer's place or the default location.
fn settings_store(explicit: Option<PathBuf>) -> SettingsStore {
    if let Some(path) = explicit {
        return SettingsStore::new(path);
    }
    let installed = dirs::home_dir().map(|home| home.join(INSTALLED_SETTINGS)).filter(|path| path.exists());
    match installed {
        Some(path) => SettingsStore::new(path),
        None => SettingsStore::default_location().unwrap_or_else(|_| SettingsStore::new(INSTALLED_SETTINGS)),
    }
}

fn freakymate_section(facts: &[(String, String)], store: &SettingsStore, settings: &Settings) -> Value {
    let mut section: Map<String, Value> =
        facts.iter().map(|(name, value)| (name.clone(), Value::String(value.clone()))).collect();
    section.insert("Settings file".to_owned(), json!(store.path().display().to_string()));
    section.insert("Settings".to_owned(), serde_json::to_value(settings).unwrap_or_default());
    Value::Object(section)
}

/// Asks Moonraker with a time limit; a failure becomes its text, so the rest of the report is still written.
struct Asker<'a>(&'a Connection);

impl Asker<'_> {
    async fn ask(&self, method: &str, params: Option<Value>) -> Result<Value, String> {
        within_time(self.0.call(method, params)).await
    }
}

async fn within_time<E: std::fmt::Display>(call: impl Future<Output = Result<Value, E>>) -> Result<Value, String> {
    match tokio::time::timeout(CALL_TIMEOUT, call).await {
        Ok(Ok(answer)) => Ok(answer),
        Ok(Err(error)) => Err(error.to_string()),
        Err(_) => Err("no answer".to_owned()),
    }
}

/// An answer for the report after `filter`, or the error as `{"error": …}`.
fn shown(answer: Result<Value, String>, filter: impl Fn(&Value) -> Value) -> Value {
    match answer {
        Ok(value) => filter(&value),
        Err(error) => json!({ "error": error }),
    }
}

fn access_kind(access: &Access) -> &'static str {
    match access {
        Access::Open => "trusted or open",
        Access::ApiKey => "api key",
        // The user name stays out.
        Access::User(_) => "login",
    }
}

fn object_names(listing: &Value) -> Vec<String> {
    listing
        .get("objects")
        .and_then(Value::as_array)
        .map(|names| names.iter().filter_map(Value::as_str).map(str::to_owned).collect())
        .unwrap_or_default()
}

/// Parameters of `printer.objects.query` for every field of every object.
fn all_objects(names: &[String]) -> Value {
    let objects: Map<String, Value> = names.iter().map(|name| (name.clone(), Value::Null)).collect();
    json!({ "objects": objects })
}

/// The status of a query without `configfile.config`, the raw text of the configuration that `settings` repeats.
fn without_raw_config(answer: &Value) -> Value {
    let mut status = answer.get("status").cloned().unwrap_or_default();
    if let Some(configfile) = status.get_mut("configfile").and_then(Value::as_object_mut) {
        configfile.remove("config");
    }
    status
}

/// What FreakyMate makes of the printer: the detected sources and the tree of carriages, heads, nozzles and lanes.
fn detected(names: &[String], components: &[String], status: &Value, help: Option<&Value>) -> Value {
    let mut sources = Sources::detect(names, ServerComponents::from_names(components.iter().map(String::as_str)));
    sources.apply_settings(status.pointer("/configfile/settings"));
    if let Some(help) = help {
        sources.apply_help(help);
    }
    let mut store = ObjectStore::default();
    store.replace(status);
    let status = PrinterStatus::from_store(&store);
    let tree = MachineTree::build(&status);
    json!({
        "summary": sources.summary(),
        "changers": status.changer_systems(),
        "machine_tree": format!("{tree:#?}").lines().collect::<Vec<_>>(),
    })
}

/// Only the listed fields of an object.
fn keep(value: &Value, fields: &[&str]) -> Value {
    let kept: Map<String, Value> = value
        .as_object()
        .map(|object| object.iter().filter(|(name, _)| fields.contains(&name.as_str())))
        .into_iter()
        .flatten()
        .map(|(name, value)| (name.clone(), value.clone()))
        .collect();
    Value::Object(kept)
}

/// An object without the listed fields.
fn without(value: &Value, fields: &[&str]) -> Value {
    let mut value = value.clone();
    if let Some(object) = value.as_object_mut() {
        object.retain(|name, _| !fields.contains(&name.as_str()));
    }
    value
}

/// The board and the system, without network, CAN bus, SD card and its serial number.
fn system_info(answer: &Value) -> Value {
    let info = answer.get("system_info").unwrap_or(&Value::Null);
    let field = |name: &str| info.get(name).cloned().unwrap_or_default();
    json!({
        "cpu_info": keep(&field("cpu_info"), CPU_FIELDS),
        "distribution": field("distribution"),
        "virtualization": field("virtualization"),
        "service_state": field("service_state"),
        "python": keep(&field("python"), &["version_string"]),
    })
}

/// The installed versions of everything the update manager knows (Klipper, Moonraker, Happy Hare, AFC, …).
fn update_status(answer: &Value) -> Value {
    let versions: Map<String, Value> = answer
        .get("version_info")
        .and_then(Value::as_object)
        .map(|entries| entries.iter().map(|(name, entry)| (name.clone(), keep(entry, UPDATE_FIELDS))).collect())
        .unwrap_or_default();
    Value::Object(versions)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_report_goes_to_the_logs_of_printer_data() {
        let data = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(data.path().join("config")).unwrap();
        std::fs::create_dir_all(data.path().join("logs")).unwrap();
        let settings = data.path().join("config/freakymate.conf");
        let current = Path::new("/somewhere");
        assert_eq!(output_path(None, &settings, current), data.path().join("logs").join(FILE_NAME));
        // Settings elsewhere: the current folder.
        assert_eq!(
            output_path(None, Path::new("/home/x/.config/freakymate/settings.conf"), current),
            current.join(FILE_NAME)
        );
        // A given folder gets the file name, a given file stays.
        assert_eq!(output_path(Some(data.path()), &settings, current), data.path().join(FILE_NAME));
        let file = data.path().join("bug.json");
        assert_eq!(output_path(Some(&file), &settings, current), file);
    }

    #[test]
    fn secrets_and_personal_fields_stay_out() {
        let info = json!({ "system_info": {
            "cpu_info": { "model": "Raspberry Pi 4", "serial_number": "1000abcd" },
            "network": { "wlan0": { "mac_address": "aa:bb" } },
            "sd_info": { "serial_number": "0x1234" },
            "distribution": { "name": "Debian 12" },
        }});
        let shown = system_info(&info);
        assert_eq!(shown["cpu_info"], json!({ "model": "Raspberry Pi 4" }));
        assert!(shown.get("network").is_none() && shown.get("sd_info").is_none());

        let updates = json!({ "version_info": { "happy-hare": {
            "version": "v3.4.1", "commits_behind": [{ "author": "someone", "subject": "fix" }], "git_messages": ["x"]
        }}});
        assert_eq!(update_status(&updates), json!({ "happy-hare": { "version": "v3.4.1" } }));

        let query =
            json!({ "status": { "configfile": { "config": { "printer": {} }, "settings": { "printer": {} } } } });
        assert_eq!(without_raw_config(&query), json!({ "configfile": { "settings": { "printer": {} } } }));
        assert_eq!(
            without(&json!({ "hostname": "voron", "state": "ready" }), &["hostname"]),
            json!({ "state": "ready" })
        );
    }
}
