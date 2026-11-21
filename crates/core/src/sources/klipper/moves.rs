//! Moves of the axes a person asks for: a step of the buttons, a typed position and the segments of the live pad.
//!
//! Klipper has two coordinates for them. The axis limits hold for the toolhead position, which includes the G-code
//! offset, skew correction, the bed mesh and `z_thermal_adjust` (klippy/toolhead.py `commanded_pos`, checked by the
//! kinematics). `G1` takes G-code coordinates and adds those on top (klippy/extras/gcode_move.py `cmd_G1`:
//! `last_position = value + base_position`, then `move_with_transform`). So a move is limited by the toolhead position
//! and sent as the G-code position plus the same distance.
//!
//! A person reads X and Y where the toolhead stands, in the configured axis range, and Z as the height over the bed
//! in G-code coordinates (0 = nozzle on the bed with the Z offset): [`shown_position`], [`entry_position`].
//!
//! Klipper plans a move that reaches an idle toolhead only after 100 ms without a further command and starts it
//! 250 ms later (klippy/toolhead.py `PRIMING_CMD_TIME`, `BUFFER_TIME_START`, `_calc_print_time`). Moves join without
//! a stop only once a second of them is queued (`BUFFER_TIME_HIGH`). The live pad therefore keeps more than the start
//! delay queued ahead of the head, in segments that each run from rest to rest.

use super::job::{PrintJob, PrintState};
use super::motion::{Axis, Motion};
use super::setup::Setup;
use std::time::Duration;

/// Speeds of the buttons and the typed positions (mm/s).
pub const MOVE_SPEED_XY: f64 = 100.0;
pub const MOVE_SPEED_Z: f64 = 10.0;
/// How often the live pad sends a segment while it is held: longer than Klipper's 100 ms wait, so each segment is
/// planned on its own and its duration is known.
pub const DRIVE_INTERVAL: Duration = Duration::from_millis(150);
/// Klipper's wait before it plans moves for an idle toolhead plus the head start it gives them (s):
/// klippy/toolhead.py `PRIMING_CMD_TIME` 0.1 + `BUFFER_TIME_START` 0.25.
pub const KLIPPER_START_DELAY: f64 = 0.35;
/// Reserve for a segment on its way to Klipper (s).
const DRIVE_MARGIN: f64 = 0.05;
/// How far the queued moves reach ahead of the head after each segment of the pad (s): until the next segment is
/// planned and started. The head runs on for up to this long after the finger lifts.
const DRIVE_LEAD: f64 = DRIVE_INTERVAL.as_secs_f64() + KLIPPER_START_DELAY + DRIVE_MARGIN;
/// A shorter segment waits for the next tick (s).
const MIN_SEGMENT_SECONDS: f64 = 0.02;
/// Near the touch point the pad does not move yet (share of the full deflection).
const DRIVE_DEAD_ZONE: f64 = 0.05;
/// Shorter moves are left out (mm).
const MIN_MOVE_MM: f64 = 0.001;
const AXES: [Axis; 3] = [Axis::X, Axis::Y, Axis::Z];

/// Where one axis ends up, in both coordinates.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct AxisTarget {
    pub axis: Axis,
    pub gcode: f64,
    pub toolhead: f64,
}

/// A move of one or more axes at `speed` (mm/s), which takes Klipper about `seconds`.
#[derive(Debug, Clone, PartialEq)]
pub struct PlannedMove {
    pub targets: Vec<AxisTarget>,
    pub speed: f64,
    pub seconds: f64,
}

/// What Klipper allows a move: `[printer]` and the toolhead status.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MoveLimits {
    /// mm/s
    pub max_velocity: f64,
    pub max_z_velocity: f64,
    /// mm/s²
    pub max_accel: f64,
    pub max_z_accel: f64,
    pub minimum_cruise_ratio: f64,
}

/// What limits the live pad besides the axis range.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DriveLimits {
    pub moves: MoveLimits,
    /// The pad does not drive Z below this G-code height.
    pub z_floor: f64,
}

pub fn move_limits(setup: &Setup, motion: &Motion) -> MoveLimits {
    // Klipper's defaults: `max_z_velocity` = `max_velocity`, `max_z_accel` = `max_accel` and never above it
    // (klippy/kinematics/corexy.py, cartesian.py).
    MoveLimits {
        max_velocity: motion.max_velocity,
        max_z_velocity: setup.max_z_velocity.unwrap_or(motion.max_velocity),
        max_accel: motion.max_accel,
        max_z_accel: setup.max_z_accel.unwrap_or(motion.max_accel).min(motion.max_accel),
        minimum_cruise_ratio: motion.minimum_cruise_ratio,
    }
}

/// The printer's limits and the lowest height for the pad: G-code Z 0, while a print is paused its height, so the
/// nozzle does not drive into the part.
pub fn drive_limits(setup: &Setup, motion: &Motion, job: &PrintJob) -> DriveLimits {
    DriveLimits { moves: move_limits(setup, motion), z_floor: print_floor(job).unwrap_or(0.0) }
}

/// The height of a paused print (G-code coordinates), below which the pad does not drive; `None` otherwise.
pub fn print_floor(job: &PrintJob) -> Option<f64> {
    (job.state == PrintState::Paused).then_some(job.height).flatten().map(|height| height.max(0.0))
}

/// The position a person reads, X, Y, Z: X and Y where the toolhead stands, Z as the G-code height. Taken from the
/// live position, so it follows the head during a move.
pub fn shown_position(motion: &Motion) -> [f64; 3] {
    let [x, y, z, _] = motion.live_position;
    [x, y, z - offset(motion, Axis::Z)]
}

/// A step of the buttons, kept inside the axis range; `None` if the axis would not move towards `distance`.
pub fn jog(motion: &Motion, axis: Axis, distance: f64, limits: &MoveLimits) -> Option<PlannedMove> {
    let target = shifted(motion, axis, distance, None)?;
    Some(planned(motion, vec![target], move_speed(axis), limits))
}

/// A typed position, read like [`entry_position`]; not limited, Klipper refuses what lies outside.
pub fn move_to(motion: &Motion, axis: Axis, position: f64, limits: &MoveLimits) -> PlannedMove {
    let gcode = position + entry_offset(motion, axis) - offset(motion, axis);
    let target = AxisTarget { axis, gcode, toolhead: toolhead_of(motion, axis, gcode) };
    planned(motion, vec![target], move_speed(axis), limits)
}

/// Where a typed position starts: X and Y where the toolhead stands, Z as the G-code height.
pub fn entry_position(motion: &Motion, axis: Axis) -> f64 {
    motion.position[axis.index()] - entry_offset(motion, axis)
}

/// Klipper would refuse a typed position because it lies outside the axis range.
pub fn outside_range(motion: &Motion, axis: Axis, position: f64) -> bool {
    range(motion, axis)
        .is_some_and(|(minimum, maximum)| !(minimum..=maximum).contains(&(position + entry_offset(motion, axis))))
}

/// The axis range for a typed position at the current position; `None` until Klipper reported limits.
pub fn entry_range(motion: &Motion, axis: Axis) -> Option<(f64, f64)> {
    let offset = entry_offset(motion, axis);
    range(motion, axis).map(|(minimum, maximum)| (minimum - offset, maximum - offset))
}

/// A G-code position in the coordinates of the toolhead.
pub fn toolhead_of(motion: &Motion, axis: Axis, gcode: f64) -> f64 {
    gcode + offset(motion, axis)
}

/// One segment of the live pad: `deflection` per axis from -1 to 1, the speed grows with its square up to
/// `max_speed` (mm/s) and the printer's limits. It runs until the queued moves reach [`DRIVE_LEAD`] ahead. `None`
/// while the pad rests, enough is queued or the axes cannot go further.
pub fn drive(motion: &Motion, deflection: [f64; 3], max_speed: f64, limits: &DriveLimits) -> Option<PlannedMove> {
    let seconds = DRIVE_LEAD - motion.queued_seconds.max(KLIPPER_START_DELAY);
    if seconds < MIN_SEGMENT_SECONDS {
        return None;
    }
    let velocity = drive_velocity(deflection, max_speed, &limits.moves);
    let speed = norm(velocity);
    if speed <= 0.0 {
        return None;
    }
    let length = Profile::new(velocity, speed, &limits.moves).length(seconds);
    let targets: Vec<AxisTarget> = AXES
        .into_iter()
        .filter(|axis| velocity[axis.index()] != 0.0)
        .filter_map(|axis| {
            let floor = (axis == Axis::Z).then_some(limits.z_floor);
            shifted(motion, axis, velocity[axis.index()] / speed * length, floor)
        })
        .collect();
    (!targets.is_empty()).then(|| planned(motion, targets, speed, &limits.moves))
}

/// Speed per axis X, Y, Z (mm/s) for a deflection of the pad.
fn drive_velocity(deflection: [f64; 3], max_speed: f64, limits: &MoveLimits) -> [f64; 3] {
    let [x, y, z] = deflection.map(|value| value.clamp(-1.0, 1.0));
    let length = x.hypot(y);
    let xy_speed = speed_for(length.min(1.0)) * max_speed.min(limits.max_velocity);
    let z_speed = speed_for(z.abs()) * max_speed.min(limits.max_z_velocity);
    let (x, y) = if xy_speed > 0.0 { (x / length * xy_speed, y / length * xy_speed) } else { (0.0, 0.0) };
    [x, y, z.signum() * z_speed]
}

fn speed_for(deflection: f64) -> f64 {
    ((deflection - DRIVE_DEAD_ZONE) / (1.0 - DRIVE_DEAD_ZONE)).clamp(0.0, 1.0).powi(2)
}

fn planned(motion: &Motion, targets: Vec<AxisTarget>, speed: f64, limits: &MoveLimits) -> PlannedMove {
    let mut delta = [0.0; 3];
    for target in &targets {
        delta[target.axis.index()] = target.gcode - motion.gcode_position[target.axis.index()];
    }
    let seconds = Profile::new(delta, speed, limits).seconds(norm(delta));
    PlannedMove { targets, speed, seconds }
}

/// How Klipper drives a single move from rest to rest: klippy/toolhead.py `Move` and `LookAheadQueue.flush` with
/// nothing queued before or after it, `mcr_pseudo_accel` from `minimum_cruise_ratio`; the Z limits scaled by the
/// share of Z in the move (klippy/kinematics/corexy.py `check_move`).
#[derive(Debug, Clone, Copy)]
struct Profile {
    /// mm/s
    speed: f64,
    /// mm/s²
    accel: f64,
    /// Limits the peak speed of short moves (mm/s²).
    cruise_accel: f64,
}

impl Profile {
    fn new(direction: [f64; 3], speed: f64, limits: &MoveLimits) -> Self {
        let mut speed = speed.min(limits.max_velocity);
        let mut accel = limits.max_accel;
        if direction[2] != 0.0 {
            let z_ratio = norm(direction) / direction[2].abs();
            speed = speed.min(limits.max_z_velocity * z_ratio);
            accel = accel.min(limits.max_z_accel * z_ratio);
        }
        let cruise_accel = (limits.max_accel * (1.0 - limits.minimum_cruise_ratio)).min(accel);
        Self { speed, accel, cruise_accel }
    }

    /// Klipper reported its acceleration; before that a move counts at constant speed.
    fn accelerates(&self) -> bool {
        self.accel > 0.0 && self.cruise_accel > 0.0
    }

    fn seconds(&self, length: f64) -> f64 {
        if length <= 0.0 || self.speed <= 0.0 {
            return 0.0;
        }
        if !self.accelerates() {
            return length / self.speed;
        }
        let peak = self.speed.min((length * self.cruise_accel).sqrt());
        length / peak + peak / self.accel
    }

    /// The length of a move that takes `seconds`, the inverse of [`Self::seconds`].
    fn length(&self, seconds: f64) -> f64 {
        if !self.accelerates() {
            return self.speed * seconds;
        }
        let at_speed = (seconds - self.speed / self.accel) * self.speed;
        if at_speed * self.cruise_accel >= self.speed * self.speed {
            return at_speed;
        }
        // Too short to reach the speed: the peak is sqrt(length · cruise_accel).
        let root = self.cruise_accel.sqrt();
        (seconds / (1.0 / root + root / self.accel)).powi(2)
    }
}

fn norm(vector: [f64; 3]) -> f64 {
    vector.iter().map(|value| value * value).sum::<f64>().sqrt()
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

/// Toolhead minus G-code position: G-code offset, skew correction, tool offset, bed mesh and `z_thermal_adjust` at
/// this point.
fn offset(motion: &Motion, axis: Axis) -> f64 {
    motion.position[axis.index()] - motion.gcode_position[axis.index()]
}

/// Toolhead minus the coordinate a person types: none for X and Y, the G-code offsets for Z.
fn entry_offset(motion: &Motion, axis: Axis) -> f64 {
    if axis == Axis::Z { offset(motion, axis) } else { 0.0 }
}

fn move_speed(axis: Axis) -> f64 {
    if axis == Axis::Z { MOVE_SPEED_Z } else { MOVE_SPEED_XY }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Skew correction pulls X 0.4 below its G-code position here; Z offset −0.05 and 0.12 of bed mesh lift the
    /// toolhead 0.07 above it.
    fn motion() -> Motion {
        Motion {
            position: [99.6, 50.0, 5.07, 0.0],
            gcode_position: [100.0, 50.0, 5.0, 0.0],
            live_position: [99.6, 50.0, 5.07, 0.0],
            axis_minimum: [0.0, 0.0, -2.0, 0.0],
            axis_maximum: [300.0, 300.0, 250.0, 0.0],
            ..Motion::default()
        }
    }

    /// Without acceleration: every move counts at constant speed.
    const STEADY: MoveLimits = MoveLimits {
        max_velocity: 300.0,
        max_z_velocity: 20.0,
        max_accel: 0.0,
        max_z_accel: 0.0,
        minimum_cruise_ratio: 0.0,
    };
    const LIMITS: DriveLimits = DriveLimits { moves: STEADY, z_floor: 0.0 };
    /// Franks Voron: `[printer]` max_accel 5000, max_z_accel 500, max_z_velocity 40; minimum_cruise_ratio 0.5.
    const VORON: MoveLimits = MoveLimits {
        max_velocity: 400.0,
        max_z_velocity: 40.0,
        max_accel: 5000.0,
        max_z_accel: 500.0,
        minimum_cruise_ratio: 0.5,
    };

    fn single(planned: Option<PlannedMove>) -> AxisTarget {
        let planned = planned.expect("a move");
        assert_eq!(planned.targets.len(), 1);
        planned.targets[0]
    }

    #[test]
    fn a_step_moves_by_its_distance_whatever_the_offsets() {
        let target = single(jog(&motion(), Axis::Z, 0.1, &STEADY));
        assert!((target.gcode - 5.1).abs() < 1e-9);
        assert!((target.toolhead - 5.17).abs() < 1e-9);
    }

    #[test]
    fn a_step_stops_at_the_axis_limit_in_toolhead_coordinates() {
        let mut motion = motion();
        motion.position[0] = 299.5;
        motion.gcode_position[0] = 298.0;
        let target = single(jog(&motion, Axis::X, 10.0, &STEADY));
        assert_eq!((target.toolhead, target.gcode), (300.0, 298.5));
        motion.position[0] = 300.0;
        assert_eq!(jog(&motion, Axis::X, 1.0, &STEADY), None);
        // Beyond the limit already (mesh), a step outwards must not pull it back.
        motion.position[0] = 300.2;
        assert_eq!(jog(&motion, Axis::X, 1.0, &STEADY), None);
    }

    #[test]
    fn x_and_y_are_typed_where_the_toolhead_stands_and_z_as_the_gcode_height() {
        let motion = motion();
        assert_eq!(shown_position(&motion), [99.6, 50.0, 5.0]);
        assert_eq!((entry_position(&motion, Axis::X), entry_position(&motion, Axis::Z)), (99.6, 5.0));
        // X 0 is the configured minimum, whatever the skew correction does to the G-code position.
        assert_eq!(entry_range(&motion, Axis::X), Some((0.0, 300.0)));
        let target = single(Some(move_to(&motion, Axis::X, 0.0, &STEADY)));
        assert!(target.toolhead.abs() < 1e-9 && (target.gcode - 0.4).abs() < 1e-9);
        assert!(outside_range(&motion, Axis::X, -0.1) && !outside_range(&motion, Axis::X, 0.0));

        let target = single(Some(move_to(&motion, Axis::Z, 10.0, &STEADY)));
        assert_eq!(target.gcode, 10.0);
        assert!((target.toolhead - 10.07).abs() < 1e-9);
        assert!(outside_range(&motion, Axis::Z, 249.95));
        assert!(!outside_range(&motion, Axis::Z, 249.9));
        let (minimum, maximum) = entry_range(&motion, Axis::Z).unwrap();
        assert!((minimum + 2.07).abs() < 1e-9 && (maximum - 249.93).abs() < 1e-9);
        assert_eq!(entry_range(&Motion::default(), Axis::X), None);
    }

    #[test]
    fn a_move_takes_as_long_as_klipper_plans_it_from_rest_to_rest() {
        // 50 mm at 100 mm/s: 0.5 s plus 0.02 s for reaching the speed and stopping.
        let profile = Profile::new([1.0, 0.0, 0.0], 100.0, &VORON);
        assert!((profile.seconds(50.0) - 0.52).abs() < 1e-9);
        // 1 mm cannot reach 100 mm/s: the peak is sqrt(1 · 2500) = 50 mm/s.
        assert!((profile.seconds(1.0) - 0.03).abs() < 1e-9);
        for seconds in [0.03, 0.1, 0.15, 0.52] {
            assert!((profile.seconds(profile.length(seconds)) - seconds).abs() < 1e-9);
        }
        // Z alone: max_z_accel 500 and max_z_velocity 40.
        let z = Profile::new([0.0, 0.0, -1.0], 100.0, &VORON);
        assert_eq!((z.speed, z.accel, z.cruise_accel), (40.0, 500.0, 500.0));
    }

    #[test]
    fn the_pad_fills_the_queue_up_to_its_lead() {
        // From rest: Klipper starts after its delay, the segment fills the rest of the lead.
        let full = drive(&motion(), [1.0, 0.0, 0.0], 50.0, &LIMITS).unwrap();
        let first = DRIVE_LEAD - KLIPPER_START_DELAY;
        assert!((full.speed - 50.0).abs() < 1e-9 && (full.seconds - first).abs() < 1e-9);
        assert!((full.targets[0].gcode - (100.0 + 50.0 * first)).abs() < 1e-9);
        // Running: a segment per tick.
        let mut running = motion();
        running.queued_seconds = DRIVE_LEAD - DRIVE_INTERVAL.as_secs_f64();
        let next = drive(&running, [1.0, 0.0, 0.0], 50.0, &LIMITS).unwrap();
        assert!((next.seconds - DRIVE_INTERVAL.as_secs_f64()).abs() < 1e-9);
        running.queued_seconds = DRIVE_LEAD;
        assert_eq!(drive(&running, [1.0, 0.0, 0.0], 50.0, &LIMITS), None);
        // With acceleration the segment is shorter, so it still ends in time.
        let limits = DriveLimits { moves: VORON, z_floor: 0.0 };
        let braked = drive(&motion(), [1.0, 0.0, 0.0], 50.0, &limits).unwrap();
        assert!((braked.seconds - first).abs() < 1e-9 && braked.targets[0].gcode < full.targets[0].gcode);
    }

    #[test]
    fn the_pad_speed_grows_with_the_square_of_the_deflection() {
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
