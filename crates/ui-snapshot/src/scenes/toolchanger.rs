//! A tool changer with three heads (fake profile `toolchanger`): the spool in use, the temperature hints, the picker for
//! a tool macro and the prepared file with its further tools.

use super::pages::prepared_job;
use crate::session::Session;
use freakymate::{Navigation, Page, Printer, SpoolPicker};
use freakymate_core::Command;
use slint::ComponentHandle;
use std::time::Duration;

/// Without a tool changer or without Spoolman the scene is skipped.
pub(super) fn toolchanger(session: &mut Session) -> Result<(), String> {
    session.settle_replies(Duration::from_secs(2));
    let snapshot = session.presenter.snapshot();
    if snapshot.status.toolchanger.is_none() || !snapshot.sources.moonraker.spoolman {
        return Ok(());
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
    session.window.global::<SpoolPicker>().invoke_show_tool(1, 3);
    session.settle_replies(Duration::from_secs(2));
    session.capture("toolchanger-tool-picker")?;
    session.window.global::<SpoolPicker>().set_open(false);

    prepared_job(session)?;
    for heater in ["extruder", "extruder1"] {
        session.service.send(Command::SetHeaterTarget { heater: heater.into(), target: 0.0 });
    }
    session.pump(Duration::from_millis(500));
    Ok(())
}
