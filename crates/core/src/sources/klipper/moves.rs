//! Moves of the axes a person asks for: a step of the buttons, a typed position and the segments of the live pad.
//!
//! Klipper has two coordinates for them. The axis limits hold for the toolhead position, which includes the G-code
//! offset, the bed mesh and `z_thermal_adjust` (klippy/toolhead.py `commanded_pos`, checked by the kinematics). `G1`
//! takes G-code coordinates and adds those on top (klippy/extras/gcode_move.py `cmd_G1`: `last_position = value +
//! base_position`, then `move_with_transform`). So a move is limited by the toolhead position and sent as the G-code
//! position plus the same distance.

use super::job::{PrintJob, PrintState};
use super::motion::{Axis, Motion};
use super::setup::Setup;
use std::time::Duration;

/// Speeds of the buttons and the typed positions (mm/s).
pub const MOVE_SPEED_XY: f64 = 100.0;
pub const MOVE_SPEED_Z: f64 = 10.0;
/// How often the live pad sends a segment while it is held; each segment moves for this long, so Klipper's queue
/// stays short and the head stops soon after the finger lifts.
pub const DRIVE_INTERVAL: Duration = Duration::from_millis(100);
/// Near the touch point the pad does not move yet (share of the full deflection).
const DRIVE_DEAD_ZONE: f64 = 0.05;
/// Shorter moves are left out (mm).
const MIN_MOVE_MM: f64 = 0.001;

/// Where one axis ends up, in both coordinates.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct AxisTarget {
    pub axis: Axis,
    pub gcode: f64,
    pub toolhead: f64,
}

/// A move of one or more axes at `speed` (mm/s).
#[derive(Debug, Clone, PartialEq)]
pub struct PlannedMove {
    pub targets: Vec<AxisTarget>,
    pub speed: f64,
}

/// What limits the live pad besides the axis range.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DriveLimits {
    /// `max_velocity` and `max_z_velocity` of the printer (mm/s).
    pub max_xy_speed: f64,
    pub max_z_speed: f64,
    /// The pad does not drive Z below this G-code height.
    pub z_floor: f64,
}

/// The printer's speed limits and the lowest height for the pad: G-code Z 0, while a print is paused its height, so
/// the nozzle does not drive into the part.
pub fn drive_limits(setup: &Setup, motion: &Motion, job: &PrintJob) -> DriveLimits {
    let max_xy_speed = motion.max_velocity;
    // Klipper's default for `max_z_velocity` is `max_velocity` (klippy/kinematics/cartesian.py, corexy.py).
    let max_z_speed = setup.max_z_velocity.unwrap_or(max_xy_speed);
    DriveLimits { max_xy_speed, max_z_speed, z_floor: print_floor(job).unwrap_or(0.0) }
}

/// The height of a paused print (G-code coordinates), below which the pad does not drive; `None` otherwise.
pub fn print_floor(job: &PrintJob) -> Option<f64> {
    (job.state == PrintState::Paused).then_some(job.height).flatten().map(|height| height.max(0.0))
}

/// A step of the buttons, kept inside the axis range; `None` if the axis would not move towards `distance`.
pub fn jog(motion: &Motion, axis: Axis, distance: f64) -> Option<PlannedMove> {
    let target = shifted(motion, axis, distance, None)?;
    Some(PlannedMove { targets: vec![target], speed: move_speed(axis) })
}

/// A typed position in G-code coordinates; not limited, Klipper refuses what lies outside.
pub fn move_to(motion: &Motion, axis: Axis, position: f64) -> PlannedMove {
    let target = AxisTarget { axis, gcode: position, toolhead: toolhead_of(motion, axis, position) };
    PlannedMove { targets: vec![target], speed: move_speed(axis) }
}

/// Klipper would refuse a typed position (G-code coordinates) because it lies outside the axis range.
pub fn outside_range(motion: &Motion, axis: Axis, position: f64) -> bool {
    range(motion, axis)
        .is_some_and(|(minimum, maximum)| !(minimum..=maximum).contains(&toolhead_of(motion, axis, position)))
}

/// The axis range in G-code coordinates at the current position; `None` until Klipper reported limits.
pub fn gcode_range(motion: &Motion, axis: Axis) -> Option<(f64, f64)> {
    let offset = offset(motion, axis);
    range(motion, axis).map(|(minimum, maximum)| (minimum - offset, maximum - offset))
}

/// A G-code position in the coordinates of the toolhead, which the display shows.
pub fn toolhead_of(motion: &Motion, axis: Axis, gcode: f64) -> f64 {
    gcode + offset(motion, axis)
}

/// One segment of the live pad: `deflection` per axis from -1 to 1, the speed grows with its square up to
/// `max_speed` (mm/s) and the printer's limits. `None` while the pad rests or the axes cannot go further.
pub fn drive(motion: &Motion, deflection: [f64; 3], max_speed: f64, limits: &DriveLimits) -> Option<PlannedMove> {
    let seconds = DRIVE_INTERVAL.as_secs_f64();
    let [x, y, z] = deflection.map(|value| value.clamp(-1.0, 1.0));
    let mut targets = Vec::new();
    let length = x.hypot(y);
    let xy_speed = speed_for(length.min(1.0)) * max_speed.min(limits.max_xy_speed);
    if xy_speed > 0.0 {
        let step = xy_speed * seconds / length;
        targets.extend(shifted(motion, Axis::X, x * step, None));
        targets.extend(shifted(motion, Axis::Y, y * step, None));
    }
    let z_speed = speed_for(z.abs()) * max_speed.min(limits.max_z_speed);
    if z_speed > 0.0 {
        targets.extend(shifted(motion, Axis::Z, z.signum() * z_speed * seconds, Some(limits.z_floor)));
    }
    if targets.is_empty() {
        return None;
    }
    // A segment cut short at a limit keeps its duration, so the next one follows in time.
    let distance = targets
        .iter()
        .map(|target| target.gcode - motion.gcode_position[target.axis.index()])
        .map(|delta| delta * delta)
        .sum::<f64>()
        .sqrt();
    Some(PlannedMove { targets, speed: distance / seconds })
}

fn speed_for(deflection: f64) -> f64 {
    ((deflection - DRIVE_DEAD_ZONE) / (1.0 - DRIVE_DEAD_ZONE)).clamp(0.0, 1.0).powi(2)
}

/// `axis` moved by `distance` within the axis range and, going down, not below `floor` (G-code coordinates);
/// `None` if that leaves it where it is or would move it the other way (already outside the range).
fn shifted(motion: &Motion, axis: Axis, distance: f64, floor: Option<f64>) -> Option<AxisTarget> {
    let index = axis.index();
    let (toolhead, gcode) = (motion.position[index], motion.gcode_position[index]);
    let mut delta = match range(motion, axis) {
        Some((minimum, maximum)) => (toolhead + distance).clamp(minimum, maximum) - toolhead,
        None => distance,
    };
    if let Some(floor) = floor.filter(|_| delta < 0.0) {
        delta = delta.max((floor - gcode).min(0.0));
    }
    (delta * distance > 0.0 && delta.abs() >= MIN_MOVE_MM).then_some(AxisTarget {
        axis,
        gcode: gcode + delta,
        toolhead: toolhead + delta,
    })
}

fn range(motion: &Motion, axis: Axis) -> Option<(f64, f64)> {
    let index = axis.index();
    let (minimum, maximum) = (motion.axis_minimum[index], motion.axis_maximum[index]);
    (maximum > minimum).then_some((minimum, maximum))
}

/// Toolhead minus G-code position: G-code offset, tool offset, bed mesh and `z_thermal_adjust` at this point.
fn offset(motion: &Motion, axis: Axis) -> f64 {
    motion.position[axis.index()] - motion.gcode_position[axis.index()]
}

fn move_speed(axis: Axis) -> f64 {
    if axis == Axis::Z { MOVE_SPEED_Z } else { MOVE_SPEED_XY }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Z offset −0.05 and 0.12 of bed mesh at this point: the toolhead stands 0.07 above the G-code position.
    fn motion() -> Motion {
        Motion {
            position: [100.0, 50.0, 5.07, 0.0],
            gcode_position: [100.0, 50.0, 5.0, 0.0],
            axis_minimum: [0.0, 0.0, -2.0, 0.0],
            axis_maximum: [300.0, 300.0, 250.0, 0.0],
            ..Motion::default()
        }
    }

    const LIMITS: DriveLimits = DriveLimits { max_xy_speed: 300.0, max_z_speed: 20.0, z_floor: 0.0 };

    fn single(planned: Option<PlannedMove>) -> AxisTarget {
        let planned = planned.expect("a move");
        assert_eq!(planned.targets.len(), 1);
        planned.targets[0]
    }

    #[test]
    fn a_step_moves_by_its_distance_whatever_the_offsets() {
        let target = single(jog(&motion(), Axis::Z, 0.1));
        assert!((target.gcode - 5.1).abs() < 1e-9);
        assert!((target.toolhead - 5.17).abs() < 1e-9);
    }

    #[test]
    fn a_step_stops_at_the_axis_limit_in_toolhead_coordinates() {
        let mut motion = motion();
        motion.position[0] = 299.5;
        motion.gcode_position[0] = 298.0;
        let target = single(jog(&motion, Axis::X, 10.0));
        assert_eq!((target.toolhead, target.gcode), (300.0, 298.5));
        motion.position[0] = 300.0;
        assert_eq!(jog(&motion, Axis::X, 1.0), None);
        // Beyond the limit already (mesh), a step outwards must not pull it back.
        motion.position[0] = 300.2;
        assert_eq!(jog(&motion, Axis::X, 1.0), None);
    }

    #[test]
    fn a_typed_position_is_taken_in_gcode_coordinates() {
        let target = single(Some(move_to(&motion(), Axis::Z, 10.0)));
        assert_eq!(target.gcode, 10.0);
        assert!((target.toolhead - 10.07).abs() < 1e-9);
        assert!(outside_range(&motion(), Axis::Z, 249.95));
        assert!(!outside_range(&motion(), Axis::Z, 249.9));
        let (minimum, maximum) = gcode_range(&motion(), Axis::Z).unwrap();
        assert!((minimum + 2.07).abs() < 1e-9 && (maximum - 249.93).abs() < 1e-9);
        assert_eq!(gcode_range(&Motion::default(), Axis::X), None);
    }

    #[test]
    fn the_pad_speed_grows_with_the_square_of_the_deflection() {
        let full = drive(&motion(), [1.0, 0.0, 0.0], 50.0, &LIMITS).unwrap();
        assert!((full.speed - 50.0).abs() < 1e-9);
        assert!((full.targets[0].gcode - 105.0).abs() < 1e-9);
        let half = drive(&motion(), [0.525, 0.0, 0.0], 50.0, &LIMITS).unwrap();
        assert!((half.speed - 12.5).abs() < 1e-9);
        assert_eq!(drive(&motion(), [0.04, 0.0, 0.0], 50.0, &LIMITS), None);
        // Diagonal: the deflection is a vector, its length counts (capped at 1).
        let diagonal = drive(&motion(), [1.0, 1.0, 0.0], 50.0, &LIMITS).unwrap();
        assert_eq!(diagonal.targets.len(), 2);
        assert!((diagonal.speed - 50.0).abs() < 1e-9);
    }

    #[test]
    fn the_pad_keeps_to_the_printer_limits_and_the_floor() {
        let fast = drive(&motion(), [0.0, 0.0, 1.0], 150.0, &LIMITS).unwrap();
        assert!((fast.speed - 20.0).abs() < 1e-9);
        let mut low = motion();
        low.position[2] = 0.17;
        low.gcode_position[2] = 0.1;
        let down = single(drive(&low, [0.0, 0.0, -1.0], 20.0, &LIMITS));
        assert!(down.gcode.abs() < 1e-9);
        low.gcode_position[2] = 0.0;
        assert_eq!(drive(&low, [0.0, 0.0, -1.0], 20.0, &LIMITS), None);
        // Below the floor already: down stays refused, up is free.
        low.gcode_position[2] = -0.5;
        assert_eq!(drive(&low, [0.0, 0.0, -1.0], 20.0, &LIMITS), None);
        assert!(drive(&low, [0.0, 0.0, 1.0], 20.0, &LIMITS).is_some());
    }
}
