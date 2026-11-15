//! The bed map of the Objects dialog: the outlines of the print's objects and a grid as SVG path commands. The view
//! box is the bed in mm with Y upwards as in the slicer; Slint scales a path evenly, so the map has the bed's shape.
//! Free of Slint types and therefore testable.

use freakymate_core::sources::klipper::{Bed, BedPoint, PrintObject, object_at};

/// Grid lines every this many mm, counted from Klipper's origin.
const GRID_STEP_MM: f64 = 50.0;
/// Outline points closer than this to the previous one are left out: the map has about one pixel per mm, and the
/// rounded parts of a slicer outline have many points.
const MIN_STEP_MM: f64 = 0.5;
/// Half the size of the marker for an object known only by its centre, as a share of the bed width; a symbol, not
/// the size of the object.
const MARKER_SHARE: f64 = 0.015;

/// A map only helps if the bed is known and the slicer told where at least one object lies.
pub fn shows_map(objects: &[PrintObject], bed: Option<Bed>) -> bool {
    bed.is_some() && objects.iter().any(PrintObject::is_located)
}

/// The object's outline, or a diamond around its centre; empty if the slicer told neither.
pub fn outline(object: &PrintObject, bed: Bed) -> String {
    if object.has_outline() {
        return polygon(&thinned(&object.outline), bed);
    }
    let Some(center) = object.center else { return String::new() };
    let size = bed.width() * MARKER_SHARE;
    let diamond = [(0.0, size), (size, 0.0), (0.0, -size), (-size, 0.0)]
        .map(|(dx, dy)| BedPoint { x: center.x + dx, y: center.y + dy });
    polygon(&diamond, bed)
}

/// Lines at round coordinates across the bed.
pub fn grid(bed: Bed) -> String {
    let mut commands = Vec::new();
    for x in steps(bed.min.x, bed.max.x) {
        let (from, to) = (view(bed, BedPoint { x, y: bed.min.y }), view(bed, BedPoint { x, y: bed.max.y }));
        commands.push(format!("M {:.1} {:.1} L {:.1} {:.1}", from.0, from.1, to.0, to.1));
    }
    for y in steps(bed.min.y, bed.max.y) {
        let (from, to) = (view(bed, BedPoint { x: bed.min.x, y }), view(bed, BedPoint { x: bed.max.x, y }));
        commands.push(format!("M {:.1} {:.1} L {:.1} {:.1}", from.0, from.1, to.0, to.1));
    }
    commands.join(" ")
}

/// The object under a tap on the map; the position and the finger's reach are shares of the map's width and height,
/// measured from the top left.
pub fn object_at_tap(objects: &[PrintObject], bed: Bed, x_share: f64, y_share: f64, reach_share: f64) -> Option<usize> {
    let point = BedPoint { x: bed.min.x + x_share * bed.width(), y: bed.max.y - y_share * bed.depth() };
    object_at(objects, point, reach_share * bed.width())
}

/// Multiples of the grid step strictly inside the range; the edges are drawn by the map's frame.
fn steps(from: f64, to: f64) -> impl Iterator<Item = f64> {
    let first = (from / GRID_STEP_MM).floor() as i64 + 1;
    let last = (to / GRID_STEP_MM).ceil() as i64 - 1;
    (first..=last).map(|step| step as f64 * GRID_STEP_MM)
}

fn thinned(outline: &[BedPoint]) -> Vec<BedPoint> {
    let mut kept: Vec<BedPoint> = Vec::with_capacity(outline.len());
    for &point in outline {
        if kept.last().is_none_or(|last| last.distance(point) >= MIN_STEP_MM) {
            kept.push(point);
        }
    }
    kept
}

fn polygon(points: &[BedPoint], bed: Bed) -> String {
    let mut commands: Vec<String> = points
        .iter()
        .enumerate()
        .map(|(index, &point)| {
            let (x, y) = view(bed, point);
            let command = if index == 0 { 'M' } else { 'L' };
            format!("{command} {x:.1} {y:.1}")
        })
        .collect();
    commands.push("Z".to_owned());
    commands.join(" ")
}

/// Bed coordinates to the view box: from the bed's left edge, and from its back edge downwards.
fn view(bed: Bed, point: BedPoint) -> (f64, f64) {
    (point.x - bed.min.x, bed.max.y - point.y)
}

#[cfg(test)]
mod tests {
    use super::*;

    const BED: Bed = Bed { min: BedPoint { x: 0.0, y: 0.0 }, max: BedPoint { x: 300.0, y: 200.0 } };

    fn located(outline: &[(f64, f64)], center: Option<(f64, f64)>) -> PrintObject {
        PrintObject {
            name: "PART".into(),
            outline: outline.iter().map(|&(x, y)| BedPoint { x, y }).collect(),
            center: center.map(|(x, y)| BedPoint { x, y }),
            ..PrintObject::default()
        }
    }

    #[test]
    fn the_outline_is_drawn_with_y_upwards() {
        let cube = located(&[(10.0, 10.0), (30.0, 10.0), (30.0, 30.0), (10.0, 30.0)], None);
        assert_eq!(outline(&cube, BED), "M 10.0 190.0 L 30.0 190.0 L 30.0 170.0 L 10.0 170.0 Z");
    }

    #[test]
    fn close_points_of_a_rounded_outline_are_left_out() {
        let round = located(&[(10.0, 10.0), (10.1, 10.0), (10.2, 10.1), (20.0, 10.0), (20.0, 20.0)], None);
        assert_eq!(outline(&round, BED), "M 10.0 190.0 L 20.0 190.0 L 20.0 180.0 Z");
    }

    #[test]
    fn an_object_known_only_by_its_centre_gets_a_marker() {
        let pin = located(&[], Some((100.0, 100.0)));
        assert_eq!(outline(&pin, BED), "M 100.0 95.5 L 104.5 100.0 L 100.0 104.5 L 95.5 100.0 Z");
        assert_eq!(outline(&located(&[], None), BED), "");
    }

    #[test]
    fn the_map_needs_the_bed_and_a_located_object() {
        let names_only = [located(&[], None)];
        let pin = [located(&[], Some((1.0, 1.0)))];
        assert!(shows_map(&pin, Some(BED)));
        assert!(!shows_map(&names_only, Some(BED)), "the slicer only named the objects");
        assert!(!shows_map(&pin, None), "the limits are not known yet");
        assert!(!shows_map(&[], Some(BED)), "the slicer defined no objects");
    }

    #[test]
    fn the_grid_has_lines_at_round_coordinates_inside_the_bed() {
        let bed = Bed { min: BedPoint { x: -5.0, y: 0.0 }, max: BedPoint { x: 100.0, y: 100.0 } };
        assert_eq!(grid(bed), "M 5.0 100.0 L 5.0 0.0 M 55.0 100.0 L 55.0 0.0 M 0.0 50.0 L 105.0 50.0");
    }

    #[test]
    fn a_tap_is_measured_from_the_top_left() {
        let objects = [located(&[(0.0, 180.0), (30.0, 180.0), (30.0, 200.0), (0.0, 200.0)], None)];
        assert_eq!(object_at_tap(&objects, BED, 0.05, 0.05, 0.0), Some(0), "back left corner");
        assert_eq!(object_at_tap(&objects, BED, 0.05, 0.95, 0.0), None, "front left corner");
        assert_eq!(object_at_tap(&objects, BED, 0.11, 0.05, 0.02), Some(0), "3 mm off within 6 mm reach");
        assert_eq!(object_at_tap(&objects, BED, 0.11, 0.05, 0.005), None, "3 mm off beyond 1.5 mm reach");
    }
}
