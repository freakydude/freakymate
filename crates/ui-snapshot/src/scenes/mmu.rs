//! Gates, tools, groups, bypass and service tab of the MMU, and the filament path.

use crate::session::Session;
use freakymate::{Actions, FilamentTab, MmuDialogs, Navigation, Page, Printer};
use freakymate_core::sources::happy_hare::ServoPosition;
use freakymate_core::Command;
use slint::ComponentHandle;
use std::time::Duration;

/// Where taps land in the 800x480 layout: the tile of gate 2 on the Gates tab, and the button "Select" of the gate
/// dialog.
const GATE_2_TILE: (f32, f32) = (450.0, 190.0);

const GATE_DIALOG_SELECT: (f32, f32) = (172.0, 230.0);

/// Service tab of the MMU, first as it opens, then after the buttons for the servo and the motors went through the app
/// to the fake: the servo is down and Home stands out again. Homes the MMU, lifts the servo and returns to the Gates tab
/// afterwards.
pub(super) fn mmu_service(session: &mut Session) -> Result<(), String> {
    session.window.global::<Navigation>().set_filament_tab(FilamentTab::Service);
    session.show_page(Page::Filament, "filament-mmu-service")?;
    {
        let actions = session.window.global::<Actions>();
        actions.invoke_mmu_servo_down();
        actions.invoke_mmu_motors_off();
    }
    session.wait_until("servo down and MMU not homed", |snapshot| {
        snapshot.status.mmu.as_ref().is_some_and(|mmu| !mmu.homed && mmu.servo == Some(ServoPosition::Down))
    })?;
    session.show_page(Page::Filament, "filament-mmu-service-motors-off")?;
    if session.window.global::<Printer>().get_mmu_servo() != "down" {
        return Err("the service tab does not show the servo down".into());
    }
    {
        let actions = session.window.global::<Actions>();
        actions.invoke_mmu_home();
        actions.invoke_mmu_servo_up();
    }
    session.wait_until("MMU homed with the servo up", |snapshot| {
        snapshot.status.mmu.as_ref().is_some_and(|mmu| mmu.homed && mmu.servo == Some(ServoPosition::Up))
    })?;
    session.window.global::<Navigation>().set_filament_tab(FilamentTab::Gates);
    Ok(())
}

/// Where taps land in the 800x480 layout: the row of T3 on the Tools tab (without a file only its gate side), and gate 1
/// in the choice of gate.
const TOOL_3_GATE_SIDE: (f32, f32) = (600.0, 406.0);
const GATE_DIALOG_GATE_1: (f32, f32) = (600.0, 206.0);

/// Tools and EndlessSpool groups through the app to the fake, both set on the Tools tab only: T3 moves to gate 1 and
/// gate 1 into the group of gate 0, with EndlessSpool on. In the 800x480 layout the move of T3 goes through taps:
/// T3 opens the choice of gate, gate 1 maps it and closes the dialog. Everything goes back afterwards.
pub(super) fn mmu_tools_and_groups(session: &mut Session) -> Result<(), String> {
    let tool_3_gate =
        |session: &Session| session.presenter.snapshot().status.mmu.and_then(|mmu| mmu.tool_to_gate.get(3).copied());
    session.window.global::<Actions>().invoke_mmu_set_endless_spool(true);
    session
        .wait_until("EndlessSpool on", |snapshot| snapshot.status.mmu.as_ref().is_some_and(|mmu| mmu.endless_spool))?;
    session.window.global::<Navigation>().set_filament_tab(FilamentTab::Tools);
    if session.has_reference_layout() {
        session.window.global::<Navigation>().set_page(Page::Filament);
        session.pump(Duration::from_millis(300));
        let before = tool_3_gate(session);
        let (x, y) = TOOL_3_GATE_SIDE;
        session.tap(x, y);
        session.pump(Duration::from_millis(500));
        if session.window.global::<MmuDialogs>().get_tool_gate() != 3 || tool_3_gate(session) != before {
            return Err("a tap on the gate side of the row of T3 does not open the choice of gate, or it maps".into());
        }
        session.show_page(Page::Filament, "filament-mmu-tool-gate-dialog")?;
        let (x, y) = GATE_DIALOG_GATE_1;
        session.tap(x, y);
    } else {
        session.window.global::<Actions>().invoke_mmu_map_tool(3, 1);
    }
    session.wait_until("T3 on gate 1", |snapshot| {
        snapshot.status.mmu.as_ref().is_some_and(|mmu| mmu.tool_to_gate.get(3) == Some(&1))
    })?;
    if session.window.global::<MmuDialogs>().get_tool_gate() != -1 {
        return Err("the choice of gate stays open after a gate was tapped".into());
    }
    session.window.global::<Actions>().invoke_mmu_set_endless_spool_group(1, 0);
    session.wait_until("gate 1 in group 0", |snapshot| {
        snapshot
            .status
            .mmu
            .as_ref()
            .is_some_and(|mmu| mmu.gates.get(1).and_then(|gate| gate.endless_spool_group) == Some(0))
    })?;
    session.show_page(Page::Filament, "filament-mmu-tools")?;
    session.window.global::<MmuDialogs>().set_endless_spool_open(true);
    session.show_page(Page::Filament, "filament-mmu-endless-spool")?;
    session.window.global::<MmuDialogs>().set_endless_spool_open(false);
    session.window.global::<Navigation>().set_filament_tab(FilamentTab::Gates);
    {
        let actions = session.window.global::<Actions>();
        actions.invoke_mmu_reset_tool_map();
        actions.invoke_mmu_set_endless_spool(false);
    }
    session.wait_until("tools back on their gates, EndlessSpool off", |snapshot| {
        snapshot.status.mmu.as_ref().is_some_and(|mmu| {
            !mmu.endless_spool && mmu.tool_to_gate.iter().enumerate().all(|(tool, gate)| tool == *gate)
        })
    })
}

/// The bypass: its dialog, then selected through the app; the fake refuses it while filament is loaded, so the scene
/// runs with the filament unloaded. Gate 2 is selected again afterwards.
pub(super) fn mmu_bypass(session: &mut Session) -> Result<(), String> {
    session.window.global::<MmuDialogs>().set_bypass_open(true);
    session.show_page(Page::Filament, "filament-mmu-bypass")?;
    session.window.global::<MmuDialogs>().set_bypass_open(false);
    session.window.global::<Actions>().invoke_mmu_select_bypass();
    session.wait_until("bypass selected", |snapshot| {
        snapshot.status.mmu.as_ref().is_some_and(|mmu| mmu.bypass_selected())
    })?;
    if !session.window.global::<Printer>().get_mmu_bypass() {
        return Err("the Gates tab does not show the bypass as selected".into());
    }
    session.window.global::<Navigation>().set_mmu_page(1);
    session.show_page(Page::Filament, "filament-mmu-bypass-selected")?;
    session.window.global::<Navigation>().set_mmu_page(0);
    session.service.send(Command::MmuSelectGate { gate: 2 });
    session
        .wait_until("gate 2 selected again", |snapshot| snapshot.status.mmu.as_ref().is_some_and(|mmu| mmu.gate == 2))
}

/// Dialog of a gate. In the 800x480 layout a tap on the tile of gate 2 opens it without moving the selector, and its
/// button "Select" selects the gate and closes it. Without Spoolman (profile `ercf`) material and colour go to
/// Happy Hare's gate map and the dialog marks what Happy Hare reports back.
pub(super) fn mmu_gate(session: &mut Session) -> Result<(), String> {
    let selected_gate = |session: &Session| session.presenter.snapshot().status.mmu.map_or(-1, |mmu| mmu.gate);
    if session.has_reference_layout() {
        let before = selected_gate(session);
        let (x, y) = GATE_2_TILE;
        session.tap(x, y);
        session.pump(Duration::from_millis(500));
        if session.window.global::<MmuDialogs>().get_gate() != 2 || selected_gate(session) != before {
            return Err("a tap on the tile of gate 2 does not open its dialog, or it moves the selector".into());
        }
        session.show_page(Page::Filament, "filament-mmu-gate")?;
        let (x, y) = GATE_DIALOG_SELECT;
        session.tap(x, y);
        session.wait_until("gate 2 selected in its dialog", |snapshot| {
            snapshot.status.mmu.as_ref().is_some_and(|mmu| mmu.gate == 2)
        })?;
        if session.window.global::<MmuDialogs>().get_gate() != -1 {
            return Err("the gate dialog stays open after Select".into());
        }
    }
    if session.window.global::<Printer>().get_has_spoolman() {
        return Ok(());
    }
    session.window.global::<MmuDialogs>().set_gate(1);
    {
        let actions = session.window.global::<Actions>();
        actions.invoke_set_gate_material(1, "PETG".into());
        actions.invoke_set_gate_color(1, slint::Color::from_rgb_u8(0xff, 0x80, 0x00));
    }
    session.wait_until("PETG in orange in gate 1", |snapshot| {
        let gate = snapshot.status.mmu.as_ref().and_then(|mmu| mmu.gates.get(1).cloned()).unwrap_or_default();
        gate.material == "PETG" && gate.color == Some([0xff, 0x80, 0x00, 0xff])
    })?;
    session.show_page(Page::Filament, "filament-mmu-gate-local")?;
    session.window.global::<MmuDialogs>().set_gate(-1);
    Ok(())
}

/// The filament path during a load: the fake holds the filament in the bowden, then lets it go back to the gate.
pub(super) fn filament_path_midway(session: &mut Session) -> Result<(), String> {
    let at = |position: i32| {
        move |snapshot: &freakymate_core::Snapshot| {
            snapshot.status.mmu.as_ref().is_some_and(|mmu| mmu.filament_pos == position)
        }
    };
    session.service.send(Command::RunGcode { script: "FAKE_MMU_FILAMENT POS=3 DIRECTION=1".into() });
    session.wait_until("filament in the bowden", at(3))?;
    session.window.global::<Navigation>().set_filament_tab(FilamentTab::Gates);
    session.show_page(Page::Filament, "filament-mmu-loading")?;
    session.service.send(Command::RunGcode { script: "FAKE_MMU_FILAMENT POS=0 DIRECTION=0".into() });
    session.wait_until("filament back in the gate", at(0))
}
