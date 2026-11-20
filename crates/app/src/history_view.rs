//! The temperature history overlay: which heater or sensor it shows and how the values reach the UI. Only what
//! Moonraker keeps in its temperature store (`server.temperature_store`) is shown; objects it does not keep (for
//! example stepper drivers) have no history.

use crate::{AppWindow, ChartLabel, Navigation, PidValueItem, Printer, chart, format, items};
use sailhand_core::domain::pid_tuning::PidTuning;
use sailhand_core::sources::PrinterStatus;
use sailhand_core::sources::klipper::PidValues;
use sailhand_core::sources::moonraker_server::temperature_store::TemperatureSeries;
use slint::{ComponentHandle, ModelRc, SharedString, VecModel};
use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::Rc;

/// Moonraker does not report changes of its store; an open chart asks for it again after this many seconds.
const REFRESH_SECONDS: u32 = 5;

struct Shown {
    object: String,
    label: String,
    series: TemperatureSeries,
}

pub struct HistoryView {
    window: slint::Weak<AppWindow>,
    shown: RefCell<Option<Shown>>,
    seconds_since_refresh: Cell<u32>,
    pid_result: Rc<VecModel<PidValueItem>>,
    /// The target of this display's `PID_CALIBRATE` for the shown heater while Klipper has not answered it.
    pid_running: Cell<Option<f64>>,
}

impl HistoryView {
    pub fn new(window: &AppWindow) -> Self {
        let pid_result = Rc::<VecModel<PidValueItem>>::default();
        window.global::<Printer>().set_history_pid_result(ModelRc::from(pid_result.clone()));
        Self {
            window: window.as_weak(),
            shown: RefCell::default(),
            seconds_since_refresh: Cell::default(),
            pid_result,
            pid_running: Cell::default(),
        }
    }

    /// Opens the chart; the values come with the next answer of the store.
    pub fn open(&self, object: &str, status: &PrinterStatus) {
        self.pid_running.set(None);
        *self.shown.borrow_mut() = Some(Shown {
            object: object.to_owned(),
            label: label(status, object),
            series: TemperatureSeries::default(),
        });
        self.seconds_since_refresh.set(0);
        if let Some(window) = self.window.upgrade() {
            window.global::<Navigation>().set_history_open(true);
        }
        self.render(status);
        self.show_pid(status);
    }

    /// The tunings of this display that wait for Klipper's answer, as (heater, target); only the shown heater's counts.
    /// While one runs the chart shows the short window its swings need.
    pub fn set_pid_runs<'a>(&self, runs: impl Iterator<Item = (&'a str, f64)>, status: &PrinterStatus) {
        let running = {
            let shown = self.shown.borrow();
            let object = shown.as_ref().map(|shown| shown.object.as_str());
            runs.filter(|(heater, _)| Some(*heater) == object).last().map(|(_, target)| target)
        };
        if running == self.pid_running.replace(running) {
            return;
        }
        if let Some(window) = self.window.upgrade() {
            window.global::<Printer>().set_history_pid_running_target(running.unwrap_or_default() as f32);
        }
        self.render(status);
    }

    /// Called with every snapshot: the PID values in printer.cfg and those waiting for `SAVE_CONFIG` change on their
    /// own, also when the tuning was started elsewhere.
    pub fn show_pid(&self, status: &PrinterStatus) {
        let Some(window) = self.window.upgrade() else { return };
        let shown = self.shown.borrow();
        let Some(shown) = shown.as_ref() else { return };
        let printer = window.global::<Printer>();
        let tuning = PidTuning::of(status, &shown.object);
        printer.set_history_pid_available(tuning.is_some());
        let Some(tuning) = tuning else { return };
        printer.set_history_pid_target(tuning.default_target.unwrap_or_default() as f32);
        printer.set_history_pid_range(status.heater(&shown.object).map(items::target_range).unwrap_or_default());
        printer.set_history_pid_current(tuning.current.map(current_values_text).unwrap_or_default().into());
        printer.set_history_pid_has_result(tuning.pending.is_some());
        let result = tuning.pending.map(|pending| pid_items(pending, tuning.current)).unwrap_or_default();
        items::replace_rows(&self.pid_result, result);
    }

    pub fn close(&self) {
        *self.shown.borrow_mut() = None;
        if let Some(window) = self.window.upgrade() {
            window.global::<Navigation>().set_history_open(false);
        }
    }

    /// Called once a second: true when an open chart should ask the store again.
    pub fn refresh_due(&self) -> bool {
        if self.shown.borrow().is_none() {
            return false;
        }
        let seconds = self.seconds_since_refresh.get() + 1;
        let due = seconds >= REFRESH_SECONDS;
        self.seconds_since_refresh.set(if due { 0 } else { seconds });
        due
    }

    /// Shows the open chart's series; Moonraker answers `{}` for a moment after Klipper became ready, and the next
    /// refresh fills the chart.
    pub fn store(&self, mut store: HashMap<String, TemperatureSeries>, status: &PrinterStatus) {
        {
            let mut shown = self.shown.borrow_mut();
            let Some(shown) = shown.as_mut() else { return };
            shown.series = store.remove(&shown.object).unwrap_or_default();
        }
        self.render(status);
    }

    fn render(&self, status: &PrinterStatus) {
        let Some(window) = self.window.upgrade() else { return };
        let printer = window.global::<Printer>();
        let shown = self.shown.borrow();
        let Some(shown) = shown.as_ref() else { return };
        let series = &shown.series;
        let has_target = series.targets.iter().any(|target| *target > 0.0);
        let window_seconds =
            if self.pid_running.get().is_some() { chart::SHORT_WINDOW_SECONDS } else { chart::FULL_WINDOW_SECONDS };
        let temperatures = chart::visible(&series.temperatures, window_seconds);
        let targets = if has_target { chart::visible(&series.targets, window_seconds) } else { &[] };
        let scale = chart::scale(temperatures, targets);
        let aspect = f64::from(window.global::<Navigation>().get_history_aspect()).max(0.1);
        let grid = chart::grid(scale, aspect, window_seconds);
        printer.set_history_object(shown.object.as_str().into());
        printer.set_history_title(shown.label.as_str().into());
        printer.set_history_current(current_text(status, &shown.object).into());
        printer.set_history_temperature_path(chart::line(&series.temperatures, scale, aspect, window_seconds).into());
        let target =
            if has_target { chart::line(&series.targets, scale, aspect, window_seconds) } else { String::new() };
        printer.set_history_target_path(target.into());
        let power = if has_target { chart::power_line(&series.powers, aspect, window_seconds) } else { String::new() };
        printer.set_history_power_path(power.into());
        printer.set_history_grid_path(grid.path.into());
        printer.set_history_temperature_labels(labels(grid.temperatures));
        printer.set_history_time_labels(labels(grid.times));
    }
}

/// `Kp 21.527 · Ki 1.063 · Kd 108.982`: the digits Klipper writes (`PID_CALIBRATE` rounds to three decimals).
fn current_values_text(values: PidValues) -> String {
    format!("Kp {:.3} · Ki {:.3} · Kd {:.3}", values.kp, values.ki, values.kd)
}

fn pid_items(pending: PidValues, current: Option<PidValues>) -> Vec<PidValueItem> {
    let item = |name: &str, pending: f64, current: Option<f64>| PidValueItem {
        name: name.into(),
        current: current.map(|value| format!("{value:.3}")).unwrap_or_default().into(),
        pending: format!("{pending:.3}").into(),
    };
    vec![
        item("Kp", pending.kp, current.map(|values| values.kp)),
        item("Ki", pending.ki, current.map(|values| values.ki)),
        item("Kd", pending.kd, current.map(|values| values.kd)),
    ]
}

fn labels(labels: Vec<(String, f32)>) -> ModelRc<ChartLabel> {
    let labels: Vec<ChartLabel> =
        labels.into_iter().map(|(text, position)| ChartLabel { text: SharedString::from(text), position }).collect();
    ModelRc::from(Rc::new(VecModel::from(labels)))
}

fn label(status: &PrinterStatus, object: &str) -> String {
    status
        .heater(object)
        .map(|heater| heater.label.clone())
        .or_else(|| status.sensors.iter().find(|sensor| sensor.object == object).map(|sensor| sensor.label.clone()))
        .unwrap_or_else(|| object.to_owned())
}

/// E.g. `212.4 °C / 215 °C`, or `–` while there is no value.
fn current_text(status: &PrinterStatus, object: &str) -> String {
    let (temperature, target) = match status.heater(object) {
        Some(heater) => (Some(heater.temperature), Some(heater.target)),
        None => {
            (status.sensors.iter().find(|sensor| sensor.object == object).and_then(|sensor| sensor.temperature), None)
        }
    };
    let Some(temperature) = temperature else { return "–".to_owned() };
    match target.filter(|target| *target > 0.0) {
        Some(target) => format::temperature_of(temperature, target),
        None => format::temperature(temperature, 1),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sailhand_core::sources::klipper::{Heater, TemperatureSensor};

    #[test]
    fn the_current_value_names_the_target_only_when_heating() {
        let status = PrinterStatus {
            heaters: vec![Heater {
                object: "extruder".into(),
                temperature: 212.44,
                target: 215.0,
                ..Heater::default()
            }],
            sensors: vec![TemperatureSensor {
                object: "tmc2240 stepper_x".into(),
                label: "Stepper X".into(),
                temperature: None,
                has_history: false,
                ..TemperatureSensor::default()
            }],
            ..PrinterStatus::default()
        };
        assert_eq!(current_text(&status, "extruder"), "212.4 / 215 °C");
        assert_eq!(current_text(&status, "tmc2240 stepper_x"), "–");
        assert_eq!(label(&status, "tmc2240 stepper_x"), "Stepper X");
    }

    #[test]
    fn pid_values_show_three_decimals_and_the_old_ones_beside_the_new() {
        let current = PidValues { kp: 21.527, ki: 1.063, kd: 108.982 };
        let found = PidValues { kp: 23.104, ki: 1.152, kd: 115.84 };
        assert_eq!(current_values_text(current), "Kp 21.527 · Ki 1.063 · Kd 108.982");

        let items = pid_items(found, Some(current));
        assert_eq!(
            (items[0].name.as_str(), items[0].current.as_str(), items[0].pending.as_str()),
            ("Kp", "21.527", "23.104")
        );
        assert_eq!(items[2].pending.as_str(), "115.840");
        assert_eq!(pid_items(found, None)[1].current.as_str(), "", "no old value for a heater without PID");
    }
}
