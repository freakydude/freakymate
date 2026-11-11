//! A running print: layer pause, objects, errors and the status bar.

use super::ask;
use super::temperature::{nozzle_history, temperature_leads_to_heaters};
use super::STATUS_BAR_LIGHT;
use crate::session::Session;
use freakymate::{Actions, Confirm, FilamentTab, MmuDialogs, Navigation, Page, Printer};
use freakymate_core::sources::klipper::PrintState;
use freakymate_core::Command;
use slint::{ComponentHandle, Model};
use std::time::Duration;

/// The Layer tile of the print page and the main button of the layer pause dialog (800x480, no fitting).
const LAYER_TILE: (f32, f32) = (580.0, 133.0);

const LAYER_PAUSE_BUTTON: (f32, f32) = (440.0, 320.0);

/// How far ahead the scene plans a pause first, so that it shows without pausing.
const PLANNED_LAYERS_AHEAD: i32 = 40;

/// The Objects button of the paused print page, then in its dialog: the fake's BENCHY_1 on the bed map (centre
/// 200/160 mm on a 300 mm bed, map at 96/128 with 268 px), the Exclude button and the close cross (800x480).
const OBJECTS_BUTTON: (f32, f32) = (740.0, 204.0);

const MAP_BENCHY_1: (f32, f32) = (275.0, 253.0);

const EXCLUDE_BUTTON: (f32, f32) = (560.0, 432.0);

const CLOSE_OBJECTS_BUTTON: (f32, f32) = (752.0, 92.0);

/// With Happy Hare the Tools tab compares the tools with the file being printed.
pub(super) fn tools_match_the_job(session: &mut Session) -> Result<(), String> {
    if session.presenter.snapshot().status.happy_hare().is_none() {
        return Ok(());
    }
    session.window.global::<Navigation>().set_filament_tab(FilamentTab::Tools);
    session.show_page(Page::Filament, "filament-mmu-tools-match")?;
    if session.window.global::<Printer>().get_mmu_match_file().is_empty() {
        session.window.global::<Navigation>().set_filament_tab(FilamentTab::Gates);
        return Err("the Tools tab does not compare with the file being printed".into());
    }
    tool_compare(session)?;
    session.window.global::<Navigation>().set_filament_tab(FilamentTab::Gates);
    Ok(())
}

/// T0 of the file mapped to the ABS of gate 3 for a moment: its dialog compares file and gate, then the map is reset.
fn tool_compare(session: &mut Session) -> Result<(), String> {
    let tool_0_gate = |snapshot: &freakymate_core::Snapshot| {
        snapshot.status.mmu.as_ref().and_then(|mmu| mmu.tool_to_gate.first().copied())
    };
    session.window.global::<Actions>().invoke_mmu_map_tool(0, 3);
    session.wait_until("T0 on gate 3", |snapshot| tool_0_gate(snapshot) == Some(3))?;
    session.window.global::<MmuDialogs>().set_tool(0);
    session.show_page(Page::Filament, "filament-mmu-tool-compare")?;
    session.window.global::<MmuDialogs>().set_tool(-1);
    session.window.global::<MmuDialogs>().set_tool_gate(0);
    session.show_page(Page::Filament, "filament-mmu-tool-gate-compare")?;
    session.window.global::<MmuDialogs>().set_tool_gate(-1);
    let tool = session.window.global::<Printer>().get_mmu_tools().row_data(0).unwrap_or_default();
    session.window.global::<Actions>().invoke_mmu_map_tool(0, 0);
    session.wait_until("T0 back on gate 0", |snapshot| tool_0_gate(snapshot) == Some(0))?;
    if !tool.material_differs {
        return Err("T0 on the ABS of gate 3 does not fit, but its dialog marks no material".into());
    }
    Ok(())
}

pub(super) fn printing(session: &mut Session) -> Result<(), String> {
    session.service.send(Command::StartPrint { filename: "benchy.gcode".into() });
    session.wait_until("print running", |snapshot| {
        snapshot.status.job.state == PrintState::Printing && snapshot.status.job.progress > 0.005
    })?;
    session.pump(Duration::from_secs(1));
    session.show_page(Page::Print, "print-printing")?;
    tools_match_the_job(session)?;
    temperature_leads_to_heaters(session)?;
    nozzle_history(session)?;
    tangle_boost(session)?;
    session.show_page(Page::Temperature, "temperature-printing")?;
    session.show_page(Page::System, "system-printing")?;

    session.window.global::<Navigation>().set_page(Page::Print);
    pause_at_next_layer(session)?;
    session.capture("print-paused")?;
    // Jogging stays possible, homing, leveling and motors off do not (the paused print resumes from its position).
    if !session.window.global::<Printer>().get_position_locked() {
        return Err("position-locked off while the print is paused".into());
    }
    session.show_page(Page::Move, "move-paused")?;
    session.window.global::<Navigation>().set_page(Page::Print);
    exclude_object(session)?;
    session.service.send(Command::ResumePrint);
    session.wait_until("print resumed", |snapshot| snapshot.status.job.state == PrintState::Printing)?;
    ask(session, "Cancel print", "The current print will be stopped and cannot be resumed.", "cancel-print", "");
    session.capture("confirm-cancel")?;
    session.window.global::<Confirm>().invoke_execute();
    session.wait_until("print cancelled", |snapshot| snapshot.status.job.state == PrintState::Cancelled)?;
    wait_for_end_time(session)?;
    session.capture("print-cancelled")?;
    print_error(session)
}

/// A pause planned layers ahead shows at the Layer tile and in its dialog (the fake has Mainsail's macros and their
/// variables). The dialog's main button then plans a pause at the next layer, and the fake pauses there. Another
/// layout sends the command instead of tapping.
pub(super) fn pause_at_next_layer(session: &mut Session) -> Result<(), String> {
    let current = session.window.global::<Printer>().get_current_layer();
    let ahead = u32::try_from(current + PLANNED_LAYERS_AHEAD).unwrap_or_default();
    session.service.send(Command::PauseAtLayer { layer: ahead });
    session.wait_until("pause planned ahead", |snapshot| {
        snapshot.status.layer_pause_plan.is_some_and(|plan| plan.at_layer == Some(ahead))
    })?;
    session.pump(Duration::from_secs(1));
    session.capture("print-pause-planned")?;
    if session.has_reference_layout() {
        let (x, y) = LAYER_TILE;
        session.tap(x, y);
        session.capture("layer-pause")?;
        let (x, y) = LAYER_PAUSE_BUTTON;
        session.tap(x, y);
    } else {
        let next = u32::try_from(session.window.global::<Printer>().get_current_layer() + 1).unwrap_or_default();
        session.service.send(Command::PauseAtLayer { layer: next });
    }
    session.wait_until("paused at the next layer", |snapshot| snapshot.status.job.state == PrintState::Paused)?;
    session.pump(Duration::from_secs(1));
    Ok(())
}

/// The end of a job comes from the history, which the service reads once the job has ended.
pub(super) fn wait_for_end_time(session: &mut Session) -> Result<(), String> {
    for _ in 0..40 {
        if session.window.global::<Printer>().get_ended() != "-" {
            return Ok(());
        }
        session.pump(Duration::from_millis(100));
    }
    Err("the end of the job did not arrive from the history".into())
}

/// A print that Klipper stops with an error (the fake's `FAKE_PRINT_ERROR`).
pub(super) fn print_error(session: &mut Session) -> Result<(), String> {
    session.service.send(Command::StartPrint { filename: "benchy.gcode".into() });
    session.wait_until("print running again", |snapshot| snapshot.status.job.state == PrintState::Printing)?;
    session.service.send(Command::RunGcode { script: "FAKE_PRINT_ERROR".into() });
    session.wait_until("print failed", |snapshot| snapshot.status.job.state == PrintState::Error)?;
    wait_for_end_time(session)?;
    session.capture("print-error")
}

/// Raises the tension in the fake's buffer until tangle prevention boosts the gear current, and takes it back after the
/// picture. Skipped without a buffer that tangle prevention watches (Happy Hare with a proportional sensor).
pub(super) fn tangle_boost(session: &mut Session) -> Result<(), String> {
    let has_buffer = session.presenter.snapshot().status.mmu.as_ref().is_some_and(|mmu| mmu.flowguard.is_some());
    if !has_buffer {
        return Ok(());
    }
    let boosting = |snapshot: &freakymate_core::Snapshot| {
        snapshot.status.mmu.as_ref().and_then(|mmu| mmu.flowguard.as_ref()).is_some_and(|flowguard| flowguard.boosting)
    };
    session.service.send(Command::RunGcode { script: "FAKE_MMU_TANGLE BOOST=1".into() });
    session.wait_until("tangle prevention boosting", boosting)?;
    session.show_page(Page::Print, "print-tangle-boost")?;
    session.service.send(Command::RunGcode { script: "FAKE_MMU_TANGLE BOOST=0".into() });
    session.wait_until("boost released", |snapshot| !boosting(snapshot))
}

/// Excludes one object of the print and checks that the Objects dialog data follows. In the reference layout it opens
/// the dialog, selects the object on the bed map and taps Exclude; another layout sends the command. Skipped without
/// the module.
pub(super) fn exclude_object(session: &mut Session) -> Result<(), String> {
    if session.presenter.snapshot().status.print_objects.is_empty() {
        return Ok(());
    }
    if session.has_reference_layout() {
        for (x, y) in [OBJECTS_BUTTON, MAP_BENCHY_1] {
            session.tap(x, y);
        }
        session.capture("print-objects-map")?;
        status_bar_cancels_the_dialog(session)?;
        for (x, y) in [OBJECTS_BUTTON, MAP_BENCHY_1, EXCLUDE_BUTTON] {
            session.tap(x, y);
        }
    } else {
        session.service.send(Command::ExcludeObject { name: "BENCHY_1".into() });
    }
    session.wait_until("object excluded", |snapshot| {
        snapshot.status.print_objects.iter().any(|object| object.name == "BENCHY_1" && object.excluded)
    })?;
    session.pump(Duration::from_secs(1));
    let items = session.window.global::<Printer>().get_print_objects();
    let excluded: Vec<String> = (0..items.row_count())
        .filter_map(|row| items.row_data(row))
        .filter(|item| item.excluded)
        .map(|item| item.name.to_string())
        .collect();
    if excluded != ["BENCHY_1"] {
        return Err(format!("Printer.print-objects shows excluded {excluded:?}, expected [\"BENCHY_1\"]"));
    }
    session.capture("print-objects-excluded")?;
    if session.has_reference_layout() {
        let (x, y) = CLOSE_OBJECTS_BUTTON;
        session.tap(x, y);
    }
    Ok(())
}

/// A tap on the status bar cancels the open Objects dialog with BENCHY_1 selected: the light panel opens, and a tap
/// where Exclude was reaches the print page instead.
pub(super) fn status_bar_cancels_the_dialog(session: &mut Session) -> Result<(), String> {
    let (x, y) = STATUS_BAR_LIGHT;
    session.tap(x, y);
    let navigation = session.window.global::<Navigation>();
    if !navigation.get_light_open() {
        return Err("a tap on the light symbol did not open the light panel".into());
    }
    navigation.set_light_open(false);
    let (x, y) = EXCLUDE_BUTTON;
    session.tap(x, y);
    session.pump(Duration::from_secs(1));
    if session.presenter.snapshot().status.print_objects.iter().any(|object| object.excluded) {
        return Err("the Objects dialog stayed open after a tap on the status bar".into());
    }
    Ok(())
}
