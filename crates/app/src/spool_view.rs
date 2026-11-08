//! Spoolman on screen: the spool in use on the print and temperature pages, and the picker's rows.

use crate::items::{id_or_none, replace_rows, spool_colors, spool_rows};
use crate::{view, Printer, SpoolItem};
use freakymate_core::domain::spools::Lane;
use freakymate_core::sources::spoolman::Spool;
use slint::{Model, ModelRc, VecModel};
use std::rc::Rc;

/// What the UI is told about Spoolman itself.
pub struct SpoolmanLine {
    /// The printer's Moonraker is configured for Spoolman.
    pub configured: bool,
    /// Configured but known not to answer.
    pub offline: bool,
    /// Weight reports waiting for Spoolman to come back.
    pub pending_reports: usize,
}

#[derive(Default)]
pub struct SpoolView {
    rows: Rc<VecModel<SpoolItem>>,
}

impl SpoolView {
    pub fn bind(&self, printer: &Printer) {
        printer.set_spools(ModelRc::from(self.rows.clone()));
    }

    /// The spool in use and Spoolman's state.
    pub fn show_active(&self, printer: &Printer, spoolman: &SpoolmanLine, active: Option<&Spool>) {
        printer.set_has_spoolman(spoolman.configured);
        printer.set_spoolman_offline(spoolman.offline);
        printer.set_spoolman_pending(spoolman.pending_reports as i32);
        printer.set_has_spool(active.is_some());
        printer.set_spool_id(id_or_none(active.map(|spool| spool.id)));
        printer.set_spool_title(active.map(Spool::title).unwrap_or_default().into());
        printer.set_spool_material(active.map(|spool| spool.material.as_str()).unwrap_or_default().into());
        printer.set_spool_weight(active.map(view::spool_remaining).unwrap_or_default().into());
        printer.set_spool_colors(active.map(spool_colors).unwrap_or_default());
        printer.set_spool_low(active.is_some_and(Spool::is_low));
        let preset = active.and_then(view::spool_preset_text);
        printer.set_has_spool_preset(preset.is_some());
        printer.set_spool_preset_text(preset.unwrap_or_default().into());
    }

    /// The picker's rows; the list is no longer loading.
    pub fn show_rows(&self, printer: &Printer, spools: &[Spool], lanes: &[Lane]) {
        replace_rows(&self.rows, spool_rows(spools, lanes));
        printer.set_spools_loading(false);
    }

    /// Shows the picker as loading as long as it has no rows.
    pub fn show_loading_if_empty(&self, printer: &Printer) {
        if self.rows.row_count() == 0 {
            printer.set_spools_loading(true);
        }
    }
}
