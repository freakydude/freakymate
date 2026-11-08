//! The console list: the lines of Klipper's console as rows of fixed height, and the filter for temperature reports.

use crate::{format, ConsoleItem, Printer};
use freakymate_core::sources::moonraker_server::gcode_store::{ConsoleKind, ConsoleLine};
use slint::{Model, ModelRc, VecModel};
use std::cell::Cell;
use std::rc::Rc;

/// The oldest rows are dropped beyond this many.
const CONSOLE_LIMIT: usize = 300;

pub struct ConsoleView {
    rows: Rc<VecModel<ConsoleItem>>,
    /// Leave Klipper's temperature reports out of the console (setting).
    hide_temperature_reports: Cell<bool>,
}

impl Default for ConsoleView {
    fn default() -> Self {
        Self { rows: Rc::default(), hide_temperature_reports: Cell::new(true) }
    }
}

impl ConsoleView {
    pub fn bind(&self, printer: &Printer) {
        printer.set_console(ModelRc::from(self.rows.clone()));
    }

    /// Shows the history instead of the rows so far.
    pub fn show_history(&self, lines: &[ConsoleLine]) {
        let items: Vec<ConsoleItem> = lines.iter().filter(|line| self.shows(line)).flat_map(console_items).collect();
        let skip = items.len().saturating_sub(CONSOLE_LIMIT);
        self.rows.set_vec(items.into_iter().skip(skip).collect::<Vec<_>>());
    }

    /// Appends the rows of a line that arrived live.
    pub fn push(&self, line: &ConsoleLine) {
        let shown = if self.shows(line) { console_items(line) } else { Vec::new() };
        for item in shown {
            if self.rows.row_count() >= CONSOLE_LIMIT {
                self.rows.remove(0);
            }
            self.rows.push(item);
        }
    }

    /// Empties the shown log only.
    pub fn clear(&self) {
        self.rows.set_vec(Vec::new());
    }

    /// Whether the setting changed.
    pub fn set_hide_temperature_reports(&self, hide: bool) -> bool {
        self.hide_temperature_reports.replace(hide) != hide
    }

    fn shows(&self, line: &ConsoleLine) -> bool {
        !(self.hide_temperature_reports.get() && line.is_temperature_report())
    }
}

/// One row per message line: the console list has fixed row heights, so a line break inside a row would overlap its neighbours.
/// Only the first row of a message shows the time.
fn console_items(line: &ConsoleLine) -> Vec<ConsoleItem> {
    let time = format::unix_clock(line.time);
    let is_command = line.kind == ConsoleKind::Command;
    let is_error = line.is_error();
    let text = line.text();
    let mut rows: Vec<&str> = text.lines().collect();
    if rows.is_empty() {
        rows.push("");
    }
    rows.into_iter()
        .enumerate()
        .map(|(index, text)| ConsoleItem {
            text: text.into(),
            time: if index == 0 { time.as_str().into() } else { "".into() },
            is_command,
            is_error,
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn line(kind: ConsoleKind, message: &str) -> ConsoleLine {
        ConsoleLine { kind, message: message.to_owned(), time: 0.0 }
    }

    #[test]
    fn multi_line_message_becomes_one_row_per_line() {
        let rows = console_items(&line(ConsoleKind::Response, "first\nsecond\r\nthird"));
        let texts: Vec<&str> = rows.iter().map(|row| row.text.as_str()).collect();
        assert_eq!(texts, ["first", "second", "third"]);
        assert!(rows[1].time.is_empty() && rows[2].time.is_empty());
    }

    #[test]
    fn empty_message_keeps_one_row() {
        let rows = console_items(&line(ConsoleKind::Response, ""));
        assert_eq!(rows.len(), 1);
        assert!(rows[0].text.is_empty());
    }

    #[test]
    fn plugin_markup_does_not_reach_the_console() {
        let rows = console_items(&line(
            ConsoleKind::Response,
            r#"// <b><span style="color:#87CEEB">EMPTY</span></b>&nbsp;gate"#,
        ));
        assert_eq!(rows[0].text, "EMPTY gate");
    }

    #[test]
    fn error_flag_applies_to_every_row() {
        let rows = console_items(&line(ConsoleKind::Response, "!! Lost connection\nsee log"));
        assert_eq!(rows[0].text, "Lost connection");
        assert!(rows.iter().all(|row| row.is_error));
        assert!(!rows.iter().any(|row| row.is_command));
    }
}
