//! Transfers core state and events into the Slint globals. Runs on the UI thread only.

use crate::bed_mesh_view::mesh_name_problem_text;
use crate::calibration_view::CalibrationView;
use crate::command_name::{command_name, running_label};
use crate::console_view::ConsoleView;
use crate::files_view::{FileState, FilesView};
use crate::history_view::HistoryView;
use crate::items::{
    connection_name, fan_item, heater_item, host_interface_item, led_effect_item, led_item, power_item, replace_rows,
    sensor_item, tuning_item, update_item,
};
use crate::macro_view::MacroView;
use crate::mmu_view::{JobFile, MmuView};
use crate::objects_view::ObjectsView;
use crate::spool_view::{SpoolView, SpoolmanLine};
use crate::status_view::StatusView;
use crate::{bed_map, format, thumbnail, view};
use crate::{
    AppWindow, FanItem, FilamentSensorItem, HeaterItem, HostInterfaceItem, LedEffectItem, LedItem, Network, PowerItem,
    Printer, SensorItem, TuningItem, UiSettings, UpdateItem,
};
use chrono::Local;
use freakymate_core::domain::filament::{self, FilamentHandling};
use freakymate_core::domain::filament_match::{self, ToolMatch};
use freakymate_core::domain::prepared_job::PreparedJob;
use freakymate_core::domain::spools::{self, Lane};
use freakymate_core::domain::{estimates, light, pause};
use freakymate_core::sources::klipper::commands::{mesh_name_problem, MeshNameProblem};
use freakymate_core::sources::klipper::{tuning, KlippyState, Leveling, BED_HEATER, NOZZLE_HEATER};
use freakymate_core::sources::moonraker_server::files::{FileEntry, FileMetadata, ThumbnailImage};
use freakymate_core::sources::moonraker_server::gcode_store::{ConsoleKind, ConsoleLine};
use freakymate_core::sources::moonraker_server::history::{FinishedJob, PrintHistory};
use freakymate_core::sources::spoolman::{Spool, SpoolmanState};
use freakymate_core::sources::PrinterStatus;
use freakymate_core::{Access, Command, ConnectionState, Event, PrinterService, Query, Snapshot};
use slint::{ComponentHandle, ModelRc, SharedString, VecModel};
use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::Rc;

type SnapshotObserver = Box<dyn Fn(&Snapshot)>;
type ConsoleObserver = Box<dyn Fn(&ConsoleLine)>;

#[derive(Default)]
struct Cache {
    snapshot: Snapshot,
    files: FileState,
    spoolman: SpoolmanState,
    /// Spoolman has answered at least once; before that it is not known to be offline.
    spoolman_known: bool,
    /// The spools of the picker, most recently used first.
    spool_list: Vec<Spool>,
    /// The newest job of the history, for the end of a finished print.
    newest_job: Option<FinishedJob>,
    /// The file prepared as the job of the print page.
    prepared: PreparedJob,
}

pub struct Presenter {
    window: slint::Weak<AppWindow>,
    service: PrinterService,
    cache: RefCell<Cache>,
    /// Last logged source of each derived value, so that a change is logged once.
    origins: RefCell<HashMap<&'static str, &'static str>>,
    heaters: Rc<VecModel<HeaterItem>>,
    sensors: Rc<VecModel<SensorItem>>,
    fans: Rc<VecModel<FanItem>>,
    leds: Rc<VecModel<LedItem>>,
    led_effects: Rc<VecModel<LedEffectItem>>,
    tuning: Rc<VecModel<TuningItem>>,
    history: HistoryView,
    /// The temperature store was asked for since the printer became ready.
    history_requested: Cell<bool>,
    /// The lengths the Filament page offers, set again only when the configuration changes them.
    extrude_lengths: RefCell<Vec<f64>>,
    filament_sensors: Rc<VecModel<FilamentSensorItem>>,
    print_objects: ObjectsView,
    calibration: CalibrationView,
    mmu: MmuView,
    files: FilesView,
    spools: SpoolView,
    macros: MacroView,
    console: ConsoleView,
    snapshot_observer: RefCell<Option<SnapshotObserver>>,
    console_observer: RefCell<Option<ConsoleObserver>>,
    power_devices: Rc<VecModel<PowerItem>>,
    updates: Rc<VecModel<UpdateItem>>,
    host_interfaces: Rc<VecModel<HostInterfaceItem>>,
    /// Commands from here that wait long for their answer, oldest first (`Event::CommandRunning`).
    running_commands: RefCell<Vec<(u64, String)>>,
    status: Rc<StatusView>,
}

impl Presenter {
    pub fn new(window: &AppWindow, service: PrinterService) -> Rc<Self> {
        let presenter = Rc::new(Self {
            window: window.as_weak(),
            service,
            cache: RefCell::default(),
            origins: RefCell::default(),
            heaters: Rc::default(),
            sensors: Rc::default(),
            fans: Rc::default(),
            leds: Rc::default(),
            led_effects: Rc::default(),
            tuning: Rc::default(),
            history: HistoryView::new(window),
            history_requested: Cell::default(),
            extrude_lengths: RefCell::default(),
            filament_sensors: Rc::default(),
            print_objects: ObjectsView::default(),
            calibration: CalibrationView::default(),
            mmu: MmuView::default(),
            files: FilesView::default(),
            spools: SpoolView::default(),
            macros: MacroView::default(),
            console: ConsoleView::default(),
            snapshot_observer: RefCell::default(),
            console_observer: RefCell::default(),
            power_devices: Rc::default(),
            updates: Rc::default(),
            host_interfaces: Rc::default(),
            running_commands: RefCell::default(),
            status: StatusView::new(window),
        });
        let printer = window.global::<Printer>();
        printer.set_heaters(ModelRc::from(presenter.heaters.clone()));
        printer.set_sensors(ModelRc::from(presenter.sensors.clone()));
        printer.set_fans(ModelRc::from(presenter.fans.clone()));
        printer.set_leds(ModelRc::from(presenter.leds.clone()));
        printer.set_led_effects(ModelRc::from(presenter.led_effects.clone()));
        printer.set_tuning(ModelRc::from(presenter.tuning.clone()));
        printer.set_filament_sensors(ModelRc::from(presenter.filament_sensors.clone()));
        presenter.print_objects.bind(&printer);
        presenter.calibration.bind(&printer);
        presenter.mmu.bind(&printer);
        presenter.files.bind(&printer);
        presenter.spools.bind(&printer);
        presenter.macros.bind(&printer);
        presenter.console.bind(&printer);
        printer.set_power_devices(ModelRc::from(presenter.power_devices.clone()));
        printer.set_updates(ModelRc::from(presenter.updates.clone()));
        window.global::<Network>().set_host_interfaces(ModelRc::from(presenter.host_interfaces.clone()));
        presenter
    }

    /// Connects the service's snapshots and events to the UI (asynchronously in the Slint event loop).
    pub fn follow(self: &Rc<Self>, channels: freakymate_core::ServiceChannels) -> Result<(), slint::EventLoopError> {
        let freakymate_core::ServiceChannels { mut snapshots, mut events } = channels;
        let presenter = Rc::clone(self);
        slint::spawn_local(async move {
            loop {
                let snapshot = snapshots.borrow_and_update().clone();
                presenter.apply_snapshot(&snapshot);
                if snapshots.changed().await.is_err() {
                    break;
                }
            }
        })?;
        let presenter = Rc::clone(self);
        slint::spawn_local(async move {
            while let Some(event) = events.recv().await {
                presenter.handle_event(event);
            }
        })?;
        Ok(())
    }

    pub fn snapshot(&self) -> Snapshot {
        self.cache.borrow().snapshot.clone()
    }

    /// The index of the print object under a tap on the bed map, see [`bed_map::object_at_tap`].
    pub fn print_object_at(&self, x_share: f64, y_share: f64, reach_share: f64) -> Option<usize> {
        let cache = self.cache.borrow();
        let status = &cache.snapshot.status;
        let bed = status.motion.bed()?;
        bed_map::object_at_tap(&status.print_objects, bed, x_share, y_share, reach_share)
    }

    /// Offers only lengths Klipper accepts; a selected length beyond them falls back to the longest one.
    fn set_extrude_lengths(&self, window: &AppWindow, lengths: Vec<f64>) {
        if *self.extrude_lengths.borrow() == lengths {
            return;
        }
        let values: Vec<f32> = lengths.iter().map(|length| *length as f32).collect();
        let longest = values.last().copied().unwrap_or_default();
        window.global::<Printer>().set_extrude_lengths(ModelRc::from(Rc::new(VecModel::from(values))));
        let settings = window.global::<UiSettings>();
        if settings.get_extrude_length() > longest {
            settings.set_extrude_length(longest);
        }
        *self.extrude_lengths.borrow_mut() = lengths;
    }

    /// Brings the whole UI up to date with a snapshot of the printer.
    pub fn apply_snapshot(&self, snapshot: &Snapshot) {
        let Some(window) = self.window.upgrade() else { return };
        if let Some(observer) = self.snapshot_observer.borrow().as_ref() {
            observer(snapshot);
        }
        let printer = window.global::<Printer>();
        let status = &snapshot.status;
        let ready = snapshot.connection == ConnectionState::Connected && status.klippy_state == KlippyState::Ready;

        self.show_connection(&printer, snapshot, ready);
        self.show_job(&printer, snapshot);
        self.show_motion(&window, status);
        self.show_devices(&printer, snapshot);
        let lanes = self.lanes(snapshot);
        let active_spool = spools::active_spool(&lanes).cloned();
        self.show_temperatures(&printer, snapshot, ready, active_spool.as_ref());
        self.print_objects.show(&printer, &status.print_objects, status.motion.bed());
        self.show_capabilities(&printer, snapshot);
        self.show_filament(&printer, snapshot, &lanes, active_spool.as_ref());

        let previous_job =
            std::mem::replace(&mut self.cache.borrow_mut().snapshot, snapshot.clone()).status.job.filename;
        if previous_job != status.job.filename {
            self.show_job_thumbnail();
        }
        self.refresh_job_texts();
    }

    /// The connection to Moonraker, Klipper's state and who is logged in.
    fn show_connection(&self, printer: &Printer, snapshot: &Snapshot, ready: bool) {
        let status = &snapshot.status;
        printer.set_connection(connection_name(snapshot.connection).into());
        printer.set_klippy_state(status.klippy_state.as_str().into());
        printer.set_klippy_message(status.klippy_message.as_str().into());
        printer.set_klippy_headline(status.klippy_message.lines().next().unwrap_or_default().into());
        printer.set_endpoint(snapshot.endpoint.as_str().into());
        printer.set_ready(ready);
        printer.set_login_user(match &snapshot.access {
            Access::User(username) => username.as_str().into(),
            Access::Open | Access::ApiKey => SharedString::new(),
        });
        printer.set_api_key(snapshot.access == Access::ApiKey);
        if snapshot.connection != ConnectionState::LoginRequired {
            printer.set_login_error(SharedString::new());
        }
        let state = format!(
            "{} {} {}",
            connection_name(snapshot.connection),
            status.klippy_state.as_str(),
            status.job.state.as_str()
        );
        self.status.note_state(&state);
    }

    /// The print: state, progress, interruption and the pause at a layer.
    fn show_job(&self, printer: &Printer, snapshot: &Snapshot) {
        let status = &snapshot.status;
        printer.set_print_state(status.job.state.as_str().into());
        printer.set_activity(status.activity.as_str().into());
        printer.set_filename(status.job.filename.as_str().into());
        printer.set_progress(status.job.progress as f32);
        let interruption = pause::interruption(status);
        printer.set_interruption_headline(
            interruption.as_ref().map(view::interruption_headline).unwrap_or_default().into(),
        );
        let interruption_text = interruption.map(|interruption| view::interruption_text(&interruption, &status.job));
        printer.set_interruption_text(interruption_text.unwrap_or_default().into());
        let layer_pause = pause::layer_pause_offer(&snapshot.sources, status);
        printer.set_layer_pause_next(layer_pause.is_some_and(|offer| offer.next_layer));
        printer.set_layer_pause_at(layer_pause.is_some_and(|offer| offer.at_layer));
        printer.set_current_layer(layer_pause.map_or(0, |offer| offer.current_layer as i32));
        printer.set_total_layers(layer_pause.and_then(|offer| offer.total_layers).map_or(0, |total| total as i32));
        let plan = layer_pause.and_then(|offer| offer.plan);
        printer.set_layer_pause_plan_known(plan.is_some());
        printer.set_layer_pause_planned(view::planned_layer_pause(plan).into());
    }

    /// Position, speed, the z offset and what the extruder can do.
    fn show_motion(&self, window: &AppWindow, status: &PrinterStatus) {
        let printer = window.global::<Printer>();
        let motion = &status.motion;
        printer.set_position_x(motion.live_position[0] as f32);
        printer.set_position_y(motion.live_position[1] as f32);
        printer.set_position_z(motion.live_position[2] as f32);
        printer.set_homed_x(motion.homed.x);
        printer.set_homed_y(motion.homed.y);
        printer.set_homed_z(motion.homed.z);
        printer.set_z_offset(motion.homing_origin[2] as f32);
        printer.set_speed_factor((motion.speed_factor * 100.0) as f32);
        printer.set_live_velocity(motion.live_velocity as f32);
        let flow = estimates::volumetric_flow(motion.live_extruder_velocity, status.extruder.filament_diameter);
        self.note_origin("volumetric flow", Some(flow.origin));
        printer.set_live_flow(flow.value as f32);
        printer.set_can_extrude(status.extruder.can_extrude);
        self.set_extrude_lengths(window, status.extruder.extrude_lengths());
    }

    /// Light, fans, LEDs, the tuning values and the pending configuration.
    fn show_devices(&self, printer: &Printer, snapshot: &Snapshot) {
        let status = &snapshot.status;
        let main_led = light::main_led(&snapshot.sources, &status.leds);
        printer.set_has_light(main_led.is_some());
        printer.set_light_name(main_led.map(|led| led.label.as_str()).unwrap_or_default().into());
        printer.set_light_on(main_led.is_some_and(|led| led.on));
        replace_rows(&self.fans, status.fans.iter().map(fan_item).collect());
        replace_rows(&self.leds, status.leds.iter().map(led_item).collect());
        replace_rows(&self.led_effects, status.led_effects.iter().map(led_effect_item).collect());
        let tuning = tuning::values(status, &snapshot.sources.klipper.tuning);
        replace_rows(&self.tuning, tuning.iter().map(tuning_item).collect());
        printer.set_save_config_pending(status.save_config_pending);
        printer.set_leveling_state(match status.leveling_applied {
            Some(true) => "applied".into(),
            Some(false) => "pending".into(),
            None => SharedString::new(),
        });
    }

    /// Nozzle and bed, the heaters with the hint of the spool in use, and the sensors.
    fn show_temperatures(&self, printer: &Printer, snapshot: &Snapshot, ready: bool, active_spool: Option<&Spool>) {
        let status = &snapshot.status;
        let nozzle = status.heater(NOZZLE_HEATER);
        printer.set_nozzle_temperature(nozzle.map_or(0.0, |heater| heater.temperature) as f32);
        printer.set_nozzle_target(nozzle.map_or(0.0, |heater| heater.target) as f32);
        let bed = status.heater(BED_HEATER);
        printer.set_has_bed(bed.is_some());
        printer.set_bed_temperature(bed.map_or(0.0, |heater| heater.temperature) as f32);
        printer.set_bed_target(bed.map_or(0.0, |heater| heater.target) as f32);
        replace_rows(
            &self.heaters,
            status
                .heaters
                .iter()
                .map(|heater| heater_item(heater, active_spool, self.history.has_history(&heater.object)))
                .collect(),
        );
        replace_rows(
            &self.sensors,
            status.sensors.iter().map(|sensor| sensor_item(sensor, self.history.has_history(&sensor.object))).collect(),
        );
        // Which objects Moonraker keeps a history of: asked once per connection, so the rows know whether to open one.
        if ready && !self.history.knows_objects() && !self.history_requested.replace(true) {
            self.service.query(Query::TemperatureHistory);
        }
        if !ready {
            self.history_requested.set(false);
        }
    }

    /// What the printer offers: leveling, probe, calibration, the filament macros, power devices, updates and macros.
    fn show_capabilities(&self, printer: &Printer, snapshot: &Snapshot) {
        let status = &snapshot.status;
        let klipper = &snapshot.sources.klipper;
        printer.set_can_quad_gantry_level(klipper.leveling == Some(Leveling::QuadGantry));
        printer.set_can_z_tilt(klipper.leveling == Some(Leveling::ZTilt));
        printer.set_has_probe(klipper.probe);
        self.calibration.show(printer, klipper, &status.calibration);
        let macros = match filament::handling(&snapshot.sources, status) {
            FilamentHandling::Macros(macros) => Some(macros),
            FilamentHandling::Mmu | FilamentHandling::None => None,
        };
        printer.set_has_load_macro(macros.is_some_and(|macros| macros.load));
        printer.set_has_unload_macro(macros.is_some_and(|macros| macros.unload));
        printer.set_has_power(snapshot.sources.moonraker.power);
        printer.set_has_update_manager(snapshot.sources.moonraker.update_manager);
        self.macros.show(&klipper.macros, &klipper.macro_details, snapshot.connection == ConnectionState::Connected);
    }

    /// The filament: sensors, the MMU, the job's file compared with it and the spool in use.
    fn show_filament(&self, printer: &Printer, snapshot: &Snapshot, lanes: &[Lane], active_spool: Option<&Spool>) {
        let status = &snapshot.status;
        replace_rows(
            &self.filament_sensors,
            filament::printer_sensors(status)
                .map(|sensor| FilamentSensorItem {
                    label: sensor.label.as_str().into(),
                    enabled: sensor.enabled,
                    detected: sensor.detected,
                })
                .collect(),
        );
        self.cache.borrow_mut().prepared.follow_job(&status.job);
        self.show_job_matches(printer, status, lanes);
        let spoolman = {
            let cache = self.cache.borrow();
            let configured = snapshot.sources.moonraker.spoolman;
            SpoolmanLine {
                configured,
                offline: configured && cache.spoolman_known && !cache.spoolman.connected,
                pending_reports: cache.spoolman.pending_reports,
            }
        };
        self.spools.show_active(printer, &spoolman, active_spool);
    }

    /// Logs which source a derived value comes from when it changes; `None` means no source has a value.
    fn note_origin(&self, value: &'static str, origin: Option<&'static str>) {
        let origin = origin.unwrap_or("none");
        if self.origins.borrow_mut().insert(value, origin) != Some(origin) {
            log::debug!("{value}: {origin}");
        }
    }

    /// Once a second: the clock and job times; an open history is asked for anew every few seconds, as Moonraker
    /// does not report changes of its temperature store.
    pub fn tick(&self) {
        self.refresh_job_texts();
        if self.history.refresh_due() {
            self.service.query(Query::TemperatureHistory);
        }
    }

    /// Opens the temperature history of a heater or sensor and asks Moonraker for the stored minutes.
    pub fn open_history(&self, object: &str) {
        self.history.open(object, &self.cache.borrow().snapshot.status);
        self.service.query(Query::TemperatureHistory);
    }

    pub fn close_history(&self) {
        self.history.close();
    }

    /// Time-related texts; also called every second by the clock timer.
    pub fn refresh_job_texts(&self) {
        let Some(window) = self.window.upgrade() else { return };
        let printer = window.global::<Printer>();
        let now = Local::now();
        let cache = self.cache.borrow();
        let job = &cache.snapshot.status.job;
        let texts = view::job_texts(
            job,
            cache.files.catalog.metadata(&job.filename),
            cache.files.catalog.last_print(&job.filename),
            now,
        );
        printer.set_job_name(texts.name.into());
        printer.set_elapsed(texts.elapsed.into());
        printer.set_remaining(texts.remaining.into());
        printer.set_eta(texts.eta.into());
        printer.set_layers(texts.layers.into());
        printer.set_print_heights(texts.heights.into());
        printer.set_filament(texts.filament.into());
        printer.set_ended(view::ended_text(job, cache.newest_job.as_ref(), now).into());
        if let Some(origin) = texts.expectation_origin {
            self.note_origin("print time expectation", Some(origin));
        }
    }

    pub fn handle_event(&self, event: Event) {
        match event {
            Event::Files(files) => self.show_files(files),
            Event::FileMetadata(metadata) => self.store_metadata(*metadata),
            Event::PrintHistory(history) => self.store_history(history),
            Event::Thumbnail { filename, image } => self.store_thumbnail(&filename, &image),
            Event::TemperatureHistory(store) => self.history.store(store, &self.cache.borrow().snapshot.status),
            Event::PowerDevices(devices) => replace_rows(&self.power_devices, devices.iter().map(power_item).collect()),
            Event::Updates(updates) => {
                replace_rows(&self.updates, updates.iter().map(update_item).collect());
                if let Some(window) = self.window.upgrade() {
                    let available = updates.iter().filter(|update| update.has_update()).count();
                    window.global::<Printer>().set_updates_available(available as i32);
                }
            }
            Event::UpdateManagerBusy(busy) => {
                if let Some(window) = self.window.upgrade() {
                    window.global::<Printer>().set_updates_busy(busy);
                }
            }
            Event::ConsoleHistory(lines) => self.console.show_history(&lines),
            Event::ConsoleLine(line) => {
                if let Some(observer) = self.console_observer.borrow().as_ref() {
                    observer(&line);
                }
                self.console.push(&line);
                if line.kind == ConsoleKind::Response {
                    self.status.console_response(&line, self.is_printing());
                }
            }
            Event::Spoolman(state) => self.store_spoolman(*state),
            Event::SpoolList(spools) => self.store_spool_list(spools),
            Event::SpoolmanChanged => {
                self.service.query(Query::Spoolman);
                self.service.query(Query::SpoolList);
            }
            Event::HostNetwork(interfaces) => {
                replace_rows(&self.host_interfaces, interfaces.iter().map(host_interface_item).collect());
            }
            Event::CommandFailed { command, error } => {
                let message = error.to_string();
                let shown = format!("{}: {message}", command_name(&command));
                self.status.command_failed(&shown, &message);
            }
            Event::LoginFailed(message) => {
                if let Some(window) = self.window.upgrade() {
                    window.global::<Printer>().set_login_error(message.into());
                }
            }
            Event::CommandRunning { id, command, script_line } => {
                self.running_commands.borrow_mut().push((id, running_label(&command, script_line.as_deref())));
                self.show_running_command();
            }
            Event::CommandFinished { id } => {
                self.running_commands.borrow_mut().retain(|(running, _)| *running != id);
                self.show_running_command();
            }
        }
    }

    /// The status bar shows the newest command that is still waiting for its answer.
    fn show_running_command(&self) {
        let Some(window) = self.window.upgrade() else { return };
        let newest = self.running_commands.borrow().last().map(|(_, command)| command.clone()).unwrap_or_default();
        window.global::<Printer>().set_running_command(newest.into());
    }

    /// Hides or shows the temperature reports in the console; the history is read again so that it follows.
    pub fn set_hide_temperature_reports(&self, hide: bool) {
        if self.console.set_hide_temperature_reports(hide) {
            self.service.query(Query::ConsoleHistory);
        }
    }

    /// Applies the hidden macros from the settings.
    pub fn set_hidden_macros(&self, hidden: Vec<String>) {
        self.macros.set_hidden(hidden);
    }

    /// Called with every snapshot after it is shown (the screen standby reads the printer from it).
    pub fn on_snapshot(&self, handler: impl Fn(&Snapshot) + 'static) {
        *self.snapshot_observer.borrow_mut() = Some(Box::new(handler));
    }

    /// Called with every console line that arrives live, not with the history loaded at the start.
    pub fn on_console_line(&self, handler: impl Fn(&ConsoleLine) + 'static) {
        *self.console_observer.borrow_mut() = Some(Box::new(handler));
    }

    /// Called with the printer's macros after a connection, so the settings can forget vanished ones.
    pub fn on_macros_loaded(&self, handler: impl Fn(&[String]) + 'static) {
        self.macros.on_loaded(handler);
    }

    /// How each tool of the job's file (the prepared one, else the printer's) fits the filament it would print with;
    /// empty without file or metadata.
    fn job_matches(&self, status: &PrinterStatus, lanes: &[Lane]) -> Vec<ToolMatch> {
        let cache = self.cache.borrow();
        cache
            .prepared
            .job_file(&status.job)
            .and_then(|file| cache.files.catalog.metadata(file))
            .map(|metadata| filament_match::match_tools(metadata, status, lanes))
            .unwrap_or_default()
    }

    /// The file of the print page's job: the prepared one, else the printer's; empty for none.
    fn job_file(&self, status: &PrinterStatus) -> String {
        self.cache.borrow().prepared.job_file(&status.job).unwrap_or_default().to_owned()
    }

    /// The MMU and the prepared file with how the job's file fits; the comparison is made once for both.
    fn show_job_matches(&self, printer: &Printer, status: &PrinterStatus, lanes: &[Lane]) {
        let matches = self.job_matches(status, lanes);
        let file = JobFile { name: format::job_name(&self.job_file(status)), matches: &matches };
        self.mmu.show(printer, status, lanes, &file);
        if let Some(mmu) = status.happy_hare() {
            self.note_origin("MMU flow rate", mmu.flow_rate.map(|rate| rate.origin));
        }
        self.show_prepared(printer, &matches);
    }

    /// The prepared file with its facts and how its tools fit, for the print page; all empty without one.
    fn show_prepared(&self, printer: &Printer, matches: &[ToolMatch]) {
        let (path, row) = {
            let cache = self.cache.borrow();
            let path = cache.prepared.path().unwrap_or_default().to_owned();
            let row =
                cache.files.catalog.position(&path).map(|index| cache.files.row(&cache.files.catalog.entries()[index]));
            (path, row)
        };
        self.mmu.show_prepared_tools(if path.is_empty() { &[] } else { matches });
        printer.set_prepared_path(path.into());
        printer.set_prepared(row.unwrap_or_default());
    }

    /// Prepares the file at `path` as the job of the print page; an empty path drops the prepared file.
    pub fn prepare_file(&self, path: &str) {
        self.cache.borrow_mut().prepared.prepare(path);
        self.refresh_job_matches();
    }

    /// The comparisons with the job's file again, after the file, its metadata or its thumbnail changed.
    fn refresh_job_matches(&self) {
        let Some(window) = self.window.upgrade() else { return };
        let snapshot = self.snapshot();
        let lanes = self.lanes(&snapshot);
        self.show_job_matches(&window.global::<Printer>(), &snapshot.status, &lanes);
    }

    /// Whether the macro takes parameters, so the form has to come before it runs.
    pub fn macro_has_params(&self, name: &str) -> bool {
        self.macros.has_params(name)
    }

    /// Fills the form with the parameters of a macro; empty values are left to Klipper's defaults.
    pub fn open_macro_form(&self, name: &str) {
        let Some(window) = self.window.upgrade() else { return };
        self.macros.open_form(&window.global::<Printer>(), name);
    }

    /// Sets one parameter of the open form.
    pub fn set_macro_param(&self, index: usize, value: &str) {
        if let Err(error) = self.macros.set_param(index, value) {
            self.show_notice(&error, true);
        }
    }

    /// The command for the open form, e.g. `PARK Z=20`.
    pub fn macro_form_script(&self) -> String {
        self.macros.form_script()
    }

    /// Empties the shown log only; the history on the printer stays and returns on the next load.
    pub fn clear_console(&self) {
        self.console.clear();
    }

    pub fn show_notice(&self, text: &str, is_error: bool) {
        self.status.notice(text, is_error);
    }

    fn is_printing(&self) -> bool {
        self.cache.borrow().snapshot.status.job.state.is_active()
    }

    pub fn open_messages(&self) {
        self.status.open_list();
    }

    pub fn close_messages(&self) {
        self.status.close_list();
    }

    pub fn clear_messages(&self) {
        self.status.clear();
    }

    fn lanes(&self, snapshot: &Snapshot) -> Vec<Lane> {
        spools::lanes(&snapshot.status, &self.cache.borrow().spoolman)
    }

    fn store_spoolman(&self, state: SpoolmanState) {
        {
            let mut cache = self.cache.borrow_mut();
            cache.files.catalog.store_spools(state.spools.values().cloned());
            cache.spoolman = state;
            cache.spoolman_known = true;
        }
        let snapshot = self.cache.borrow().snapshot.clone();
        self.apply_snapshot(&snapshot);
        self.show_spool_rows();
        self.show_file_rows();
    }

    fn store_spool_list(&self, spools: Vec<Spool>) {
        {
            let mut cache = self.cache.borrow_mut();
            cache.files.catalog.store_spools(spools.iter().cloned());
            cache.spool_list = spools;
        }
        self.show_spool_rows();
        self.show_file_rows();
    }

    fn show_spool_rows(&self) {
        let Some(window) = self.window.upgrade() else { return };
        let snapshot = self.snapshot();
        let lanes = self.lanes(&snapshot);
        self.spools.show_rows(&window.global::<Printer>(), &self.cache.borrow().spool_list, &lanes);
    }

    /// Asks for the spools of the picker, and for the state, which the picker shows the places from.
    pub fn load_spools(&self) {
        if let Some(window) = self.window.upgrade() {
            self.spools.show_loading_if_empty(&window.global::<Printer>());
        }
        self.service.query(Query::Spoolman);
        self.service.query(Query::SpoolList);
    }

    /// Heats to what the spool in use recommends; the bed keeps its target if Spoolman has no temperature for it.
    pub fn apply_spool_preset(&self) {
        let snapshot = self.snapshot();
        let lanes = self.lanes(&snapshot);
        if let Some(preset) = spools::active_spool(&lanes).and_then(|spool| spools::preset(spool, &snapshot.status)) {
            self.service.send(preset);
        }
    }

    /// Why `name` cannot be saved as a bed mesh profile, or that it replaces one; empty while nothing is typed.
    pub fn bed_mesh_name_hint(&self, name: &str) -> String {
        let name = name.trim();
        match mesh_name_problem(name) {
            Some(problem) => mesh_name_problem_text(problem).to_owned(),
            None if self.has_bed_mesh_profile(name) => "Overwrites this profile".to_owned(),
            None => String::new(),
        }
    }

    /// Saves the active mesh as `name`, or says why Klipper would not.
    pub fn save_bed_mesh(&self, name: &str) {
        match mesh_name_problem(name) {
            Some(MeshNameProblem::Empty) => self.show_notice("Enter a name for the profile", true),
            Some(problem) => self.show_notice(mesh_name_problem_text(problem), true),
            None => self.service.send(Command::SaveBedMesh { profile: name.to_owned() }),
        }
    }

    fn has_bed_mesh_profile(&self, name: &str) -> bool {
        let snapshot = self.snapshot();
        snapshot
            .status
            .calibration
            .bed_mesh
            .as_ref()
            .is_some_and(|mesh| mesh.profiles.iter().any(|profile| profile == name))
    }

    /// Klipper would refuse `typed` as the target of `heater`; a text that is no number is not judged.
    pub fn heater_target_outside(&self, heater: &str, typed: &str) -> bool {
        let Ok(target) = typed.trim().parse::<f64>() else { return false };
        self.snapshot().status.heater(heater).is_some_and(|heater| !heater.accepts_target(target))
    }

    /// What was sent to pause at `layer`, in the words of the dialog's button.
    pub fn layer_pause_notice(&self, layer: u32) -> String {
        view::layer_pause_notice(layer, self.cache.borrow().snapshot.status.job.current_layer)
    }

    /// The height `layer` of the running print will have, for the layer pause dialog.
    pub fn height_at_layer_text(&self, layer: i32) -> String {
        let Ok(layer) = u32::try_from(layer) else { return String::new() };
        let cache = self.cache.borrow();
        let job = &cache.snapshot.status.job;
        view::height_at_layer(job, cache.files.catalog.metadata(&job.filename), layer)
    }

    /// Hints before printing the file, one line each, for the rows of the question.
    pub fn print_hints(&self, path: &str) -> Vec<String> {
        let snapshot = self.snapshot();
        let lanes = self.lanes(&snapshot);
        let cache = self.cache.borrow();
        let Some(metadata) = cache.files.catalog.metadata(path) else { return Vec::new() };
        let show_tool = lanes.iter().any(|lane| lane.gate.is_some());
        filament_match::print_hints(metadata, &snapshot.status, &lanes)
            .iter()
            .map(|hint| view::print_hint_text(hint, show_tool))
            .collect()
    }

    pub fn request_files(&self) {
        self.service.query(Query::Files);
        self.service.query(Query::PrintHistory);
        self.service.query(Query::SpoolList);
    }

    /// Marks or unmarks a file of the list; marked files can be deleted, a single one can be prepared.
    pub fn toggle_file_mark(&self, path: &str) {
        self.cache.borrow_mut().files.toggle_mark(path);
        self.show_file_rows();
    }

    pub fn clear_file_marks(&self) {
        self.cache.borrow_mut().files.clear_marks();
        self.show_file_rows();
    }

    /// The marked files, which are no longer marked afterwards.
    pub fn take_marked_files(&self) -> Vec<String> {
        let marked = self.cache.borrow_mut().files.take_marked();
        self.show_file_rows();
        marked
    }

    fn show_files(&self, files: Vec<FileEntry>) {
        {
            let mut cache = self.cache.borrow_mut();
            cache.files.forget_vanished_marks(&files);
            cache.prepared.follow_files(files.iter().map(|file| file.path.as_str()));
            for changed in cache.files.catalog.set_entries(files) {
                cache.files.thumbnails.remove(&changed);
            }
        }
        self.show_file_rows();
        self.refresh_job_matches();
        let missing = self.cache.borrow().files.catalog.needing_details();
        for filename in missing {
            self.service.query(Query::FileDetails { filename });
        }
    }

    fn show_file_rows(&self) {
        self.files.show_rows(&self.cache.borrow().files);
        let Some(window) = self.window.upgrade() else { return };
        self.files.show_marks(&window.global::<Printer>(), &self.cache.borrow().files);
    }

    fn store_history(&self, history: PrintHistory) {
        {
            let mut cache = self.cache.borrow_mut();
            cache.files.catalog.store_last_prints(history.last_prints);
            cache.newest_job = history.newest;
        }
        self.show_file_rows();
        self.refresh_job_texts();
    }

    fn store_metadata(&self, metadata: FileMetadata) {
        let filename = metadata.filename.clone();
        let (is_job, is_compared) = {
            let mut cache = self.cache.borrow_mut();
            cache.files.catalog.store_metadata(metadata);
            let is_job = cache.snapshot.status.job.filename == filename;
            (is_job, is_job || cache.prepared.path() == Some(filename.as_str()))
        };
        self.files.update_row(&self.cache.borrow().files, &filename);
        if is_job {
            self.refresh_job_texts();
        }
        if is_compared {
            self.refresh_job_matches();
        }
    }

    fn store_thumbnail(&self, filename: &str, thumbnail: &ThumbnailImage) {
        let Some(image) = thumbnail::to_image(thumbnail) else {
            log::info!("thumbnail of {filename} cannot be shown");
            return;
        };
        let is_prepared = {
            let mut cache = self.cache.borrow_mut();
            cache.files.thumbnails.insert(filename.to_owned(), image);
            cache.prepared.path() == Some(filename)
        };
        self.files.update_row(&self.cache.borrow().files, filename);
        self.show_job_thumbnail();
        if is_prepared {
            self.refresh_job_matches();
        }
    }

    fn show_job_thumbnail(&self) {
        let Some(window) = self.window.upgrade() else { return };
        let printer = window.global::<Printer>();
        let cache = self.cache.borrow();
        match cache.files.thumbnails.get(&cache.snapshot.status.job.filename) {
            Some(image) => {
                printer.set_job_thumbnail(image.clone());
                printer.set_has_job_thumbnail(true);
            }
            None => printer.set_has_job_thumbnail(false),
        }
    }
}
