//! The Calibrate tab and the dialog of a calibration mode: what the printer offers, the bed mesh with its profiles,
//! the screw results, the waiting mode and what `SAVE_CONFIG` would write.

use crate::bed_mesh_view;
use crate::items::replace_rows;
use crate::{BedMeshMap, EddyValue, MeshCellItem, MeshFrameItem, Printer, ScrewItem};
use sailhand_core::domain::manual_probe::{Direction, Heights, ProbeSteps};
use sailhand_core::sources::klipper::{
    BedMesh, CalibrationMode, CalibrationState, EddyOption, EddyProbe, PendingSection, ScrewAdjustment, Setup,
    tap_height_fits,
};
use slint::{ModelRc, SharedString, VecModel};
use std::cell::RefCell;
use std::rc::Rc;

/// Longer values (a mesh's points) are left out of the pending lines; a section with several options shows their
/// number only.
const SHORT_VALUE: usize = 12;

#[derive(Default)]
pub struct CalibrationView {
    profiles: Rc<VecModel<SharedString>>,
    screws: Rc<VecModel<ScrewItem>>,
    mesh_cells: Rc<VecModel<MeshCellItem>>,
    mesh_frames: Rc<VecModel<MeshFrameItem>>,
    /// The mesh last drawn: the picture is built again only when the mesh changes.
    drawn_mesh: RefCell<Option<BedMesh>>,
    probe: RefCell<ProbeState>,
}

/// The paper test: the heights last reported and the coarse ladder the buttons go by.
#[derive(Default)]
struct ProbeState {
    heights: Option<Heights>,
    steps: ProbeSteps,
}

impl CalibrationView {
    pub fn bind(&self, printer: &Printer) {
        printer.set_bed_mesh_profiles(ModelRc::from(self.profiles.clone()));
        printer.set_screw_results(ModelRc::from(self.screws.clone()));
        printer.set_bed_mesh_cells(ModelRc::from(self.mesh_cells.clone()));
        printer.set_bed_mesh_frames(ModelRc::from(self.mesh_frames.clone()));
    }

    pub fn show(&self, printer: &Printer, setup: &Setup, calibration: &CalibrationState) {
        let offered = &setup.calibration;
        printer.set_has_bed_mesh(offered.bed_mesh);
        printer.set_has_screws_tilt(offered.screws_tilt);
        printer.set_has_bed_screws(offered.bed_screws);
        printer.set_z_calibration(match (setup.probe, offered.z_endstop) {
            (true, _) => "probe".into(),
            (false, true) => "endstop".into(),
            (false, false) => SharedString::new(),
        });

        let mesh = calibration.bed_mesh.as_ref();
        printer.set_bed_mesh_active(mesh.map(|mesh| mesh.active.as_str()).unwrap_or_default().into());
        let profiles = mesh.map(|mesh| mesh.profiles.iter().map(|profile| profile.into()).collect());
        replace_rows(&self.profiles, profiles.unwrap_or_default());
        self.show_mesh(printer, mesh.and_then(|mesh| mesh.mesh.as_ref()));
        replace_rows(&self.screws, calibration.screws.iter().map(screw_item).collect());
        let pending: Vec<String> = calibration.pending.iter().map(pending_line).collect();
        printer.set_pending_config(pending.join(" · ").into());
        show_eddy(printer, calibration.eddy.as_ref());
        show_mode(printer, calibration.mode.as_ref());
        self.show_probe_steps(printer, calibration.mode.as_ref());
    }

    /// Whether the toolhead height suits the tap steps of the Eddy probe; only a hint.
    pub fn show_toolhead_height(&self, printer: &Printer, calibration: &CalibrationState, z: f64) {
        printer.set_eddy_tap_height_off(calibration.eddy.is_some() && !tap_height_fits(z));
    }

    /// A coarse press in `direction`: the distance to send, if the button is on.
    pub fn press_coarse(&self, direction: Direction) -> Option<f64> {
        let mut probe = self.probe.borrow_mut();
        let heights = probe.heights?;
        probe.steps.press_coarse(heights, direction)
    }

    /// A move other than a coarse press (fine, fixed step): the ladder starts over.
    pub fn probe_moved(&self) {
        self.probe.borrow_mut().steps.reset();
    }

    /// The target on each of the four buttons; the ladder starts over when no paper test runs.
    fn show_probe_steps(&self, printer: &Printer, mode: Option<&CalibrationMode>) {
        let mut probe = self.probe.borrow_mut();
        probe.heights = match mode {
            Some(CalibrationMode::ManualProbe { z, lower, upper }) => {
                Some(Heights { z: *z, lower: *lower, upper: *upper })
            }
            _ => None,
        };
        let Some(heights) = probe.heights else {
            probe.steps.reset();
            return;
        };
        probe.steps.observe(heights.z);
        let text = |target: f64| SharedString::from(format!("{target:.3}"));
        let coarse = |direction| probe.steps.coarse_target(heights, direction);
        printer.set_probe_fine_down(text(ProbeSteps::fine_target(heights, Direction::Down)));
        printer.set_probe_fine_up(text(ProbeSteps::fine_target(heights, Direction::Up)));
        printer.set_probe_coarse_down(coarse(Direction::Down).map(text).unwrap_or_default());
        printer.set_probe_coarse_up(coarse(Direction::Up).map(text).unwrap_or_default());
    }
}

impl CalibrationView {
    fn show_mesh(&self, printer: &Printer, mesh: Option<&BedMesh>) {
        let mut drawn = self.drawn_mesh.borrow_mut();
        if drawn.as_ref() == mesh {
            return;
        }
        *drawn = mesh.cloned();
        let (map, cells, frames) = match mesh.map(bed_mesh_view::picture) {
            Some(picture) => (picture.map, picture.cells, picture.frames),
            None => (BedMeshMap::default(), Vec::new(), Vec::new()),
        };
        printer.set_bed_mesh_map(map);
        self.mesh_cells.set_vec(cells);
        self.mesh_frames.set_vec(frames);
    }
}

fn show_eddy(printer: &Printer, eddy: Option<&EddyProbe>) {
    let Some(eddy) = eddy else {
        printer.set_eddy_section(SharedString::new());
        printer.set_eddy_has_offset(false);
        return;
    };
    printer.set_eddy_section(eddy.section().into());
    printer.set_eddy_has_offset(eddy.has_offset());
    printer.set_eddy_drive_current(eddy_value(&eddy.drive_current, String::clone));
    printer.set_eddy_calibration(eddy_value(&eddy.calibration_points, |points| points_text(*points)));
    printer.set_eddy_tap_threshold(eddy_value(&eddy.tap_threshold, String::clone));
}

fn eddy_value<T>(option: &EddyOption<T>, text: impl Fn(&T) -> String) -> EddyValue {
    let show = |value: &Option<T>| value.as_ref().map(&text).unwrap_or_default().into();
    EddyValue { current: show(&option.current), pending: show(&option.pending) }
}

/// The table of the `calibrate` option is shown by its size.
fn points_text(points: usize) -> String {
    format!("{points} {}", if points == 1 { "point" } else { "points" })
}

fn show_mode(printer: &Printer, mode: Option<&CalibrationMode>) {
    let height = |z: Option<f64>| z.map(|z| format!("{z:.3}")).unwrap_or_default().into();
    match mode {
        Some(CalibrationMode::ManualProbe { z, lower, upper }) => {
            printer.set_calibration_mode("manual-probe".into());
            printer.set_probe_z(height(Some(*z)));
            printer.set_probe_z_lower(height(*lower));
            printer.set_probe_z_upper(height(*upper));
        }
        Some(CalibrationMode::BedScrews { screw, fine, accepted, screws }) => {
            printer.set_calibration_mode("bed-screws".into());
            printer.set_bed_screw(screw.as_str().into());
            printer.set_bed_screw_fine(*fine);
            printer.set_bed_screws_accepted(*accepted as i32);
            printer.set_bed_screws_count(*screws as i32);
        }
        None => printer.set_calibration_mode(SharedString::new()),
    }
}

fn screw_item(screw: &ScrewAdjustment) -> ScrewItem {
    ScrewItem {
        name: screw.name.as_str().into(),
        turns: screw.turns.as_str().into(),
        clockwise: screw.clockwise,
        is_base: screw.is_base,
        level: screw.is_level(),
    }
}

/// `probe: z_offset = 0.550`, `bed_mesh default: 8 values`, `bed_mesh old: removed`.
fn pending_line(section: &PendingSection) -> String {
    let name = &section.section;
    match section.options.as_deref() {
        None => format!("{name}: removed"),
        Some([(option, value)]) if value.len() <= SHORT_VALUE && !value.contains('\n') => {
            format!("{name}: {option} = {}", value.trim())
        }
        Some([(option, _)]) => format!("{name}: {option}"),
        Some(options) => format!("{name}: {} values", options.len()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_table_is_shown_by_its_points() {
        assert_eq!(points_text(48), "48 points");
        assert_eq!(points_text(1), "1 point");
    }

    #[test]
    fn pending_lines_show_short_values_only() {
        let option = |name: &str, value: &str| (name.to_owned(), value.to_owned());
        let probe = PendingSection { section: "probe".into(), options: Some(vec![option("z_offset", "0.550")]) };
        assert_eq!(pending_line(&probe), "probe: z_offset = 0.550");
        let mesh = PendingSection {
            section: "bed_mesh default".into(),
            options: Some(vec![option("points", "\n0.1, 0.2\n0.3, 0.4"), option("x_count", "2")]),
        };
        assert_eq!(pending_line(&mesh), "bed_mesh default: 2 values");
        let removed = PendingSection { section: "bed_mesh old".into(), options: None };
        assert_eq!(pending_line(&removed), "bed_mesh old: removed");
    }
}
