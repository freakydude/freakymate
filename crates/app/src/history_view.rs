//! The temperature history overlay: which heater or sensor it shows and how the values reach the UI. Only what
//! Moonraker keeps in its temperature store (`server.temperature_store`) is shown; objects it does not keep (for
//! example stepper drivers) have no history.

use crate::{chart, AppWindow, ChartLabel, Navigation, Printer};
use freakymate_core::sources::moonraker_server::temperature_store::TemperatureSeries;
use freakymate_core::sources::PrinterStatus;
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
}

impl HistoryView {
    pub fn new(window: &AppWindow) -> Self {
        Self { window: window.as_weak(), shown: RefCell::default(), seconds_since_refresh: Cell::default() }
    }

    /// Opens the chart; the values come with the next answer of the store.
    pub fn open(&self, object: &str, status: &PrinterStatus) {
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
        let scale = chart::scale(&series.temperatures, if has_target { &series.targets } else { &[] });
        let aspect = f64::from(window.global::<Navigation>().get_history_aspect()).max(0.1);
        let grid = chart::grid(scale, aspect);
        printer.set_history_title(shown.label.as_str().into());
        printer.set_history_current(current_text(status, &shown.object).into());
        printer.set_history_temperature_path(chart::line(&series.temperatures, scale, aspect).into());
        let target = if has_target { chart::line(&series.targets, scale, aspect) } else { String::new() };
        printer.set_history_target_path(target.into());
        let power = if has_target { chart::power_line(&series.powers, aspect) } else { String::new() };
        printer.set_history_power_path(power.into());
        printer.set_history_grid_path(grid.path.into());
        printer.set_history_temperature_labels(labels(grid.temperatures));
        printer.set_history_time_labels(labels(grid.times));
    }
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
        Some(target) => format!("{temperature:.1} °C / {target:.0} °C"),
        None => format!("{temperature:.1} °C"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use freakymate_core::sources::klipper::{Heater, TemperatureSensor};

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
            }],
            ..PrinterStatus::default()
        };
        assert_eq!(current_text(&status, "extruder"), "212.4 °C / 215 °C");
        assert_eq!(current_text(&status, "tmc2240 stepper_x"), "–");
        assert_eq!(label(&status, "tmc2240 stepper_x"), "Stepper X");
    }
}
