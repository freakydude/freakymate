//! A tool changer with three heads (fake profile `toolchanger`): the spool in use, the temperature hints, the picker for
//! a tool macro and the prepared file with its further tools; on every profile with a tool changer the head level of the
//! Filament page (`toolchanger-mmu`: heads with a unit of their own).

use super::pages::prepared_job;
use crate::session::Session;
use freakymate::{ChangerTab, Confirm, FilamentTab, Navigation, Page, Printer, SpoolPicker};
use freakymate_core::Command;
use slint::ComponentHandle;
use slint::Model;
use std::time::Duration;

/// The chip of the head at the right end of the tab row, in the 800x480 layout; in the picker the row of T2 (a tap on it
/// chooses the head) and the button of that row.
const HEAD_CHIP: (f32, f32) = (712.0, 90.0);
const PICKER_ROW_T2: (f32, f32) = (300.0, 280.0);
const PICKER_BUTTON_T2: (f32, f32) = (690.0, 280.0);
/// The button of the hint in the tab row (Initialize, Recover).
const NOTE_BUTTON: (f32, f32) = (550.0, 80.0);

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

    tune_head_fan(session)?;
    prepared_job(session)?;
    for heater in ["extruder", "extruder1"] {
        session.service.send(Command::SetHeaterTarget { heater: heater.into(), target: 0.0 });
    }
    session.pump(Duration::from_millis(500));
    head_level(session)
}

/// The part fan of the mounted head on Tune › Basics: the fake gives each head a `fan_generic T<n>_partfan` and has
/// `M106` drive the fan of the active head (viesturz `tool.py` Z.126, `toolchanger.py` Z.837-838). T1 runs at 50 %
/// (`toolchanger-tune-fan`); then the head and the fan go back as they were.
fn tune_head_fan(session: &mut Session) -> Result<(), String> {
    session.service.send(Command::RunMacro { name: "T1".into() });
    session.wait_until("T1 selected", |snapshot| {
        snapshot.status.toolchanger.as_ref().is_some_and(|changer| changer.selected == Some(1))
    })?;
    session.service.send(Command::RunGcode { script: "M106 S127.5".into() });
    session.pump(Duration::from_secs(1));
    let printer = session.window.global::<Printer>();
    if !printer.get_has_part_fan() || (printer.get_part_fan().speed - 50.0).abs() > 0.5 {
        return Err("Tune should show the fan of the mounted head at 50 %".into());
    }
    session.show_page(Page::Tune, "toolchanger-tune-fan")?;
    session.service.send(Command::RunGcode { script: "M107".into() });
    session.service.send(Command::RunMacro { name: "T0".into() });
    session.wait_until("T0 selected again", |snapshot| {
        snapshot.status.toolchanger.as_ref().is_some_and(|changer| changer.selected == Some(0))
    })
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

/// The Filament page with the chip of the head and its picker: the mounted head, a head that is not mounted with its "Select" button, the
/// deviations of the changer (error with its confirmation, not initialized) and back to ready.
fn head_level(session: &mut Session) -> Result<(), String> {
    // A changer that requires a mounted tool has no `UNSELECT_TOOL`: the mounted head without "Drop".
    let has_drop = session.presenter.snapshot().sources.klipper.has_unselect_tool;
    session.show_page(Page::Filament, if has_drop { "toolchanger-filament-heads" } else { "toolchanger-no-drop" })?;
    if session.window.global::<Printer>().get_heads().row_count() < 2 {
        return Err("a tool changer should give the Filament page a row per head".into());
    }
    if !has_drop {
        return Ok(());
    }
    let reference = session.has_reference_layout();
    if session.presenter.snapshot().status.mmu.is_some() {
        // Heads with a unit of their own: Gates and Extruder beside the chip; Tools and Service are on Changer.
        session.window.global::<Navigation>().set_filament_head(1);
        session.capture("toolchanger-mmu-second-head")?;
        error_with_unit(session)?;
        session.window.global::<Navigation>().set_filament_tab(FilamentTab::Extruder);
        session.capture("toolchanger-mmu-extruder")?;
        session.window.global::<Navigation>().set_filament_tab(FilamentTab::Gates);
        session.window.global::<Navigation>().set_filament_head(-1);
        for (tab, name) in
            [(ChangerTab::Tools, "toolchanger-mmu-tools"), (ChangerTab::Service, "toolchanger-mmu-service")]
        {
            session.window.global::<Navigation>().set_changer_tab(tab);
            session.show_page(Page::Changer, name)?;
        }
        session.window.global::<Navigation>().set_changer_tab(ChangerTab::Tools);
        return Ok(());
    }

    // The chip opens the picker; a tap on the row of T2 shows that head, which is not mounted: its own nozzle and
    // spool, nothing to extrude. The button of its row in the picker selects it.
    if reference {
        session.tap(HEAD_CHIP.0, HEAD_CHIP.1);
    } else {
        session.window.global::<Navigation>().set_head_picker_open(true);
    }
    session.capture("toolchanger-head-picker")?;
    if reference {
        session.tap(PICKER_ROW_T2.0, PICKER_ROW_T2.1);
    } else {
        session.window.global::<Navigation>().set_filament_head(2);
        session.window.global::<Navigation>().set_head_picker_open(false);
    }
    if session.window.global::<Navigation>().get_head_picker_open()
        || session.window.global::<Navigation>().get_filament_head() != 2
    {
        return Err("a tap on the row of T2 should show that head and close the picker".into());
    }
    session.capture("toolchanger-filament-parked")?;
    if reference {
        session.tap(HEAD_CHIP.0, HEAD_CHIP.1);
        session.tap(PICKER_BUTTON_T2.0, PICKER_BUTTON_T2.1);
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

/// `UNSELECT_TOOL` drops the head: no head on the carriage, the Filament page with its chip and the status bar
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
