//! A dense bed mesh (the scan of an Eddy probe, 30 x 40 points; 31 x 31 on a round bed): the UI draws a mesh of up to
//! 10 x 10 points tile by tile and a larger one as an image, which no other scene shows.

use crate::session::Session;
use sailhand::{Actions, MeshDialogs, MotionTab, Navigation, Page};
use sailhand_core::{Command, Snapshot};
use slint::ComponentHandle;
use std::time::Duration;

const PROFILE: &str = "dense";
/// `PROBE_COUNT` of a rectangular bed.
const PROBE_COUNT: (usize, usize) = (30, 40);
/// `ROUND_PROBE_COUNT` of a round bed (a fake started with `--round-bed`); Klipper reads only the one that fits.
const ROUND_PROBE_COUNT: usize = 31;

/// Scans the dense mesh, shows the card and the map, then removes the profile and loads `default` again, so the
/// scenes after it see the profiles they started with.
pub(super) fn bed_mesh_dense(session: &mut Session) -> Result<(), String> {
    if !session.presenter.snapshot().sources.klipper.calibration.bed_mesh {
        return Ok(());
    }
    session.window.global::<Navigation>().set_motion_tab(MotionTab::Calibrate);
    session.show_page(Page::Motion, "bed-mesh-dense")?;
    let (x_count, y_count) = PROBE_COUNT;
    session.service.send(Command::RunGcode {
        script: format!(
            "BED_MESH_CALIBRATE PROFILE={PROFILE} PROBE_COUNT={x_count},{y_count} ROUND_PROBE_COUNT={ROUND_PROBE_COUNT}"
        ),
    });
    session.wait_until("dense mesh", |snapshot| {
        mesh_columns(snapshot, PROFILE).is_some_and(|columns| columns == x_count || columns == ROUND_PROBE_COUNT)
    })?;
    session.pump(Duration::from_millis(300));
    session.capture("bed-mesh-dense")?;

    session.window.global::<MeshDialogs>().set_map_open(true);
    session.pump(Duration::from_millis(300));
    if !session.window.global::<MeshDialogs>().get_map_open() {
        return Err("the dense mesh map did not open".into());
    }
    session.capture("bed-mesh-dense-map")?;
    session.window.global::<MeshDialogs>().set_map_open(false);

    session.window.global::<Actions>().invoke_remove_bed_mesh(PROFILE.into());
    session.wait_until("dense profile removed", |snapshot| !super::calibrate::has_mesh_profile(snapshot, PROFILE))?;
    session.service.send(Command::RunGcode { script: "BED_MESH_PROFILE LOAD=default".into() });
    session.wait_until("default mesh loaded", |snapshot| mesh_columns(snapshot, "default").is_some())?;
    session.service.send(Command::SaveConfig);
    session.wait_until("saved", |snapshot| !snapshot.status.save_config_pending)?;
    session.window.global::<Navigation>().set_motion_tab(MotionTab::Axes);
    Ok(())
}

/// The number of columns of the active mesh when it is the profile `name`.
fn mesh_columns(snapshot: &Snapshot, name: &str) -> Option<usize> {
    let state = snapshot.status.calibration.bed_mesh.as_ref().filter(|state| state.active == name)?;
    state.mesh.as_ref().map(|mesh| mesh.columns())
}
