//! Lanes and the load path (package D2): the lanes of AFC as tiles with their dialog, the load path card for the lane
//! loaded and the one tapped last, the head's part of the path on the Extruder view of a tool changer, and the sensors
//! of the machine on the Sensors tab where there are several heads. Runs on the profiles `afc`, `afc-toolchanger`,
//! `toolchanger-mmu` and `toolchanger`; what a profile does not have is skipped.

use crate::session::Session;
use freakymate::{Actions, ChangerTab, FilamentTab, MmuDialogs, Navigation, Page, Printer};
use freakymate_core::domain::changer;
use freakymate_core::domain::machine_tree::LaneId;
use freakymate_core::sources::happy_hare::LoadedFilament;
use freakymate_core::Command;
use slint::{ComponentHandle, Model};
use std::time::Duration;

/// Where the tiles of the Gates tab sit in the 800x480 layout (one page, five tiles): the first and the third.
const TILE_FIRST: (f32, f32) = (182.0, 190.0);
const TILE_THIRD: (f32, f32) = (450.0, 190.0);

pub(super) fn lanes(session: &mut Session) -> Result<(), String> {
    session.settle_replies(Duration::from_secs(1));
    let snapshot = session.presenter.snapshot();
    session.window.global::<Navigation>().set_filament_tab(FilamentTab::Gates);
    session.window.global::<Navigation>().set_filament_head(-1);
    if snapshot.status.afc.is_some() && snapshot.tree.has_swappable_heads() {
        afc_heads(session)?;
    } else if snapshot.status.afc.is_some() {
        afc_lanes(session)?;
    }
    if snapshot.status.afc.is_some() {
        afc_changer(session)?;
    }
    if snapshot.status.mmu.is_some() && snapshot.tree.has_swappable_heads() {
        units_on_heads(session)?;
    }
    if snapshot.status.mmu.is_none() && snapshot.status.toolchanger.is_some() {
        head_without_lanes(session)?;
    }
    Ok(())
}

/// A tap on a tile opens its dialog and makes its lane the one of the path card. Without the reference layout the
/// taps are made through the app's callbacks.
fn tap_tile(session: &mut Session, row: i32, at: (f32, f32)) {
    if session.has_reference_layout() {
        session.tap(at.0, at.1);
    } else {
        session.window.global::<Actions>().invoke_lane_tapped(row);
        session.window.global::<MmuDialogs>().set_gate_row(row);
    }
    session.pump(Duration::from_millis(500));
}

fn close_dialog(session: &Session) {
    session.window.global::<MmuDialogs>().set_gate_row(-1);
}

/// Loads `lane` through `CHANGE_TOOL` and waits until AFC names it as the lane in use.
fn change_tool(session: &mut Session, lane: &str) -> Result<(), String> {
    session.service.send(Command::RunGcode { script: format!("CHANGE_TOOL LANE={lane}") });
    session.wait_until(&format!("{lane} loaded"), |snapshot| {
        snapshot.status.afc.as_ref().is_some_and(|afc| afc.current_load.as_deref() == Some(lane))
    })
}

/// A Box Turtle: the tiles, the card of the lane in use (lane2), then lane3 loaded through `CHANGE_TOOL`, the card
/// after a tap on a lane that is not loaded, and the dialog of a lane that is.
fn afc_lanes(session: &mut Session) -> Result<(), String> {
    if !session.window.global::<Printer>().get_has_lanes() || session.window.global::<Printer>().get_has_mmu() {
        return Err("AFC should give the Gates tab lane tiles and not Happy Hare's controls".into());
    }
    session.show_page(Page::Filament, "afc-lanes")?;
    change_tool(session, "lane3")?;
    session.show_page(Page::Filament, "afc-path-loaded")?;

    tap_tile(session, 0, TILE_FIRST);
    if session.window.global::<MmuDialogs>().get_gate_row() != 0 {
        return Err("a tap on the first lane tile should open its dialog".into());
    }
    session.capture("afc-lane-dialog")?;
    close_dialog(session);
    session.show_page(Page::Filament, "afc-path-unloaded")?;

    tap_tile(session, 2, TILE_THIRD);
    session.capture("afc-lane-dialog-loaded")?;
    close_dialog(session);
    Ok(())
}

/// AFC with its own tool changer has one lane per head and no Gates tab: the heads' Extruder view with the lane of the
/// mounted head, then `CHANGE_TOOL` to the lane of the third head (the head moves to the carriage, its lane loads), and
/// the view of a head that is not mounted.
fn afc_heads(session: &mut Session) -> Result<(), String> {
    session.window.global::<Navigation>().set_filament_tab(FilamentTab::Extruder);
    session.show_page(Page::Filament, "afc-toolchanger-heads")?;
    change_tool(session, "e2")?;
    session.show_page(Page::Filament, "afc-toolchanger-path-loaded")?;
    session.window.global::<Navigation>().set_filament_head(0);
    session.show_page(Page::Filament, "afc-toolchanger-head-parked")?;
    session.window.global::<Navigation>().set_filament_head(-1);
    // The mounted head can be dropped when AFC lists `AFC_UNSELECT_TOOL`: "Drop" in the picker.
    session.window.global::<Navigation>().set_head_picker_open(true);
    session.capture("afc-toolchanger-head-picker")?;
    session.window.global::<Navigation>().set_head_picker_open(false);
    if !session.window.global::<Printer>().get_has_drop() {
        return Err("AFC's tool changer lists AFC_UNSELECT_TOOL, so the picker should offer Drop".into());
    }
    Ok(())
}

/// The tools of AFC (a tap on a row chooses the lane; AFC swaps the lanes of two tools) and its service on the Changer
/// page, then the error state: the hint on the Filament page, "Clear error" and "Back to hub" on Service.
fn afc_changer(session: &mut Session) -> Result<(), String> {
    session.window.global::<Navigation>().set_filament_tab(FilamentTab::Gates);
    session.window.global::<Navigation>().set_changer_tab(ChangerTab::Tools);
    if session.window.global::<Printer>().get_afc_tools().row_count() == 0 {
        return Err("AFC should give the Changer page a Tools tab with its tool map".into());
    }
    session.show_page(Page::Changer, "afc-changer-tools")?;
    session.window.global::<MmuDialogs>().set_afc_tool(1);
    session.capture("afc-changer-tool-lane")?;
    session.window.global::<MmuDialogs>().set_afc_tool(-1);

    let mapped = |snapshot: &freakymate_core::Snapshot, tool: usize, lane: &str| {
        changer::afc_tool_map(&snapshot.status).iter().any(|entry| entry.tool == tool && entry.lane == lane)
    };
    // Without tiles (AFC's own changer has none) there is no lane to choose from by tile.
    if let Some(row) = lane_row(session, "lane2") {
        // AFC gives lane1 the tool lane2 had.
        session.window.global::<Actions>().invoke_map_tool_to_lane(0, row);
        session.wait_until("T0 on lane2", |snapshot| mapped(snapshot, 0, "lane2"))?;
        session.pump(Duration::from_millis(300));
        session.show_page(Page::Changer, "afc-changer-tools-swapped")?;
        session.window.global::<Actions>().invoke_mmu_reset_tool_map();
        session.wait_until("T0 on lane1", |snapshot| mapped(snapshot, 0, "lane1"))?;
    }

    session.window.global::<Navigation>().set_changer_tab(ChangerTab::Service);
    session.show_page(Page::Changer, "afc-changer-service")?;
    afc_error(session)
}

/// The tile row of a lane on the Gates tab.
fn lane_row(session: &Session, lane: &str) -> Option<i32> {
    let row = session.presenter.lane_row(&LaneId::Afc(lane.to_owned()));
    (row >= 0).then_some(row)
}

/// `FAKE_AFC_ERROR LANE=lane3`: AFC in its error state with lane3's filament behind the hub. "Clear error" is on the
/// Filament page and on Service, and every lane with filament can go back to the hub; `RESET_FAILURE` ends the state.
fn afc_error(session: &mut Session) -> Result<(), String> {
    // With lane3 of the Box Turtle (behind its hub); AFC's own changer has no hub, there the state alone.
    let lanes = session.presenter.snapshot().status.afc.map(|afc| afc.lanes).unwrap_or_default();
    let script =
        if lanes.iter().any(|lane| lane.name == "lane3") { "FAKE_AFC_ERROR LANE=lane3" } else { "FAKE_AFC_ERROR" };
    session.service.send(Command::RunGcode { script: script.into() });
    session.wait_until("AFC error", |snapshot| snapshot.status.afc.as_ref().is_some_and(|afc| afc.error))?;
    session.pump(Duration::from_millis(300));
    if !session.window.global::<Printer>().get_afc_error() {
        return Err("AFC's error state should reach the page".into());
    }
    session.window.global::<Navigation>().set_filament_tab(FilamentTab::Gates);
    session.show_page(Page::Filament, "afc-error-filament")?;
    session.window.global::<Navigation>().set_changer_tab(ChangerTab::Service);
    session.show_page(Page::Changer, "afc-error-service")?;
    session.window.global::<Actions>().invoke_afc_reset_failure();
    session.wait_until("AFC idle", |snapshot| snapshot.status.afc.as_ref().is_some_and(|afc| !afc.error))?;
    session.pump(Duration::from_millis(300));
    session.show_page(Page::Changer, "afc-error-cleared")
}

/// The second unit loaded on the second head: the path card of a gate of that unit, and the Extruder view of the head.
fn units_on_heads(session: &mut Session) -> Result<(), String> {
    let snapshot = session.presenter.snapshot();
    let Some(gate) = snapshot.status.mmu.as_ref().and_then(|mmu| mmu.units.get(1)).map(|unit| unit.first_gate) else {
        return Ok(());
    };
    if snapshot.status.mmu.as_ref().is_some_and(|mmu| !mmu.homed) {
        session.service.send(Command::MmuHome);
        session.wait_until("MMU homed", |snapshot| snapshot.status.mmu.as_ref().is_some_and(|mmu| mmu.homed))?;
    }
    // Happy Hare selects another gate only with nothing loaded.
    if snapshot.status.mmu.as_ref().is_some_and(|mmu| mmu.filament != LoadedFilament::Unloaded) {
        session.service.send(Command::MmuUnload);
        session.wait_until("nothing loaded", |snapshot| {
            snapshot.status.mmu.as_ref().is_some_and(|mmu| mmu.filament == LoadedFilament::Unloaded)
        })?;
    }
    session.service.send(Command::LoadLane { lane: LaneId::Gate(gate) });
    session.wait_until("a gate of the second unit loaded", |snapshot| {
        snapshot
            .status
            .mmu
            .as_ref()
            .is_some_and(|mmu| mmu.gate == gate as i32 && mmu.filament == LoadedFilament::Loaded)
    })?;
    session.window.global::<Navigation>().set_filament_head(1);
    session.show_page(Page::Filament, "lanes-unit-second-head")?;
    session.window.global::<Navigation>().set_filament_tab(FilamentTab::Extruder);
    session.show_page(Page::Filament, "lanes-head-extruder-mmu")?;
    session.window.global::<Navigation>().set_filament_tab(FilamentTab::Gates);
    session.service.send(Command::UnloadLane { lane: LaneId::Gate(gate) });
    session.wait_until("unloaded again", |snapshot| {
        snapshot.status.mmu.as_ref().is_some_and(|mmu| mmu.filament == LoadedFilament::Unloaded)
    })?;
    session.window.global::<Navigation>().set_filament_head(-1);
    Ok(())
}

/// A tool changer without lanes: the head's part of the path on its Extruder view, the machine's sensors on Changer > Sensors.
fn head_without_lanes(session: &mut Session) -> Result<(), String> {
    session.window.global::<Navigation>().set_filament_head(1);
    session.window.global::<Navigation>().set_filament_tab(FilamentTab::Extruder);
    session.show_page(Page::Filament, "lanes-head-extruder")?;
    if session.window.global::<Printer>().get_filament_sensors().row_count() == 0 {
        return Err("the machine should have filament sensors for the Sensors tab".into());
    }
    session.window.global::<Navigation>().set_changer_tab(ChangerTab::Sensors);
    session.show_page(Page::Changer, "lanes-sensors")?;
    session.window.global::<Navigation>().set_changer_tab(ChangerTab::Tools);
    session.window.global::<Navigation>().set_filament_head(-1);
    Ok(())
}
