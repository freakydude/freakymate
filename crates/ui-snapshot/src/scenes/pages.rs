//! Every page in standby: the file list, the prepared job, the pages with their tabs and the MMU dialogs.

use super::mmu::{mmu_bypass, mmu_gate, mmu_service, mmu_tools_and_groups};
use super::temperature::heat_tabs;
use crate::session::Session;
use freakymate::{Actions, Confirm, FilamentTab, Navigation, Page, Printer, TuneTab};
use freakymate_core::sources::klipper::tuning::TuningKey;
use freakymate_core::Command;
use slint::{ComponentHandle, Model, SharedString};
use std::time::Duration;

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
    session.window.global::<Actions>().invoke_prepare_file(SharedString::new());
    session.pump(Duration::from_millis(100));
    Ok(())
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
            mmu_service(session)?;
            mmu_gate(session)?;
            mmu_tools_and_groups(session)?;
            mmu_bypass(session)?;
            session.window.global::<Navigation>().set_filament_tab(FilamentTab::Extruder);
            session.show_page(Page::Filament, "filament-extruder")?;
            session.window.global::<Navigation>().set_filament_tab(FilamentTab::Gates);
        }
    }
    messages(session)?;
    command_running(session)
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
