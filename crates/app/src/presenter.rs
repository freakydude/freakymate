//! Transfers core state and events into the Slint globals. Runs on the UI thread only.

use crate::bed_mesh_view::mesh_name_problem_text;
use crate::calibration_view::CalibrationView;
use crate::history_view::HistoryView;
use crate::items::{
    color_bar, connection_name, console_items, fan_item, filament_path_item, heater_item, host_interface_item,
    led_effect_item, led_item, power_item, replace_rows, sensor_item, spool_rows, tuning_item, update_item,
};
use crate::objects_view::ObjectsView;
use crate::status_log::{self, display_time, StatusLog};
use crate::{bed_map, format, thumbnail, view};
use crate::{
    AppWindow, ConsoleItem, FanItem, FilamentSensorItem, FileItem, HeaterItem, HostInterfaceItem, LedEffectItem,
    LedItem, MacroItem, MacroParamItem, MessageItem, MmuGateItem, MmuSensorItem, MmuToolItem, Network, PowerItem,
    Printer, SensorItem, SpoolItem, TuningItem, UiSettings, UpdateItem,
};
use chrono::Local;
use freakymate_core::domain::filament::{self, FilamentHandling};
use freakymate_core::domain::filament_match::{self, FileColor, ToolMatch, Verdict};
use freakymate_core::domain::file_catalog::FileCatalog;
use freakymate_core::domain::spools::{self, Lane};
use freakymate_core::domain::{estimates, light, pause};
use freakymate_core::sources::happy_hare::{MmuGate, MmuStatus};
use freakymate_core::sources::klipper::commands::{mesh_name_problem, MeshNameProblem};
use freakymate_core::sources::klipper::{offered_macros, tuning, KlippyState, Leveling, MacroDetails, MacroForm};
use freakymate_core::sources::moonraker_server::files::{FileEntry, FileMetadata, ThumbnailImage};
use freakymate_core::sources::moonraker_server::gcode_store::{ConsoleKind, ConsoleLine};
use freakymate_core::sources::moonraker_server::history::{FinishedJob, PrintHistory};
use freakymate_core::sources::spoolman::{Spool, SpoolmanState};
use freakymate_core::sources::PrinterStatus;
use freakymate_core::{Access, Command, ConnectionState, Event, PrinterService, Query, Snapshot};
use slint::{ComponentHandle, Image, Model, ModelRc, SharedString, Timer, TimerMode, VecModel};
use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::collections::{BTreeMap, BTreeSet};
use std::rc::Rc;
use std::sync::Arc;

const CONSOLE_LIMIT: usize = 300;
/// Status bar and its flyout. Shared with the timer that hides informations, hence the `Rc`.
struct StatusView {
    window: slint::Weak<AppWindow>,
    log: RefCell<StatusLog>,
    messages: Rc<VecModel<MessageItem>>,
}

impl StatusView {
    /// Puts the log on screen: the flyout list with its sections, the event the status bar shows and the counter.
    fn refresh(&self) {
        let log = self.log.borrow();
        let has_new = log.messages().any(|message| log.is_new(message));
        let has_earlier = log.messages().any(|message| !log.is_new(message));
        let mut top = if has_new { status_log::SECTION_HEIGHT } else { 0.0 };
        let mut earlier_top = -1.0;
        let items: Vec<MessageItem> = log
            .messages()
            .map(|message| {
                let new = log.is_new(message);
                if !new && earlier_top < 0.0 {
                    earlier_top = top;
                    top += status_log::SECTION_HEIGHT;
                }
                let height = status_log::row_height(&message.text);
                let item = MessageItem {
                    text: SharedString::from(message.text.as_str()),
                    time: format::unix_clock(message.time).into(),
                    is_error: message.is_error,
                    new,
                    top,
                    height,
                };
                top += height + status_log::ROW_GAP;
                item
            })
            .collect();
        replace_rows(&self.messages, items);
        let Some(window) = self.window.upgrade() else { return };
        let printer = window.global::<Printer>();
        printer.set_messages_height(top);
        printer.set_messages_has_new(has_new);
        printer.set_messages_has_earlier(has_earlier);
        printer.set_messages_earlier_top(earlier_top.max(0.0));
        printer.set_unread_errors(i32::try_from(log.unread_errors()).unwrap_or(i32::MAX));
        let event = log.event();
        printer.set_status_message(event.map(|message| SharedString::from(message.headline())).unwrap_or_default());
        printer.set_status_message_error(event.is_some_and(|message| message.is_error));
        printer.set_status_message_visible(event.is_some());
    }
}

fn now_seconds() -> f64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0.0, |elapsed| elapsed.as_secs_f64())
}

type MacrosLoaded = Box<dyn Fn(&[String])>;
type SnapshotObserver = Box<dyn Fn(&Snapshot)>;
type ConsoleObserver = Box<dyn Fn(&ConsoleLine)>;

#[derive(Default)]
struct Cache {
    snapshot: Snapshot,
    catalog: FileCatalog,
    thumbnails: HashMap<String, Image>,
    spoolman: SpoolmanState,
    /// Spoolman has answered at least once; before that it is not known to be offline.
    spoolman_known: bool,
    /// The spools of the picker, most recently used first.
    spool_list: Vec<Spool>,
    /// The newest job of the history, for the end of a finished print.
    newest_job: Option<FinishedJob>,
    /// Paths of the files marked in the file list; they outlive a rebuild of the rows.
    marked_files: BTreeSet<String>,
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
    /// Leave Klipper's temperature reports out of the console (setting).
    hide_temperature_reports: Cell<bool>,
    /// The lengths the Filament page offers, set again only when the configuration changes them.
    extrude_lengths: RefCell<Vec<f64>>,
    filament_sensors: Rc<VecModel<FilamentSensorItem>>,
    print_objects: ObjectsView,
    calibration: CalibrationView,
    mmu_gates: Rc<VecModel<MmuGateItem>>,
    mmu_sensors: Rc<VecModel<MmuSensorItem>>,
    mmu_tools: Rc<VecModel<MmuToolItem>>,
    macros: Rc<VecModel<SharedString>>,
    macro_items: Rc<VecModel<MacroItem>>,
    all_macros: RefCell<Vec<String>>,
    hidden_macros: RefCell<Vec<String>>,
    macros_loaded: RefCell<Option<MacrosLoaded>>,
    snapshot_observer: RefCell<Option<SnapshotObserver>>,
    console_observer: RefCell<Option<ConsoleObserver>>,
    macro_details: RefCell<Arc<BTreeMap<String, MacroDetails>>>,
    macro_form: Rc<VecModel<MacroParamItem>>,
    macro_form_state: RefCell<MacroForm>,
    files: Rc<VecModel<FileItem>>,
    spools: Rc<VecModel<SpoolItem>>,
    console: Rc<VecModel<ConsoleItem>>,
    power_devices: Rc<VecModel<PowerItem>>,
    updates: Rc<VecModel<UpdateItem>>,
    host_interfaces: Rc<VecModel<HostInterfaceItem>>,
    /// Commands from here that wait long for their answer, oldest first (`Event::CommandRunning`).
    running_commands: RefCell<Vec<(u64, String)>>,
    status: Rc<StatusView>,
    status_timer: Timer,
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
            hide_temperature_reports: Cell::new(true),
            extrude_lengths: RefCell::default(),
            filament_sensors: Rc::default(),
            print_objects: ObjectsView::default(),
            calibration: CalibrationView::default(),
            mmu_gates: Rc::default(),
            mmu_sensors: Rc::default(),
            mmu_tools: Rc::default(),
            macros: Rc::default(),
            macro_items: Rc::default(),
            all_macros: RefCell::default(),
            hidden_macros: RefCell::default(),
            macros_loaded: RefCell::default(),
            snapshot_observer: RefCell::default(),
            console_observer: RefCell::default(),
            macro_details: RefCell::default(),
            macro_form: Rc::default(),
            macro_form_state: RefCell::default(),
            files: Rc::default(),
            spools: Rc::default(),
            console: Rc::default(),
            power_devices: Rc::default(),
            updates: Rc::default(),
            host_interfaces: Rc::default(),
            running_commands: RefCell::default(),
            status: Rc::new(StatusView { window: window.as_weak(), log: RefCell::default(), messages: Rc::default() }),
            status_timer: Timer::default(),
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
        printer.set_mmu_gates(ModelRc::from(presenter.mmu_gates.clone()));
        printer.set_mmu_sensors(ModelRc::from(presenter.mmu_sensors.clone()));
        printer.set_mmu_tools(ModelRc::from(presenter.mmu_tools.clone()));
        printer.set_macros(ModelRc::from(presenter.macros.clone()));
        printer.set_macro_items(ModelRc::from(presenter.macro_items.clone()));
        printer.set_macro_form(ModelRc::from(presenter.macro_form.clone()));
        printer.set_messages(ModelRc::from(presenter.status.messages.clone()));
        printer.set_files(ModelRc::from(presenter.files.clone()));
        printer.set_spools(ModelRc::from(presenter.spools.clone()));
        printer.set_console(ModelRc::from(presenter.console.clone()));
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

    pub fn apply_snapshot(&self, snapshot: &Snapshot) {
        let Some(window) = self.window.upgrade() else { return };
        if let Some(observer) = self.snapshot_observer.borrow().as_ref() {
            observer(snapshot);
        }
        let printer = window.global::<Printer>();
        let status = &snapshot.status;
        let connected = snapshot.connection == ConnectionState::Connected;
        let ready = connected && status.klippy_state == KlippyState::Ready;

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
        self.set_extrude_lengths(&window, status.extruder.extrude_lengths());

        let nozzle = status.heater("extruder");
        printer.set_nozzle_temperature(nozzle.map_or(0.0, |heater| heater.temperature) as f32);
        printer.set_nozzle_target(nozzle.map_or(0.0, |heater| heater.target) as f32);
        let bed = status.heater("heater_bed");
        printer.set_has_bed(bed.is_some());
        printer.set_bed_temperature(bed.map_or(0.0, |heater| heater.temperature) as f32);
        printer.set_bed_target(bed.map_or(0.0, |heater| heater.target) as f32);
        let klipper = &snapshot.sources.klipper;
        let main_led = light::main_led(&snapshot.sources, &status.leds);
        printer.set_has_light(main_led.is_some());
        printer.set_light_name(main_led.map(|led| led.label.as_str()).unwrap_or_default().into());
        printer.set_light_on(main_led.is_some_and(|led| led.on));
        replace_rows(&self.fans, status.fans.iter().map(fan_item).collect());
        replace_rows(&self.leds, status.leds.iter().map(led_item).collect());
        replace_rows(&self.led_effects, status.led_effects.iter().map(led_effect_item).collect());
        replace_rows(&self.tuning, tuning::values(status, &klipper.tuning).iter().map(tuning_item).collect());
        printer.set_save_config_pending(status.save_config_pending);
        printer.set_leveling_state(match status.leveling_applied {
            Some(true) => "applied".into(),
            Some(false) => "pending".into(),
            None => SharedString::new(),
        });

        let lanes = self.lanes(snapshot);
        let active_spool = spools::active_spool(&lanes).cloned();
        replace_rows(
            &self.heaters,
            status
                .heaters
                .iter()
                .map(|heater| heater_item(heater, active_spool.as_ref(), self.history.has_history(&heater.object)))
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
        self.print_objects.show(&printer, &status.print_objects, status.motion.bed());

        printer.set_can_quad_gantry_level(klipper.leveling == Some(Leveling::QuadGantry));
        printer.set_can_z_tilt(klipper.leveling == Some(Leveling::ZTilt));
        printer.set_has_probe(klipper.probe);
        self.calibration.show(&printer, klipper, &status.calibration);
        let macros = match filament::handling(&snapshot.sources, status) {
            FilamentHandling::Macros(macros) => Some(macros),
            FilamentHandling::Mmu | FilamentHandling::None => None,
        };
        printer.set_has_load_macro(macros.is_some_and(|macros| macros.load));
        printer.set_has_unload_macro(macros.is_some_and(|macros| macros.unload));
        printer.set_has_power(snapshot.sources.moonraker.power);
        printer.set_has_update_manager(snapshot.sources.moonraker.update_manager);
        self.set_macro_details(&klipper.macro_details);
        self.set_macros(&klipper.macros, snapshot.connection == ConnectionState::Connected);
        self.set_mmu(status, &lanes);
        self.set_spool_views(&printer, snapshot, active_spool.as_ref());

        let previous_job =
            std::mem::replace(&mut self.cache.borrow_mut().snapshot, snapshot.clone()).status.job.filename;
        if previous_job != status.job.filename {
            self.show_job_thumbnail();
        }
        self.refresh_job_texts();
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
        let texts =
            view::job_texts(job, cache.catalog.metadata(&job.filename), cache.catalog.last_print(&job.filename), now);
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
            Event::ConsoleHistory(lines) => {
                let items: Vec<ConsoleItem> =
                    lines.iter().filter(|line| self.shows_in_console(line)).flat_map(console_items).collect();
                let skip = items.len().saturating_sub(CONSOLE_LIMIT);
                self.console.set_vec(items.into_iter().skip(skip).collect::<Vec<_>>());
            }
            Event::ConsoleLine(line) => {
                if let Some(observer) = self.console_observer.borrow().as_ref() {
                    observer(&line);
                }
                let shown = if self.shows_in_console(&line) { console_items(&line) } else { Vec::new() };
                for item in shown {
                    if self.console.row_count() >= CONSOLE_LIMIT {
                        self.console.remove(0);
                    }
                    self.console.push(item);
                }
                if line.kind == ConsoleKind::Response {
                    self.show_console_response(&line);
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
            Event::CommandFailed { command, message } => {
                let shown = format!("{command}: {message}");
                let id = self.status.log.borrow_mut().add_command_failure(&shown, &message, now_seconds());
                self.show_message(id, &shown, true);
            }
            Event::LoginFailed(message) => {
                if let Some(window) = self.window.upgrade() {
                    window.global::<Printer>().set_login_error(message.into());
                }
            }
            Event::CommandRunning { id, command } => {
                self.running_commands.borrow_mut().push((id, command));
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

    /// Applies the hidden macros from the settings.
    /// Hides or shows the temperature reports in the console; the history is read again so that it follows.
    pub fn set_hide_temperature_reports(&self, hide: bool) {
        if self.hide_temperature_reports.replace(hide) != hide {
            self.service.query(Query::ConsoleHistory);
        }
    }

    fn shows_in_console(&self, line: &ConsoleLine) -> bool {
        !(self.hide_temperature_reports.get() && line.is_temperature_report())
    }

    pub fn set_hidden_macros(&self, hidden: Vec<String>) {
        *self.hidden_macros.borrow_mut() = hidden;
        self.refresh_macro_views();
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
        *self.macros_loaded.borrow_mut() = Some(Box::new(handler));
    }

    /// Happy Hare on the Filament page while it has the say; an enabled one that is too old gets a notice instead.
    fn set_mmu(&self, status: &PrinterStatus, lanes: &[Lane]) {
        let Some(window) = self.window.upgrade() else { return };
        let printer = window.global::<Printer>();
        let in_charge = status.happy_hare();
        printer.set_has_mmu(in_charge.is_some());
        printer.set_mmu_unsupported(status.mmu.as_ref().is_some_and(MmuStatus::is_too_old));
        let Some(mmu) = in_charge else {
            replace_rows(&self.mmu_gates, Vec::new());
            replace_rows(&self.mmu_sensors, Vec::new());
            replace_rows(&self.mmu_tools, Vec::new());
            return;
        };

        let matches = self.job_matches(status, lanes);
        let gates = mmu
            .gates
            .iter()
            .map(|gate| {
                let spool = gate_spool(lanes, gate.index);
                let (colors, longitudinal) = gate_colors(gate, spool);
                let bar = color_bar(&colors);
                MmuGateItem {
                    spool_text: spool.map(view::spool_remaining).unwrap_or_default().into(),
                    spool_low: spool.is_some_and(Spool::is_low),
                    spool_id: gate.spool_id.map_or(-1, |id| id as i32),
                    index: gate.index as i32,
                    availability: view::mmu_availability(gate.availability).into(),
                    name: gate.name.as_str().into(),
                    material: gate.material.as_str().into(),
                    color: bar.0,
                    color_2: bar.1,
                    color_3: bar.2,
                    color_count: bar.3,
                    longitudinal,
                    has_color: gate.color.is_some(),
                    tools: view::mmu_tools(&gate.tools).into(),
                    file_verdict: view::combined_verdict(
                        matches.iter().filter(|found| found.gate == Some(gate.index)).map(|found| &found.verdict),
                    )
                    .into(),
                    group: gate.endless_spool_group.map_or(-1, |group| group as i32),
                    selected: usize::try_from(mmu.gate) == Ok(gate.index),
                }
            })
            .collect();
        replace_rows(&self.mmu_gates, gates);
        self.set_mmu_tools(&printer, &mmu.tool_to_gate, &matches, status, lanes);
        printer.set_mmu_bypass(mmu.bypass_selected());
        printer.set_mmu_endless_spool(mmu.endless_spool);
        printer.set_mmu_gate_map_local(mmu.gate_map_is_local());
        let selected = usize::try_from(mmu.gate).ok().and_then(|index| mmu.gates.get(index));
        let (colors, longitudinal) =
            selected.map(|gate| gate_colors(gate, gate_spool(lanes, gate.index))).unwrap_or_default();
        printer.set_mmu_path(filament_path_item(&mmu.filament_path(), mmu.gate, &colors, longitudinal));
        let has_buffer = mmu.sync_feedback.is_some();
        let sensors = view::mmu_chip_sensors(&mmu.sensors, has_buffer)
            .into_iter()
            .map(|sensor| MmuSensorItem {
                label: view::mmu_sensor_label(&sensor.name).into(),
                state: view::mmu_sensor_state(sensor.triggered).into(),
            })
            .collect();
        replace_rows(&self.mmu_sensors, sensors);

        printer.set_mmu_filament(view::mmu_filament(mmu.filament).into());
        printer.set_mmu_action(mmu.action.as_str().into());
        printer.set_mmu_gate(mmu.gate.max(-1));
        printer.set_mmu_tool(mmu.tool.max(-1));
        printer.set_mmu_paused(mmu.paused);
        printer.set_mmu_locked(mmu.locked);
        printer.set_mmu_homed(mmu.homed);
        printer.set_mmu_servo(view::mmu_servo(mmu.servo).into());
        printer.set_mmu_has_buffer(has_buffer);
        let feedback = mmu.sync_feedback.unwrap_or_default();
        printer.set_mmu_buffer_state(view::mmu_buffer_state(feedback.state).into());
        printer.set_mmu_buffer_bias(feedback.bias as f32);
        let flowguard = mmu.flowguard.clone().unwrap_or_default();
        printer.set_mmu_flowguard_state(view::mmu_flowguard_state(flowguard.state()).into());
        let detail: Vec<&str> = [flowguard.trigger.as_str(), flowguard.reason.as_str()]
            .into_iter()
            .filter(|part| !part.is_empty())
            .collect();
        printer.set_mmu_flowguard_detail(detail.join(": ").into());
        self.note_origin("MMU flow rate", mmu.flow_rate.map(|rate| rate.origin));
        printer.set_mmu_flow_rate(mmu.flow_rate.map_or(-1.0, |rate| rate.value as f32));
    }

    /// How each tool of the job's file fits the filament it would print with; empty without file or metadata.
    fn job_matches(&self, status: &PrinterStatus, lanes: &[Lane]) -> Vec<ToolMatch> {
        let cache = self.cache.borrow();
        cache
            .catalog
            .metadata(&status.job.filename)
            .map(|metadata| filament_match::match_tools(metadata, status, lanes))
            .unwrap_or_default()
    }

    /// The tools of Happy Hare with what the job's file asks of them, then the file's tools that have no gate; and the
    /// line about the file above them.
    fn set_mmu_tools(
        &self,
        printer: &Printer,
        tool_to_gate: &[usize],
        matches: &[ToolMatch],
        status: &PrinterStatus,
        lanes: &[Lane],
    ) {
        let mapped = tool_to_gate
            .iter()
            .enumerate()
            .map(|(tool, gate)| (tool, Some(*gate), matches.iter().find(|found| found.request.tool == tool)));
        let without_gate = matches
            .iter()
            .filter(|found| found.verdict == Verdict::NoGate)
            .map(|found| (found.request.tool, None, Some(found)));
        let tools = mapped
            .chain(without_gate)
            .enumerate()
            .map(|(row, (tool, gate, found))| {
                let mut item = mmu_tool_item(tool, gate, found);
                let fitting = found.map(|found| filament_match::fitting_gates(&found.request, status, lanes));
                item.fitting_gates = reuse_flags(self.mmu_tools.row_data(row).map(|old| old.fitting_gates), fitting);
                item
            })
            .collect();
        replace_rows(&self.mmu_tools, tools);
        let file = if matches.is_empty() { String::new() } else { format::job_name(&status.job.filename) };
        printer.set_mmu_match_file(file.into());
        printer.set_mmu_match_summary(view::match_summary(matches).into());
        printer.set_mmu_match_verdict(view::combined_verdict(matches.iter().map(|found| &found.verdict)).into());
    }

    /// Stores the printer's macros. An empty list while disconnected must not count as "all macros vanished".
    fn set_macros(&self, macros: &[String], connected: bool) {
        if self.all_macros.borrow().as_slice() == macros {
            return;
        }
        *self.all_macros.borrow_mut() = macros.to_vec();
        self.refresh_macro_views();
        if connected && !macros.is_empty() {
            if let Some(handler) = self.macros_loaded.borrow().as_ref() {
                handler(macros);
            }
        }
    }

    fn refresh_macro_views(&self) {
        let offered =
            offered_macros(&self.all_macros.borrow(), &self.macro_details.borrow(), &self.hidden_macros.borrow());
        let visible: Vec<SharedString> =
            offered.iter().filter(|offer| offer.visible).map(|offer| SharedString::from(offer.name.as_str())).collect();
        replace_rows(&self.macros, visible);
        let items: Vec<MacroItem> = offered
            .iter()
            .map(|offer| MacroItem { name: SharedString::from(offer.name.as_str()), visible: offer.visible })
            .collect();
        replace_rows(&self.macro_items, items);
    }

    fn set_macro_details(&self, details: &Arc<BTreeMap<String, MacroDetails>>) {
        if Arc::ptr_eq(&self.macro_details.borrow(), details) {
            return;
        }
        *self.macro_details.borrow_mut() = Arc::clone(details);
        self.refresh_macro_views();
    }

    /// Whether the macro takes parameters, so the form has to come before it runs.
    pub fn macro_has_params(&self, name: &str) -> bool {
        self.macro_details.borrow().get(&name.to_ascii_uppercase()).is_some_and(|details| !details.params.is_empty())
    }

    /// Fills the form with the parameters of a macro; empty values are left to Klipper's defaults.
    pub fn open_macro_form(&self, name: &str) {
        let form = MacroForm::open(name, self.macro_details.borrow().get(&name.to_ascii_uppercase()));
        let rows: Vec<MacroParamItem> = form
            .fields()
            .iter()
            .map(|field| MacroParamItem {
                name: SharedString::from(field.name.as_str()),
                placeholder: SharedString::from(field.placeholder.as_str()),
                value: SharedString::from(field.value.as_str()),
                numeric: field.numeric,
            })
            .collect();
        *self.macro_form_state.borrow_mut() = form;
        self.macro_form.set_vec(rows);
        self.show_macro_form_name();
    }

    /// Sets one parameter of the open form.
    pub fn set_macro_param(&self, index: usize, value: &str) {
        let result = self.macro_form_state.borrow_mut().set_value(index, value);
        match result {
            Ok(()) => {
                if let Some(mut row) = self.macro_form.row_data(index) {
                    row.value = SharedString::from(value);
                    self.macro_form.set_row_data(index, row);
                }
            }
            Err(error) => self.show_notice(&error.to_string(), true),
        }
    }

    /// The command for the open form, e.g. `PARK Z=20`.
    pub fn macro_form_script(&self) -> String {
        self.macro_form_state.borrow().script()
    }

    fn show_macro_form_name(&self) {
        let Some(window) = self.window.upgrade() else { return };
        window.global::<Printer>().set_macro_form_name(SharedString::from(self.macro_form_state.borrow().name()));
    }

    /// Empties the shown log only; the history on the printer stays and returns on the next load.
    pub fn clear_console(&self) {
        self.console.set_vec(Vec::new());
    }

    pub fn show_notice(&self, text: &str, is_error: bool) {
        let id = self.status.log.borrow_mut().add(text, is_error, now_seconds());
        self.show_message(id, text, is_error);
    }

    /// Shows a message in the status bar as an event; it leaves again after a while, only the newest one is shown
    /// (hence one timer). Errors stay in the counter until the flyout is opened.
    fn show_message(&self, id: u64, text: &str, is_error: bool) {
        let status = Rc::clone(&self.status);
        self.status_timer.start(TimerMode::SingleShot, display_time(text, is_error), move || {
            status.log.borrow_mut().hide_event(id);
            status.refresh();
        });
        self.status.refresh();
    }

    /// A `!!` (error) or `//` (information) response of Klipper. Other responses stay in the console.
    /// While a print runs, informations only go to the list: Klipper sends many of them and the status bar would flicker.
    fn show_console_response(&self, line: &ConsoleLine) {
        let is_error = line.is_error();
        if !is_error && !line.is_information() {
            return;
        }
        let text = line.text();
        if is_error || !self.is_printing() {
            let added = self.status.log.borrow_mut().add_response(&text, is_error, now_seconds());
            if let Some(id) = added {
                self.show_message(id, &text, is_error);
            }
        } else {
            self.status.log.borrow_mut().add_quietly(&text, false, now_seconds());
            self.status.refresh();
        }
    }

    fn is_printing(&self) -> bool {
        self.cache.borrow().snapshot.status.job.state.is_active()
    }

    pub fn open_messages(&self) {
        self.status.log.borrow_mut().open_list();
        self.status.refresh();
    }

    pub fn close_messages(&self) {
        self.status.log.borrow_mut().close_list();
        self.status.refresh();
    }

    pub fn clear_messages(&self) {
        self.status.log.borrow_mut().clear();
        self.status.refresh();
    }

    fn lanes(&self, snapshot: &Snapshot) -> Vec<Lane> {
        spools::lanes(&snapshot.status, &self.cache.borrow().spoolman)
    }

    /// The spool in use on the page of the print and on the temperature page, and Spoolman's state.
    fn set_spool_views(&self, printer: &Printer, snapshot: &Snapshot, active: Option<&Spool>) {
        let configured = snapshot.sources.moonraker.spoolman;
        let (offline, pending) = {
            let cache = self.cache.borrow();
            (configured && cache.spoolman_known && !cache.spoolman.connected, cache.spoolman.pending_reports)
        };
        printer.set_has_spoolman(configured);
        printer.set_spoolman_offline(offline);
        printer.set_spoolman_pending(pending as i32);
        printer.set_has_spool(active.is_some());
        printer.set_spool_id(active.map_or(-1, |spool| i32::try_from(spool.id).unwrap_or(-1)));
        printer.set_spool_title(active.map(Spool::title).unwrap_or_default().into());
        printer.set_spool_material(active.map(|spool| spool.material.as_str()).unwrap_or_default().into());
        printer.set_spool_weight(active.map(view::spool_remaining).unwrap_or_default().into());
        let bar = color_bar(active.map(|spool| spool.colors.as_slice()).unwrap_or_default());
        printer.set_spool_color(bar.0);
        printer.set_spool_color_2(bar.1);
        printer.set_spool_color_3(bar.2);
        printer.set_spool_color_count(bar.3);
        printer.set_spool_longitudinal(active.is_some_and(|spool| spool.longitudinal && spool.colors.len() > 1));
        printer.set_spool_low(active.is_some_and(Spool::is_low));
        let preset = active.and_then(view::spool_preset_text);
        printer.set_has_spool_preset(preset.is_some());
        printer.set_spool_preset_text(preset.unwrap_or_default().into());
    }

    fn store_spoolman(&self, state: SpoolmanState) {
        {
            let mut cache = self.cache.borrow_mut();
            cache.catalog.store_spools(state.spools.values().cloned());
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
            cache.catalog.store_spools(spools.iter().cloned());
            cache.spool_list = spools;
        }
        self.show_spool_rows();
        self.show_file_rows();
    }

    fn show_spool_rows(&self) {
        let snapshot = self.cache.borrow().snapshot.clone();
        let lanes = self.lanes(&snapshot);
        let rows = spool_rows(&self.cache.borrow().spool_list, &lanes);
        replace_rows(&self.spools, rows);
        if let Some(window) = self.window.upgrade() {
            window.global::<Printer>().set_spools_loading(false);
        }
    }

    /// Asks for the spools of the picker, and for the state, which the picker shows the places from.
    pub fn load_spools(&self) {
        if self.spools.row_count() == 0 {
            if let Some(window) = self.window.upgrade() {
                window.global::<Printer>().set_spools_loading(true);
            }
        }
        self.service.query(Query::Spoolman);
        self.service.query(Query::SpoolList);
    }

    /// Heats to what the spool in use recommends; the bed keeps its target if Spoolman has no temperature for it.
    pub fn apply_spool_preset(&self) {
        let snapshot = self.snapshot();
        let lanes = self.lanes(&snapshot);
        let Some(spool) = spools::active_spool(&lanes) else { return };
        let Some(extruder) = spool.extruder_temp else { return };
        let bed =
            spool.bed_temp.or_else(|| snapshot.status.heater("heater_bed").map(|heater| heater.target)).unwrap_or(0.0);
        self.service.send(Command::ApplyPreset { extruder, bed });
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

    /// What differs between the file and the spools, as lines for the confirmation to start it; empty if nothing.
    /// What was sent to pause at `layer`, in the words of the dialog's button.
    pub fn layer_pause_notice(&self, layer: u32) -> String {
        view::layer_pause_notice(layer, self.cache.borrow().snapshot.status.job.current_layer)
    }

    /// The height `layer` of the running print will have, for the layer pause dialog.
    pub fn height_at_layer_text(&self, layer: i32) -> String {
        let Ok(layer) = u32::try_from(layer) else { return String::new() };
        let cache = self.cache.borrow();
        let job = &cache.snapshot.status.job;
        view::height_at_layer(job, cache.catalog.metadata(&job.filename), layer)
    }

    pub fn print_hints_text(&self, path: &str) -> String {
        let snapshot = self.snapshot();
        let lanes = self.lanes(&snapshot);
        let cache = self.cache.borrow();
        let Some(metadata) = cache.catalog.metadata(path) else { return String::new() };
        let show_tool = lanes.iter().any(|lane| lane.gate.is_some());
        let lines: Vec<String> = filament_match::print_hints(metadata, &snapshot.status, &lanes)
            .iter()
            .map(|hint| format!("• {}", view::print_hint_text(hint, show_tool)))
            .collect();
        if lines.is_empty() {
            String::new()
        } else {
            format!("\n\n{}", lines.join("\n"))
        }
    }

    pub fn request_files(&self) {
        self.service.query(Query::Files);
        self.service.query(Query::PrintHistory);
        self.service.query(Query::SpoolList);
    }

    /// Marks or unmarks a file of the list; marked files can be deleted, a single one can be printed.
    pub fn toggle_file_mark(&self, path: &str) {
        {
            let mut cache = self.cache.borrow_mut();
            if !cache.marked_files.remove(path) {
                cache.marked_files.insert(path.to_owned());
            }
        }
        self.show_file_rows();
    }

    pub fn clear_file_marks(&self) {
        self.cache.borrow_mut().marked_files.clear();
        self.show_file_rows();
    }

    /// The marked files, which are no longer marked afterwards.
    pub fn take_marked_files(&self) -> Vec<String> {
        let marked = std::mem::take(&mut self.cache.borrow_mut().marked_files);
        self.show_file_rows();
        marked.into_iter().collect()
    }

    fn show_files(&self, files: Vec<FileEntry>) {
        {
            let mut cache = self.cache.borrow_mut();
            let listed: BTreeSet<&str> = files.iter().map(|file| file.path.as_str()).collect();
            cache.marked_files.retain(|path| listed.contains(path.as_str()));
            for changed in cache.catalog.set_entries(files) {
                cache.thumbnails.remove(&changed);
            }
        }
        self.show_file_rows();
        let missing = self.cache.borrow().catalog.needing_details();
        for filename in missing {
            self.service.query(Query::FileDetails { filename });
        }
    }

    fn show_file_rows(&self) {
        let rows: Vec<FileItem> = {
            let cache = self.cache.borrow();
            cache.catalog.entries().iter().map(|entry| file_row(&cache, entry)).collect()
        };
        replace_rows(&self.files, rows);
        self.show_file_marks();
    }

    /// How many files are marked, and which one if it is a single one (the one to print).
    fn show_file_marks(&self) {
        let Some(window) = self.window.upgrade() else { return };
        let cache = self.cache.borrow();
        let printer = window.global::<Printer>();
        printer.set_marked_file_count(i32::try_from(cache.marked_files.len()).unwrap_or(i32::MAX));
        let single = match cache.marked_files.iter().collect::<Vec<_>>().as_slice() {
            [path] => Some((*path).clone()),
            _ => None,
        };
        printer.set_marked_file_path(single.as_deref().unwrap_or_default().into());
        printer.set_marked_file_name(single.as_deref().map(format::job_name).unwrap_or_default().into());
    }

    fn store_history(&self, history: PrintHistory) {
        {
            let mut cache = self.cache.borrow_mut();
            cache.catalog.store_last_prints(history.last_prints);
            cache.newest_job = history.newest;
        }
        self.show_file_rows();
        self.refresh_job_texts();
    }

    fn store_metadata(&self, metadata: FileMetadata) {
        let filename = metadata.filename.clone();
        let is_job = {
            let mut cache = self.cache.borrow_mut();
            cache.catalog.store_metadata(metadata);
            cache.snapshot.status.job.filename == filename
        };
        self.update_file_row(&filename);
        if is_job {
            self.refresh_job_texts();
            let snapshot = self.snapshot();
            self.set_mmu(&snapshot.status, &self.lanes(&snapshot));
        }
    }

    fn store_thumbnail(&self, filename: &str, thumbnail: &ThumbnailImage) {
        let Some(image) = thumbnail::to_image(thumbnail) else {
            log::info!("thumbnail of {filename} cannot be shown");
            return;
        };
        self.cache.borrow_mut().thumbnails.insert(filename.to_owned(), image);
        self.update_file_row(filename);
        self.show_job_thumbnail();
    }

    fn update_file_row(&self, filename: &str) {
        let cache = self.cache.borrow();
        let Some(index) = cache.catalog.position(filename) else { return };
        let row = file_row(&cache, &cache.catalog.entries()[index]);
        drop(cache);
        if index < self.files.row_count() {
            self.files.set_row_data(index, row);
        }
    }

    fn show_job_thumbnail(&self) {
        let Some(window) = self.window.upgrade() else { return };
        let printer = window.global::<Printer>();
        let cache = self.cache.borrow();
        match cache.thumbnails.get(&cache.snapshot.status.job.filename) {
            Some(image) => {
                printer.set_job_thumbnail(image.clone());
                printer.set_has_job_thumbnail(true);
            }
            None => printer.set_has_job_thumbnail(false),
        }
    }
}

/// A tool of Happy Hare (`gate` `None` for a tool of the file without gate) with what the file asks of it.
fn mmu_tool_item(tool: usize, gate: Option<usize>, found: Option<&ToolMatch>) -> MmuToolItem {
    let mut item =
        MmuToolItem { tool: tool as i32, gate: gate.map_or(-1, |gate| gate as i32), ..MmuToolItem::default() };
    let Some(found) = found else { return item };
    let request = &found.request;
    item.used = true;
    match request.color {
        Some(FileColor::Rgb([red, green, blue])) => {
            item.wanted_color = slint::Color::from_rgb_u8(red, green, blue);
            item.wanted_has_color = true;
        }
        Some(FileColor::Transparent) => item.wanted_transparent = true,
        None => {}
    }
    item.wanted_material = request.material.clone().unwrap_or_default().into();
    item.wanted_grams = request.grams.map(format::filament_weight).unwrap_or_default().into();
    item.wanted_profile = request.profile.clone().unwrap_or_default().into();
    item.verdict = view::verdict_key(&found.verdict).into();
    item.differences = view::tool_differences(found).into();
    item
}

/// The flags as a model, the old one while they are unchanged: a new model counts as a changed row and would rebuild
/// the tiles on every status update.
fn reuse_flags(old: Option<ModelRc<bool>>, flags: Option<Vec<bool>>) -> ModelRc<bool> {
    let flags = flags.unwrap_or_default();
    match old {
        Some(old) if old.iter().eq(flags.iter().copied()) => old,
        _ => ModelRc::new(VecModel::from(flags)),
    }
}

/// The spool Spoolman knows in a gate.
fn gate_spool(lanes: &[Lane], gate: usize) -> Option<&Spool> {
    lanes.iter().find(|lane| lane.gate == Some(gate)).and_then(|lane| lane.spool.as_ref())
}

/// The colours of a gate and whether they follow each other along the filament: a multi-colour spool of Spoolman
/// shows all of its colours; otherwise the gate has the one colour Happy Hare reports.
fn gate_colors(gate: &MmuGate, spool: Option<&Spool>) -> (Vec<[u8; 3]>, bool) {
    match spool.filter(|spool| spool.colors.len() > 1) {
        Some(spool) => (spool.colors.clone(), spool.longitudinal),
        None => (gate.color.into_iter().collect(), false),
    }
}

fn file_row(cache: &Cache, file: &FileEntry) -> FileItem {
    let info = cache.catalog.info(file);
    let facts = view::file_facts(&info);
    let thumbnail = cache.thumbnails.get(&file.path);
    FileItem {
        path: file.path.as_str().into(),
        name: format::job_name(&file.path).into(),
        modified: facts.modified.into(),
        material: facts.material.into(),
        weight: facts.weight.into(),
        nozzle: facts.nozzle.into(),
        time: facts.time.into(),
        extruder_temperature: facts.extruder_temperature.into(),
        bed_temperature: facts.bed_temperature.into(),
        chamber_temperature: facts.chamber_temperature.into(),
        thumbnail: thumbnail.cloned().unwrap_or_default(),
        has_thumbnail: thumbnail.is_some(),
        show_thumbnail: thumbnail.is_some() || info.thumbnail_expected(),
        marked: cache.marked_files.contains(&file.path),
    }
}
