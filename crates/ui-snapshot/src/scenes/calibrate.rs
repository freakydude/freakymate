//! The Calibrate tab of the Move page and the bed mesh dialogs.

use crate::session::Session;
use freakymate::{Actions, MeshDialogs, MotionTab, Navigation, Page};
use freakymate_core::sources::klipper::{CalibrationMode, TestZ};
use freakymate_core::{Command, Snapshot};
use slint::ComponentHandle;
use std::time::Duration;

/// The Move entry of the navigation (800x480, no fitting).
const NAV_MOVE: (f32, f32) = (38.0, 155.0);

/// Measures what the printer can measure, then shows each waiting mode in its dialog and leaves it; at the end the
/// changes are saved, so the scenes after it start without pending ones.
pub(super) fn calibrate(session: &mut Session) -> Result<(), String> {
    let offered = session.presenter.snapshot().sources.klipper.calibration;
    session.window.global::<Navigation>().set_motion_tab(MotionTab::Calibrate);
    session.show_page(Page::Motion, "calibrate")?;
    if offered.bed_mesh {
        session.service.send(Command::CalibrateBedMesh);
        session.wait_until("bed mesh", |snapshot| {
            snapshot.status.calibration.bed_mesh.as_ref().is_some_and(|mesh| !mesh.active.is_empty())
        })?;
    }
    if offered.screws_tilt {
        session.service.send(Command::CalculateScrewsTilt);
        session.wait_until("screw results", |snapshot| !snapshot.status.calibration.screws.is_empty())?;
    }
    session.pump(Duration::from_millis(300));
    session.capture("calibrate-results")?;
    if offered.bed_mesh {
        bed_mesh_dialogs(session)?;
    }

    session.service.send(Command::CalibrateZ);
    session.wait_until("manual probe", |snapshot| snapshot.status.calibration.mode.is_some())?;
    session.service.send(Command::TestZ { step: TestZ::By(-1.0) });
    session.service.send(Command::TestZ { step: TestZ::BisectUp });
    session.wait_until("heights tried", |snapshot| {
        matches!(snapshot.status.calibration.mode, Some(CalibrationMode::ManualProbe { lower: Some(_), .. }))
    })?;
    session.pump(Duration::from_millis(300));
    session.capture("calibrate-probe")?;
    session.service.send(Command::AcceptCalibration);
    session.wait_until("probe accepted", |snapshot| snapshot.status.calibration.mode.is_none())?;
    session.pump(Duration::from_millis(300));
    session.capture("calibrate-pending")?;

    if offered.bed_screws {
        session.service.send(Command::AdjustBedScrews);
        session.wait_until("bed screws", |snapshot| snapshot.status.calibration.mode.is_some())?;
        session.service.send(Command::AcceptCalibration);
        session.wait_until("first screw accepted", |snapshot| {
            matches!(snapshot.status.calibration.mode, Some(CalibrationMode::BedScrews { accepted: 1, .. }))
        })?;
        session.pump(Duration::from_millis(300));
        session.capture("calibrate-screws")?;
        // Closing the dialog, here by a tap on the navigation, leaves the mode.
        if session.has_reference_layout() {
            let (x, y) = NAV_MOVE;
            session.tap(x, y);
        } else {
            session.service.send(Command::AbortCalibration);
        }
        session.wait_until("bed screws left", |snapshot| snapshot.status.calibration.mode.is_none())?;
    }
    session.service.send(Command::SaveConfig);
    session.wait_until("saved", |snapshot| !snapshot.status.save_config_pending)?;
    session.window.global::<Navigation>().set_motion_tab(MotionTab::Axes);
    Ok(())
}

/// The tiles of the bed mesh card on the Calibrate tab (800x480, no fitting): the map and the profile selector.
const MESH_MAP_TILE: (f32, f32) = (660.0, 178.0);

const MESH_PROFILE_TILE: (f32, f32) = (660.0, 240.0);

/// The map of the measured mesh with its faulty region, then the profiles: one saved under a new name and removed
/// again, so the scenes after it see the profiles they started with.
pub(super) fn bed_mesh_dialogs(session: &mut Session) -> Result<(), String> {
    if session.has_reference_layout() {
        let (x, y) = MESH_MAP_TILE;
        session.tap(x, y);
    } else {
        session.window.global::<MeshDialogs>().set_map_open(true);
    }
    session.pump(Duration::from_millis(300));
    if !session.window.global::<MeshDialogs>().get_map_open() {
        return Err("a tap on the mesh thumbnail did not open the map".into());
    }
    session.capture("calibrate-mesh")?;
    session.window.global::<MeshDialogs>().set_map_open(false);

    if session.has_reference_layout() {
        let (x, y) = MESH_PROFILE_TILE;
        session.tap(x, y);
    } else {
        session.window.global::<MeshDialogs>().set_profiles_open(true);
    }
    session.window.global::<Actions>().invoke_save_bed_mesh("smooth-pei".into());
    session.wait_until("profile saved", |snapshot| has_mesh_profile(snapshot, "smooth-pei"))?;
    session.pump(Duration::from_millis(300));
    if !session.window.global::<MeshDialogs>().get_profiles_open() {
        return Err("a tap on the profile selector did not open the profiles".into());
    }
    session.capture("calibrate-mesh-profiles")?;
    session.window.global::<Actions>().invoke_remove_bed_mesh("smooth-pei".into());
    session.wait_until("profile removed", |snapshot| !has_mesh_profile(snapshot, "smooth-pei"))?;
    session.window.global::<MeshDialogs>().set_profiles_open(false);
    Ok(())
}

pub(super) fn has_mesh_profile(snapshot: &Snapshot, name: &str) -> bool {
    snapshot
        .status
        .calibration
        .bed_mesh
        .as_ref()
        .is_some_and(|mesh| mesh.profiles.iter().any(|profile| profile == name))
}
