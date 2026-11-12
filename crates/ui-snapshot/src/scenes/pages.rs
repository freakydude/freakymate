//! Every page in standby: the file list, the prepared job, the pages with their tabs and the MMU dialogs.

use super::mmu::{mmu_bypass, mmu_gate, mmu_service, mmu_tools_and_groups};
use super::temperature::heat_tabs;
use crate::session::Session;
use freakymate::{Actions, ChangerTab, Confirm, FilamentTab, MmuDialogs, Navigation, Page, Printer, TuneTab};
use freakymate_core::sources::klipper::tuning::TuningKey;
use freakymate_core::Command;
use slint::{ComponentHandle, Model, SharedString};
use std::time::Duration;

/// The chip of T1 in the Tool-to-gate map of the prepared `long_job.gcode` (800x480, Happy Hare).
const PREPARED_CHIP_T1: (f32, f32) = (585.0, 311.0);

/// A file prepared from the list: the print page shows it with its facts and the comparison of its tools (the fake's
/// `long_job.gcode` asks for PLA and PETG). Dropped afterwards, so the other scenes see the printer's job.
pub(super) fn prepared_job(session: &mut Session) -> Result<(), String> {
    let actions = session.window.global::<Actions>();
    actions.invoke_toggle_file_mark("long_job.gcode".into());
    let marked = session.window.global::<Printer>().get_marked_file_path();
    if marked != "long_job.gcode" {
        return Err(format!("expected long_job.gcode as the single marked file, got \"{marked}\""));
    }
    actions.invoke_prepare_file(marked);
    actions.invoke_clear_file_marks();
    session.settle_replies(Duration::from_secs(2));
    let printer = session.window.global::<Printer>();
    if printer.get_prepared_path() != "long_job.gcode" || printer.get_prepared_tools().row_count() == 0 {
        return Err("the prepared file or its tools are missing".into());
    }
    session.show_page(Page::Print, "print-prepared")?;
    if session.has_reference_layout() && session.presenter.snapshot().status.mmu.is_some() {
        tool_chip_opens_its_dialog(session)?;
    }
    session.window.global::<Actions>().invoke_prepare_file(SharedString::new());
    session.pump(Duration::from_millis(100));
    Ok(())
}

/// A tap on a tool of the Tool-to-gate map leads to Changer > Tools with that tool's dialog open.
/// Afterwards back to the print page with the dialog closed, as the later scenes expect.
fn tool_chip_opens_its_dialog(session: &mut Session) -> Result<(), String> {
    session.tap(PREPARED_CHIP_T1.0, PREPARED_CHIP_T1.1);
    session.pump(Duration::from_millis(300));
    let (page, tab) = {
        let navigation = session.window.global::<Navigation>();
        (navigation.get_page(), navigation.get_changer_tab())
    };
    let tool = session.window.global::<MmuDialogs>().get_tool();
    if page != Page::Changer || tab != ChangerTab::Tools || tool != 1 {
        return Err(format!("tap on T1 of the map: expected Changer > Tools with the dialog of T1, got tool {tool}"));
    }
    session.capture("print-prepared-tool-dialog")?;
    session.window.global::<MmuDialogs>().set_tool(-1);
    session.show_page(Page::Print, "print-prepared-back")
}

pub(super) fn pages(session: &mut Session) -> Result<(), String> {
    session.show_page(Page::Print, "print-standby")?;
    session.presenter.request_files();
    session.window.global::<Navigation>().set_files_open(true);
    session.settle_replies(Duration::from_secs(2));
    session.capture("print-files")?;
    // Two marked files: the delete button is enabled and the prepare button waits for a single one.
    for row in 0..2 {
        let path =
            session.window.global::<Printer>().get_files().row_data(row).map(|file| file.path).unwrap_or_default();
        session.window.global::<Actions>().invoke_toggle_file_mark(path);
    }
    session.capture("print-files-marked")?;
    // The question before deleting names the marked files, one row each.
    let names = session.window.global::<Printer>().get_marked_file_names();
    session.window.global::<Confirm>().invoke_ask_for_items(
        "Delete files".into(),
        SharedString::new(),
        names,
        "delete-files".into(),
        SharedString::new(),
        true,
    );
    session.pump(Duration::from_millis(300));
    session.capture("print-files-delete")?;
    session.window.global::<Confirm>().set_open(false);
    session.window.global::<Actions>().invoke_clear_file_marks();
    session.window.global::<Navigation>().set_files_open(false);
    prepared_job(session)?;
    // The console comes before the Thermal page, whose fan test would fill it with the fake's commands.
    for (page, name) in [
        (Page::Move, "move"),
        (Page::Console, "console"),
        (Page::Temperature, "temperature"),
        (Page::Tune, "tune"),
        (Page::Filament, "filament"),
        (Page::System, "system"),
        (Page::Updates, "updates"),
        (Page::Settings, "settings"),
    ] {
        session.show_page(page, name)?;
        if page == Page::Filament {
            head_picker(session)?;
        }
        if page == Page::Temperature {
            heat_tabs(session)?;
        }
        let has_tuning = session.presenter.snapshot().sources.klipper.tuning.get(TuningKey::MaxAccel).is_some();
        if page == Page::Tune && has_tuning {
            session.window.global::<Navigation>().set_tune_tab(TuneTab::Expert);
            session.show_page(Page::Tune, "tune-expert")?;
            session.window.global::<Navigation>().set_tune_tab(TuneTab::Basics);
        }
        if page == Page::Filament && session.presenter.snapshot().status.mmu.is_some() {
            session.window.global::<Navigation>().set_filament_tab(FilamentTab::Gates);
            session.show_page(Page::Filament, "filament-mmu")?;
            unit_pages(session)?;
            mmu_service(session)?;
            mmu_gate(session)?;
            mmu_tools_and_groups(session)?;
            mmu_bypass(session)?;
            session.window.global::<Navigation>().set_filament_tab(FilamentTab::Extruder);
            session.show_page(Page::Filament, "filament-extruder")?;
            session.window.global::<Navigation>().set_filament_tab(FilamentTab::Gates);
        }
    }
    changer_pages(session)?;
    messages(session)?;
    command_running(session)
}

/// The picker over the Filament page lists the heads of a tool changer (`filament-head-picker`); without heads nothing
/// opens.
fn head_picker(session: &mut Session) -> Result<(), String> {
    if session.window.global::<Printer>().get_heads().row_count() == 0 {
        return Ok(());
    }
    session.window.global::<Navigation>().set_head_picker_open(true);
    session.capture("filament-head-picker")?;
    session.window.global::<Navigation>().set_head_picker_open(false);
    Ok(())
}

/// The tabs of the Changer page, each only where the printer has content for it; none on a printer without a changer
/// (the entry is not there either).
fn changer_pages(session: &mut Session) -> Result<(), String> {
    let (tools, service, sensors) = {
        let printer = session.window.global::<Printer>();
        (printer.get_has_changer_tools(), printer.get_has_changer_service(), printer.get_has_changer_sensors())
    };
    for (tab, present, name) in [
        (ChangerTab::Tools, tools, "changer-tools"),
        (ChangerTab::Service, service, "changer-service"),
        (ChangerTab::Sensors, sensors, "changer-sensors"),
    ] {
        if present {
            session.window.global::<Navigation>().set_changer_tab(tab);
            session.show_page(Page::Changer, name)?;
        }
    }
    session.window.global::<Navigation>().set_changer_tab(ChangerTab::Tools);
    Ok(())
}

/// A command that waits long for its answer: the status bar shows the turning wheel and the command until the answer
/// is there. The fake answers `G4` only after the dwell.
pub(super) fn command_running(session: &mut Session) -> Result<(), String> {
    session.service.send(Command::RunGcode { script: "G4 P2500".into() });
    session.pump(Duration::from_millis(1300));
    if session.window.global::<Printer>().get_running_command() != "G4 P2500" {
        return Err("the status bar does not show the running command".into());
    }
    session.show_page(Page::Print, "command-running")?;
    session.pump(Duration::from_millis(1500));
    if !session.window.global::<Printer>().get_running_command().is_empty() {
        return Err("the status bar still shows the command after its answer".into());
    }
    Ok(())
}

/// An error from a command the printer refuses: first an event in the field of the status bar with the counter, then
/// the flyout, which resets the counter, and after closing it nothing is unread.
pub(super) fn messages(session: &mut Session) -> Result<(), String> {
    session.service.send(Command::RunGcode { script: "SET_FAN_SPEED FAN=missing SPEED=1".into() });
    session.pump(Duration::from_secs(1));
    let printer = session.window.global::<Printer>();
    if printer.get_unread_errors() < 1 || !printer.get_status_message_visible() {
        return Err("the status bar shows neither the error nor the counter".into());
    }
    session.capture("messages-event")?;
    let navigation = session.window.global::<Navigation>();
    navigation.invoke_open_messages();
    session.pump(Duration::from_millis(200));
    if session.window.global::<Printer>().get_unread_errors() != 0 {
        return Err("the counter is not reset by opening the list".into());
    }
    session.capture("messages")?;
    session.window.global::<Navigation>().invoke_close_messages();
    session.pump(Duration::from_millis(200));
    if session.window.global::<Printer>().get_messages().row_data(0).is_some_and(|message| message.new) {
        return Err("the messages are still new after closing the list".into());
    }
    // The scenes after it should not show this made-up error in the status bar.
    session.presenter.clear_messages();
    session.pump(Duration::from_millis(200));
    Ok(())
}

/// With several units the Gates tab has a page per unit: the tab opens on the unit of the loaded gate, a chevron
/// leads over the unit's boundary; `filament-units-<n>` is page n. Skipped for one unit and with heads (see the
/// scene `toolchanger`).
fn unit_pages(session: &mut Session) -> Result<(), String> {
    let printer = session.window.global::<Printer>();
    let ranges = printer.get_mmu_page_ranges();
    let pages = printer.get_mmu_pages();
    let Some(range) = ranges.row_data(0).filter(|_| ranges.row_count() == 1 && printer.get_heads().row_count() == 0)
    else {
        return Ok(());
    };
    let first = usize::try_from(range.first).unwrap_or_default();
    let unit_of = |page: i32| pages.row_data(first + page as usize).map(|page| page.unit.to_string());
    if (0..range.count).filter_map(unit_of).all(|unit| unit.is_empty()) {
        return Ok(());
    }
    for page in 0..range.count {
        session.window.global::<Navigation>().set_mmu_page(page);
        session.capture(&format!("filament-units-{page}"))?;
    }
    session.window.global::<Navigation>().set_mmu_page(-1);
    Ok(())
}
