//! The Axes tab of the Motion page: the area of the live pad and the number entries of the axes.

use crate::{AxesAreaItem, AxisEntryItem, format};
use sailhand_core::sources::klipper::moves;
use sailhand_core::sources::klipper::{Axis, Motion, PrintJob};

const AXES: [Axis; 3] = [Axis::X, Axis::Y, Axis::Z];

/// The reachable area in the displayed coordinates (X and Y of the toolhead, Z as G-code height); Z from the floor of
/// the pad.
pub fn axes_area(motion: &Motion, job: &PrintJob) -> AxesAreaItem {
    let ranges = AXES.map(|axis| moves::entry_range(motion, axis));
    let known = ranges.iter().all(Option::is_some);
    let [x, y, z] = ranges.map(Option::unwrap_or_default);
    let print_floor = moves::print_floor(job);
    let floor = print_floor.unwrap_or(0.0).max(z.0);
    AxesAreaItem {
        known,
        x_min: x.0 as f32,
        x_max: x.1 as f32,
        y_min: y.0 as f32,
        y_max: y.1 as f32,
        z_floor: floor as f32,
        z_max: z.1 as f32,
        floor_is_print: print_floor.is_some(),
    }
}

/// Per axis X, Y, Z: the position a typed one starts from, and the range Klipper takes.
pub fn axis_entries(motion: &Motion) -> Vec<AxisEntryItem> {
    AXES.iter()
        .map(|&axis| {
            let position = moves::entry_position(motion, axis);
            let range = moves::entry_range(motion, axis).map(|(low, high)| format::axis_range(low, high));
            AxisEntryItem {
                position: position as f32,
                now: format::axis_position(position).into(),
                range: range.unwrap_or_default().into(),
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use sailhand_core::sources::klipper::PrintState;

    /// Skew correction pulls X 0.4 below its G-code position; Z offset and mesh lift Z by 0.07.
    fn motion() -> Motion {
        Motion {
            position: [149.6, 150.0, 10.07, 0.0],
            gcode_position: [150.0, 150.0, 10.0, 0.0],
            axis_minimum: [0.0, 0.0, -2.0, 0.0],
            axis_maximum: [301.0, 309.0, 320.0, 0.0],
            ..Motion::default()
        }
    }

    #[test]
    fn x_and_y_keep_the_configured_range_and_z_counts_from_the_bed() {
        let area = axes_area(&motion(), &PrintJob::default());
        assert!(area.known && !area.floor_is_print);
        assert_eq!((area.x_min, area.x_max), (0.0, 301.0));
        assert!(area.z_floor.abs() < 1e-5 && (area.z_max - 319.93).abs() < 1e-4);
        let paused = PrintJob { state: PrintState::Paused, height: Some(12.4), ..PrintJob::default() };
        let area = axes_area(&motion(), &paused);
        assert!(area.floor_is_print && (area.z_floor - 12.4).abs() < 1e-4);
        assert!(!axes_area(&Motion::default(), &paused).known);

        let entries = axis_entries(&motion());
        assert_eq!(entries[0].now, "149.60 mm");
        assert_eq!(entries[2].now, "10.00 mm");
        assert_eq!(entries[2].range, "-2.1–319.9 mm");
        assert_eq!(entries[0].range, "0.0–301.0 mm");
    }
}
