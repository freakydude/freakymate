//! Spoolman: the spool in use, the picker and the hints before a print.

use super::mmu::filament_path_midway;
use crate::session::Session;
use freakymate::{Confirm, Navigation, Page, Printer, SpoolPicker};
use freakymate_core::Command;
use slint::{ComponentHandle, ModelRc, VecModel};
use std::time::Duration;

/// Without Spoolman in Moonraker there is nothing to show and the scene is skipped.
pub(super) fn spools(session: &mut Session) -> Result<(), String> {
    session.settle_replies(Duration::from_secs(2));
    let snapshot = session.presenter.snapshot();
    if !snapshot.sources.moonraker.spoolman {
        return Ok(());
    }
    let has_mmu = snapshot.status.mmu.is_some();
    if has_mmu {
        session.service.send(Command::MmuHome);
        session.wait_until("MMU homed", |snapshot| snapshot.status.mmu.as_ref().is_some_and(|mmu| mmu.homed))?;
        session.service.send(Command::MmuSelectGate { gate: 1 });
        session
            .wait_until("gate 1 selected", |snapshot| snapshot.status.mmu.as_ref().is_some_and(|mmu| mmu.gate == 1))?;
        filament_path_midway(session)?;
    } else {
        session.service.send(Command::SetActiveSpool { spool_id: Some(6) });
    }
    session.settle_replies(Duration::from_secs(2));
    session.service.send(Command::SetHeaterTarget { heater: "extruder".into(), target: 210.0 });
    session.pump(Duration::from_secs(1));
    session.show_page(Page::Print, "print-spool")?;
    session.show_page(Page::Temperature, "temperature-spool")?;

    session.window.global::<Navigation>().set_page(Page::Print);
    // Opens like a tap on the strip or gate 1: the spool in use is selected.
    let current = session.window.global::<Printer>().get_spool_id();
    session.window.global::<SpoolPicker>().invoke_show(if has_mmu { 1 } else { -1 }, current);
    session.settle_replies(Duration::from_secs(2));
    session.capture("spool-picker")?;
    session.window.global::<SpoolPicker>().set_open(false);

    session.presenter.request_files();
    session.settle_replies(Duration::from_secs(2));
    let tools = session.presenter.start_tools("long_job.gcode");
    session.window.global::<Confirm>().invoke_ask_to_start(
        "long_job".into(),
        "long_job.gcode".into(),
        ModelRc::new(VecModel::from(tools)),
    );
    session.pump(Duration::from_millis(300));
    session.capture("confirm-start-hints")?;
    session.window.global::<Confirm>().set_open(false);
    session.service.send(Command::SetHeaterTarget { heater: "extruder".into(), target: 0.0 });
    session.pump(Duration::from_millis(500));
    Ok(())
}
