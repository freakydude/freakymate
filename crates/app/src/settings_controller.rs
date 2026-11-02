//! Connects `UiSettings` with the settings file and applies changes immediately.

use crate::network_controller::NetworkController;
use crate::presenter::Presenter;
use crate::{AppWindow, Backlight, PresetItem, UiSettings};
use freakymate_core::backlight::{usable_brightness, BacklightService, BacklightSettings};
use freakymate_core::settings::{Settings, SettingsStore, TemperaturePreset, MAX_SHOWN_PRESETS};
use freakymate_core::{Endpoint, PrinterService};
use slint::{ComponentHandle, ModelRc, VecModel};
use std::cell::RefCell;
use std::rc::Rc;

/// What the settings act on.
pub struct Bridges {
    pub service: PrinterService,
    pub presenter: Rc<Presenter>,
    pub network: Rc<NetworkController>,
    pub backlight: BacklightService,
}

pub struct SettingsController {
    window: slint::Weak<AppWindow>,
    store: SettingsStore,
    /// File contents; an address from the command line is not saved.
    settings: RefCell<Settings>,
    service: PrinterService,
    presenter: Rc<Presenter>,
    network: Rc<NetworkController>,
    backlight: BacklightService,
    presets: Rc<VecModel<PresetItem>>,
}

impl SettingsController {
    pub fn bind(
        window: &AppWindow,
        store: SettingsStore,
        settings: Settings,
        endpoint: &Endpoint,
        bridges: Bridges,
    ) -> Rc<Self> {
        let Bridges { service, presenter, network, backlight } = bridges;
        let controller = Rc::new(Self {
            window: window.as_weak(),
            store,
            settings: RefCell::new(settings),
            service,
            presenter,
            network,
            backlight,
            presets: Rc::default(),
        });
        controller.apply_hidden_macros();
        controller.presenter.set_hide_temperature_reports(controller.settings.borrow().hide_temperature_reports);
        let weak = Rc::downgrade(&controller);
        controller.presenter.on_macros_loaded(move |macros| {
            if let Some(controller) = weak.upgrade() {
                controller.reconcile_macros(macros);
            }
        });
        controller.show(window, endpoint);
        controller.connect_callbacks(window);
        controller
    }

    pub fn preset(&self, index: usize) -> Option<TemperaturePreset> {
        self.settings.borrow().temperature_presets.get(index).cloned()
    }

    pub fn fullscreen(&self) -> bool {
        self.settings.borrow().fullscreen
    }

    fn show(&self, window: &AppWindow, endpoint: &Endpoint) {
        let ui = window.global::<UiSettings>();
        let settings = self.settings.borrow();
        ui.set_moonraker(endpoint.display().into());
        ui.set_fullscreen(settings.fullscreen);
        ui.set_hide_temperature_reports(settings.hide_temperature_reports);
        ui.set_move_step(settings.steps.move_mm as f32);
        ui.set_z_offset_step(settings.steps.z_offset_mm as f32);
        ui.set_temperature_step(settings.steps.temperature as f32);
        ui.set_extrude_length(settings.steps.extrude_mm as f32);
        ui.set_extrude_speed(settings.steps.extrude_speed as f32);
        ui.set_settings_file(self.store.path().display().to_string().into());
        ui.set_version(freakymate_core::VERSION.into());
        let backlight = window.global::<Backlight>();
        backlight.set_idle_minutes(minutes_shown(settings.backlight.off_when_idle_minutes));
        backlight.set_printing_minutes(minutes_shown(settings.backlight.off_when_printing_minutes));
        let presets = settings.temperature_presets.iter().take(MAX_SHOWN_PRESETS).map(preset_item);
        self.presets.set_vec(presets.collect::<Vec<_>>());
        ui.set_presets(ModelRc::from(self.presets.clone()));
    }

    fn connect_callbacks(self: &Rc<Self>, window: &AppWindow) {
        let ui = window.global::<UiSettings>();
        let controller = Rc::downgrade(self);
        ui.on_steps_changed(move || {
            if let Some(controller) = controller.upgrade() {
                controller.store_steps();
            }
        });
        let controller = Rc::downgrade(self);
        ui.on_moonraker_entered(move |address| {
            if let Some(controller) = controller.upgrade() {
                controller.change_moonraker(&address);
            }
        });
        let controller = Rc::downgrade(self);
        ui.on_fullscreen_toggled(move |fullscreen| {
            if let Some(controller) = controller.upgrade() {
                controller.change_fullscreen(fullscreen);
            }
        });
        let controller = Rc::downgrade(self);
        ui.on_hide_temperature_reports_toggled(move |hide| {
            if let Some(controller) = controller.upgrade() {
                controller.update(|settings| settings.hide_temperature_reports = hide);
                controller.presenter.set_hide_temperature_reports(hide);
            }
        });
        let backlight = window.global::<Backlight>();
        let controller = Rc::downgrade(self);
        backlight.on_brightness_chosen(move |percent| {
            if let Some(controller) = controller.upgrade() {
                controller.change_brightness(percent);
            }
        });
        let controller = Rc::downgrade(self);
        backlight.on_idle_minutes_chosen(move |minutes| {
            if let Some(controller) = controller.upgrade() {
                controller.change_timeouts(|settings| settings.off_when_idle_minutes = minutes_setting(minutes));
            }
        });
        let controller = Rc::downgrade(self);
        backlight.on_printing_minutes_chosen(move |minutes| {
            if let Some(controller) = controller.upgrade() {
                controller.change_timeouts(|settings| settings.off_when_printing_minutes = minutes_setting(minutes));
            }
        });
    }

    /// Applies the brightness and keeps it for the next start.
    fn change_brightness(&self, percent: f32) {
        let percent = usable_brightness(percent.round().clamp(0.0, 100.0) as u8);
        self.backlight.set_brightness(percent);
        self.update(|settings| settings.backlight.brightness = Some(percent));
    }

    fn change_timeouts(&self, change: impl FnOnce(&mut BacklightSettings)) {
        self.update(|settings| change(&mut settings.backlight));
        self.backlight.set_timeouts(self.settings.borrow().backlight.timeouts());
    }

    fn store_steps(&self) {
        let Some(window) = self.window.upgrade() else { return };
        let ui = window.global::<UiSettings>();
        self.update(|settings| {
            settings.steps.move_mm = f64::from(ui.get_move_step());
            settings.steps.z_offset_mm = f64::from(ui.get_z_offset_step());
            settings.steps.temperature = f64::from(ui.get_temperature_step());
            settings.steps.extrude_mm = f64::from(ui.get_extrude_length());
            settings.steps.extrude_speed = f64::from(ui.get_extrude_speed());
        });
    }

    fn change_moonraker(&self, address: &str) {
        match Endpoint::parse(address) {
            Ok(endpoint) => {
                let display = endpoint.display();
                self.update(|settings| settings.moonraker.clone_from(&display));
                if let Some(window) = self.window.upgrade() {
                    window.global::<UiSettings>().set_moonraker(display.as_str().into());
                }
                self.network.set_endpoint(&endpoint);
                self.service.connect_to(endpoint);
                self.presenter.show_notice(&format!("Connecting to {display}"), false);
            }
            Err(error) => self.presenter.show_notice(&error.to_string(), true),
        }
    }

    fn change_fullscreen(&self, fullscreen: bool) {
        if let Some(window) = self.window.upgrade() {
            window.window().set_fullscreen(fullscreen);
        }
        self.update(|settings| settings.fullscreen = fullscreen);
    }

    /// Shows a macro in the console or hides it; the choice is kept in the settings file.
    pub fn set_macro_visible(&self, name: &str, visible: bool) {
        self.update(|settings| {
            settings.hidden_macros.retain(|hidden| hidden != name);
            if !visible {
                settings.hidden_macros.push(name.to_owned());
            }
        });
        self.apply_hidden_macros();
    }

    /// Forgets macros the printer no longer has; new macros are known and visible.
    fn reconcile_macros(&self, macros: &[String]) {
        self.update(|settings| {
            settings.reconcile_macros(macros);
        });
        self.apply_hidden_macros();
    }

    fn apply_hidden_macros(&self) {
        self.presenter.set_hidden_macros(self.settings.borrow().hidden_macros.clone());
    }

    fn update(&self, change: impl FnOnce(&mut Settings)) {
        let mut settings = self.settings.borrow_mut();
        change(&mut settings);
        if let Err(error) = self.store.save(&settings) {
            log::warn!("{error}");
            self.presenter.show_notice(&error.to_string(), true);
        }
    }
}

/// The standby time of the settings file as the buttons show it: minutes, 0 for never.
fn minutes_shown(minutes: Option<u32>) -> i32 {
    minutes.map_or(0, |minutes| minutes as i32)
}

fn minutes_setting(minutes: i32) -> Option<u32> {
    u32::try_from(minutes).ok().filter(|minutes| *minutes > 0)
}

fn preset_item(preset: &TemperaturePreset) -> PresetItem {
    PresetItem { name: preset.name.as_str().into(), extruder: preset.extruder as f32, bed: preset.bed as f32 }
}
