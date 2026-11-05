//! Shared state of all connections: printer, host, file catalog and runtime.

use crate::access::{Access, AccessConfig};
use crate::files::Catalog;
use crate::host::Host;
use crate::machine::MachineProfile;
use crate::printer::Printer;
use crate::spoolman::Spoolman;
use crate::wled::Wled;
use serde_json::{json, Value};
use std::sync::Mutex;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use tokio::sync::broadcast;

const NOTIFICATION_BUFFER: usize = 64;

pub struct Backend {
    printer: Mutex<Printer>,
    host: Mutex<Host>,
    spoolman: Mutex<Spoolman>,
    spoolman_enabled: bool,
    wled: Mutex<Wled>,
    /// Moonraker's `wled` component, with the extra devices of the profile.
    wled_enabled: bool,
    catalog: Mutex<Catalog>,
    /// Jobs that ended while the fake runs, newest first; the history lists them before the demo jobs.
    ended_jobs: Mutex<Vec<Value>>,
    started: Instant,
    notifications: broadcast::Sender<String>,
    access: Access,
}

impl Backend {
    pub fn new(time_scale: f64) -> Self {
        Self::with_profile(time_scale, MachineProfile::default())
    }

    pub fn with_profile(time_scale: f64, profile: MachineProfile) -> Self {
        Self {
            printer: Mutex::new(Printer::with_profile(time_scale, profile)),
            host: Mutex::new(Host::default()),
            spoolman: Mutex::new(Spoolman::default()),
            spoolman_enabled: profile.spoolman,
            wled: Mutex::new(Wled::default()),
            wled_enabled: profile.devices,
            catalog: Mutex::new(Catalog::demo()),
            ended_jobs: Mutex::new(Vec::new()),
            started: Instant::now(),
            notifications: broadcast::channel(NOTIFICATION_BUFFER).0,
            access: Access::default(),
        }
    }

    /// Wants a login from every connection, unlike the default that trusts all.
    pub fn with_access(mut self, config: Option<AccessConfig>) -> Self {
        self.access = Access::new(config);
        self
    }

    pub fn access(&self) -> &Access {
        &self.access
    }

    /// Receiver for notifications to all connections (G-code responses, Klippy state ...).
    pub fn notifications(&self) -> broadcast::Receiver<String> {
        self.notifications.subscribe()
    }

    pub fn notify(&self, method: &str, params: Value) {
        let message = json!({ "jsonrpc": "2.0", "method": method, "params": params });
        let _ = self.notifications.send(message.to_string());
    }

    /// Reports a Klipper response line like Moonraker: in the history and as `notify_gcode_response`.
    pub fn respond(&self, message: &str) {
        self.with_host(|host| host.record("response", message, unix_time()));
        self.notify("notify_gcode_response", json!([message]));
    }

    pub fn record_command(&self, script: &str) {
        self.with_host(|host| host.record("command", script, unix_time()));
    }

    pub fn with_host<R>(&self, action: impl FnOnce(&mut Host) -> R) -> R {
        let mut host = self.host.lock().expect("host lock poisoned");
        action(&mut host)
    }

    pub fn spoolman_enabled(&self) -> bool {
        self.spoolman_enabled
    }

    pub fn wled_enabled(&self) -> bool {
        self.wled_enabled
    }

    pub fn with_wled<R>(&self, action: impl FnOnce(&mut Wled) -> R) -> R {
        let mut wled = self.wled.lock().expect("wled lock poisoned");
        action(&mut wled)
    }

    pub fn with_spoolman<R>(&self, action: impl FnOnce(&mut Spoolman) -> R) -> R {
        let mut spoolman = self.spoolman.lock().expect("spoolman lock poisoned");
        action(&mut spoolman)
    }

    pub fn catalog(&self) -> std::sync::MutexGuard<'_, Catalog> {
        self.catalog.lock().expect("catalog lock poisoned")
    }

    /// `server.history.list`: the jobs that ended here, then the demo history.
    pub fn history(&self) -> Value {
        let demo = self.catalog().history();
        let mut jobs = self.ended_jobs.lock().expect("history lock poisoned").clone();
        jobs.extend(demo["jobs"].as_array().cloned().unwrap_or_default());
        json!({ "count": jobs.len(), "jobs": jobs })
    }

    pub fn status(&self) -> Value {
        self.with_printer(|printer| printer.status())
    }

    pub fn object_names(&self) -> Vec<String> {
        self.with_printer(|printer| printer.object_names())
    }

    pub fn advance_simulation(&self, elapsed: Duration) {
        self.with_printer(|printer| printer.advance(elapsed));
    }

    pub fn eventtime(&self) -> f64 {
        self.started.elapsed().as_secs_f64()
    }

    /// Every change of the printer passes here, so the end of a job is recorded like Moonraker's history does.
    pub fn with_printer<R>(&self, action: impl FnOnce(&mut Printer) -> R) -> R {
        let (result, ended_job) = {
            let mut printer = self.printer.lock().expect("printer lock poisoned");
            let was_active = printer.print_state().is_active();
            let result = action(&mut printer);
            let ended = was_active && !printer.print_state().is_active();
            (result, ended.then(|| printer.ended_job(unix_time())).flatten())
        };
        if let Some(job) = ended_job {
            self.ended_jobs.lock().expect("history lock poisoned").insert(0, job.clone());
            self.notify("notify_history_changed", json!([{ "action": "finished", "job": job }]));
        }
        result
    }
}

fn unix_time() -> f64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map_or(0.0, |elapsed| elapsed.as_secs_f64())
}
