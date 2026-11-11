//! A tool changer with three heads (fake profile `toolchanger`): the spool in use, the temperature hints, the picker for
//! a tool macro and the prepared file with its further tools; on every profile with a tool changer the head level of the
//! Filament page (`toolchanger-mmu`: heads with a unit of their own).

use super::pages::prepared_job;
use crate::session::Session;
use freakymate::{Confirm, FilamentTab, Navigation, Page, Printer, SpoolPicker};
use freakymate_core::Command;
use slint::ComponentHandle;
use slint::Model;
use std::time::Duration;

/// The tab of head T2 and the button at the top right of the head view, in the 800x480 layout.
const HEAD_TAB_T2: (f32, f32) = (310.0, 90.0);
const HEAD_ACTION: (f32, f32) = (730.0, 140.0);
/// The button of the line under the tabs (Initialize, Recover).
const NOTE_BUTTON: (f32, f32) = (730.0, 145.0);

/// Without a tool changer or without Spoolman the scene is skipped.
pub(super) fn toolchanger(session: &mut Session) -> Result<(), String> {
    session.settle_replies(Duration::from_secs(2));
    let snapshot = session.presenter.snapshot();
    if snapshot.status.toolchanger.is_none() || !snapshot.sources.moonraker.spoolman {
        return Ok(());
    }
    start_state(session)?;
    if snapshot.status.mmu.is_some() {
        return head_level(session);
    }
    session.presenter.request_files();
    session.window.global::<Navigation>().set_files_open(true);
    session.settle_replies(Duration::from_secs(2));
    session.window.global::<Navigation>().set_files_open(false);
    // Off from what the spools recommend, so that both nozzles show the hint.
    for (heater, target) in [("extruder", 190.0), ("extruder1", 260.0)] {
        session.service.send(Command::SetHeaterTarget { heater: heater.into(), target });
    }
    session.pump(Duration::from_secs(1));
    if session.window.global::<Printer>().get_nozzle_head().is_empty() {
        return Err("the status bar should name the head of the nozzle it shows".into());
    }
    session.show_page(Page::Filament, "toolchanger-filament")?;
    session.show_page(Page::Temperature, "toolchanger-temperature")?;

    // Opens like a tap on the spool strip: the spool in use belongs to the tool macro of the mounted head.
    session.window.global::<Navigation>().set_page(Page::Filament);
    if session.window.global::<Printer>().get_spool_tool() < 0 {
        return Err("the spool in use should belong to the tool macro of the mounted head".into());
    }
    session.window.global::<SpoolPicker>().invoke_show_in_use();
    session.settle_replies(Duration::from_secs(2));
    session.capture("toolchanger-spool-picker")?;
    session.window.global::<SpoolPicker>().set_open(false);

    // A tap on T1 in the prepared file's rows: its own tool macro, with the spool it has now.
    session.window.global::<SpoolPicker>().invoke_show_tool(1, 3, "T1".into());
    session.settle_replies(Duration::from_secs(2));
    session.capture("toolchanger-tool-picker")?;
    session.window.global::<SpoolPicker>().set_open(false);

    prepared_job(session)?;
    for heater in ["extruder", "extruder1"] {
        session.service.send(Command::SetHeaterTarget { heater: heater.into(), target: 0.0 });
    }
    session.pump(Duration::from_millis(500));
    head_level(session)
}

/// The plugin is `uninitialized` after the connect and after any rejected command (viesturz `_handle_connect`,
/// `_handle_command_error`). The page names it only where a tool change would
/// fail (`initialize_on` other than `first-use`); then the scene initializes the changer, which the following scenes
/// assume (a head selected, the nozzle of the status bar named).
fn start_state(session: &mut Session) -> Result<(), String> {
    let first_use = session
        .presenter
        .snapshot()
        .status
        .toolchanger
        .as_ref()
        .is_some_and(|changer| changer.initializes_on_first_use());
    // With `initialize_on: home` the homing of the session has initialized it already; the hook sets the state back.
    session.service.send(Command::RunMacro { name: "FAKE_TOOLCHANGER_STATE STATE=uninitialized".into() });
    session.wait_until("uninitialized", |snapshot| {
        snapshot.status.toolchanger.as_ref().is_some_and(|changer| changer.status == "uninitialized")
    })?;
    session.show_page(Page::Filament, "toolchanger-start")?;
    let noted = !session.window.global::<Printer>().get_changer_note().is_empty();
    if noted == first_use {
        return Err(format!(
            "the note 'not initialized' should show exactly where a change would fail (first use: {first_use})"
        ));
    }
    session.service.send(Command::InitializeToolchanger { recover: false });
    session.wait_until("initialized", |snapshot| {
        snapshot.status.toolchanger.as_ref().is_some_and(|changer| changer.is_ready())
    })
}

/// The Filament page with a tab per head: the mounted head, a head that is not mounted with its "Select" button, the
/// deviations of the changer (error with its confirmation, not initialized) and back to ready.
fn head_level(session: &mut Session) -> Result<(), String> {
    // A changer that requires a mounted tool has no `UNSELECT_TOOL`: the mounted head without "Drop".
    let has_drop = session.presenter.snapshot().sources.klipper.has_unselect_tool;
    session.show_page(Page::Filament, if has_drop { "toolchanger-filament-heads" } else { "toolchanger-no-drop" })?;
    if session.window.global::<Printer>().get_heads().row_count() < 2 {
        return Err("a tool changer should give the Filament page a tab per head".into());
    }
    if !has_drop {
        return Ok(());
    }
    let reference = session.has_reference_layout();
    if session.presenter.snapshot().status.mmu.is_some() {
        // Heads with a unit of their own: Gates and Extruder below the heads, Tools and Service beside them.
        session.window.global::<Navigation>().set_filament_head(1);
        session.capture("toolchanger-mmu-second-head")?;
        error_with_unit(session)?;
        session.window.global::<Navigation>().set_filament_tab(FilamentTab::Extruder);
        session.capture("toolchanger-mmu-extruder")?;
        session.window.global::<Navigation>().set_filament_tab(FilamentTab::Tools);
        session.capture("toolchanger-mmu-tools")?;
        session.window.global::<Navigation>().set_filament_tab(FilamentTab::Service);
        session.capture("toolchanger-mmu-service")?;
        session.window.global::<Navigation>().set_filament_tab(FilamentTab::Gates);
        session.window.global::<Navigation>().set_filament_head(-1);
        return Ok(());
    }

    // T2 is not mounted: its own nozzle and spool, nothing to extrude, the button picks it up.
    if reference {
        session.tap(HEAD_TAB_T2.0, HEAD_TAB_T2.1);
    } else {
        session.window.global::<Navigation>().set_filament_head(2);
    }
    session.capture("toolchanger-filament-parked")?;
    if reference {
        session.tap(HEAD_ACTION.0, HEAD_ACTION.1);
    } else {
        session.service.send(Command::RunMacro { name: "T2".into() });
    }
    session.wait_until("T2 selected", |snapshot| {
        snapshot.status.toolchanger.as_ref().is_some_and(|changer| changer.selected == Some(2))
    })?;
    session.capture("toolchanger-filament-selected")?;

    // The deviations, set through the fake.
    for (state, name) in [("error", "toolchanger-error"), ("uninitialized", "toolchanger-uninitialized")] {
        session.service.send(Command::RunMacro { name: format!("FAKE_TOOLCHANGER_STATE STATE={state}") });
        session.wait_until(state, |snapshot| {
            snapshot.status.toolchanger.as_ref().is_some_and(|changer| changer.status == state)
        })?;
        session.capture(name)?;
        if state == "uninitialized" && session.window.global::<Printer>().get_changer_note().is_empty() {
            // `initialize_on: first-use`: nothing to tell, the next selection initializes by itself.
            break;
        }
        if state == "error" && reference {
            session.tap(NOTE_BUTTON.0, NOTE_BUTTON.1);
            session.capture("toolchanger-recover-confirm")?;
            session.window.global::<Confirm>().set_open(false);
        }
    }
    if reference && !session.window.global::<Printer>().get_changer_note().is_empty() {
        session.tap(NOTE_BUTTON.0, NOTE_BUTTON.1);
    } else {
        session.service.send(Command::InitializeToolchanger { recover: false });
    }
    session.wait_until("ready again", |snapshot| {
        snapshot.status.toolchanger.as_ref().is_some_and(|changer| changer.is_ready())
    })?;
    session.capture("toolchanger-ready-again")?;
    session.window.global::<Navigation>().set_filament_head(-1);
    no_head_mounted(session)
}

/// `UNSELECT_TOOL` drops the head: no head on the carriage, the Filament page with its head tabs and the status bar
/// without a nozzle of a mounted head (`toolchanger-no-head`); then T2 is picked up again for the following scenes.
fn no_head_mounted(session: &mut Session) -> Result<(), String> {
    session.service.send(Command::RunGcode { script: "UNSELECT_TOOL".into() });
    session.wait_until("no head mounted", |snapshot| {
        snapshot.status.toolchanger.as_ref().is_some_and(|changer| changer.selected.is_none())
    })?;
    session.show_page(Page::Filament, "toolchanger-no-head")?;
    session.service.send(Command::RunMacro { name: "T2".into() });
    session.wait_until("T2 selected again", |snapshot| {
        snapshot.status.toolchanger.as_ref().is_some_and(|changer| changer.selected == Some(2))
    })
}

/// The line of the changer's error above the tabs of a head with a unit of its own, the fullest page of the head level
/// (`toolchanger-error-unit`); back to ready afterwards.
fn error_with_unit(session: &mut Session) -> Result<(), String> {
    session.service.send(Command::RunMacro { name: "FAKE_TOOLCHANGER_STATE STATE=error".into() });
    session.wait_until("error", |snapshot| {
        snapshot.status.toolchanger.as_ref().is_some_and(|changer| changer.status == "error")
    })?;
    session.capture("toolchanger-error-unit")?;
    session.service.send(Command::InitializeToolchanger { recover: false });
    session.wait_until("ready again", |snapshot| {
        snapshot.status.toolchanger.as_ref().is_some_and(|changer| changer.is_ready())
    })
}
