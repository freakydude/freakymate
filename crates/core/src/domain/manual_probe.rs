//! The paper test of Klipper's `manual_probe`: where each of the four step buttons would move the nozzle, and the
//! coarse ladder. The target shown on a button and the distance sent with it come from the same function.
//!
//! Fine is Klipper's own bisecting `TESTZ Z=+` / `Z=-` (`cmd_TESTZ` in `manual_probe.py`). Coarse is a relative
//! `TESTZ Z=<distance>`: 0.5, 1, then 2 mm while no height was tried on that side, otherwise halfway to the nearest
//! height tried, at most 2 mm and only if that is more than a fine step.

/// Largest step of Klipper's bisecting `TESTZ` (`BISECT_MAX`).
pub const FINE_MAX: f64 = 0.2;
/// Coarse steps on a side without a height tried; the last one stays.
const LADDER: [f64; 3] = [0.5, 1.0, 2.0];
/// Largest coarse step towards a height tried.
const COARSE_MAX: f64 = 2.0;
/// Absorbs rounding when a half gap is compared with the fine step.
const EPSILON: f64 = 1e-9;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Direction {
    Up,
    Down,
}

/// What `manual_probe` reports: the height now and the nearest heights tried below and above.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Heights {
    pub z: f64,
    pub lower: Option<f64>,
    pub upper: Option<f64>,
}

impl Heights {
    /// The nearest height tried on the side of `direction`.
    fn tried(&self, direction: Direction) -> Option<f64> {
        match direction {
            Direction::Up => self.upper,
            Direction::Down => self.lower,
        }
    }
}

/// Klipper reports heights as floats; a move that arrived is within this of its target, in mm.
const ARRIVED_MM: f64 = 0.0005;

/// The last coarse press: its rung, and where it started and should end, to notice moves made elsewhere.
#[derive(Debug, Clone, Copy, PartialEq)]
struct CoarsePress {
    direction: Direction,
    rung: usize,
    from: f64,
    to: f64,
}

/// The coarse ladder: which rung the next coarse press in a direction takes.
#[derive(Debug, Default, Clone, PartialEq)]
pub struct ProbeSteps {
    /// The last coarse press up the ladder, if no other move came after it.
    last_coarse: Option<CoarsePress>,
}

impl ProbeSteps {
    /// A reported height that is neither where the last coarse press started nor where it should end means someone
    /// moved elsewhere (Mainsail, console): the ladder starts over.
    pub fn observe(&mut self, z: f64) {
        let moved_elsewhere = self
            .last_coarse
            .is_some_and(|press| (z - press.from).abs() > ARRIVED_MM && (z - press.to).abs() > ARRIVED_MM);
        if moved_elsewhere {
            self.reset();
        }
    }

    /// A move other than a ladder press, or the end or start of a mode, starts the ladder over.
    pub fn reset(&mut self) {
        self.last_coarse = None;
    }

    /// Where Klipper's `TESTZ Z=+` / `Z=-` goes.
    pub fn fine_target(heights: Heights, direction: Direction) -> f64 {
        match direction {
            Direction::Up => {
                heights.upper.map_or(f64::MAX, |upper| (upper + heights.z) / 2.0).min(heights.z + FINE_MAX)
            }
            Direction::Down => {
                heights.lower.map_or(f64::MIN, |lower| (lower + heights.z) / 2.0).max(heights.z - FINE_MAX)
            }
        }
    }

    /// The signed distance of the next coarse press; `None` if it would do no more than the fine step.
    pub fn coarse_distance(&self, heights: Heights, direction: Direction) -> Option<f64> {
        let magnitude = match heights.tried(direction) {
            Some(tried) => {
                let half = (tried - heights.z).abs() / 2.0;
                if half <= FINE_MAX + EPSILON {
                    return None;
                }
                half.min(COARSE_MAX)
            }
            None => LADDER[self.next_rung(direction)],
        };
        let magnitude = (magnitude * 10_000.0).round() / 10_000.0;
        Some(match direction {
            Direction::Up => magnitude,
            Direction::Down => -magnitude,
        })
    }

    /// Where the next coarse press goes.
    pub fn coarse_target(&self, heights: Heights, direction: Direction) -> Option<f64> {
        self.coarse_distance(heights, direction).map(|distance| heights.z + distance)
    }

    /// Takes a coarse press: the distance to send, and the ladder moves up if the side has no height tried.
    pub fn press_coarse(&mut self, heights: Heights, direction: Direction) -> Option<f64> {
        let distance = self.coarse_distance(heights, direction)?;
        self.last_coarse = match heights.tried(direction) {
            None => Some(CoarsePress {
                direction,
                rung: self.next_rung(direction),
                from: heights.z,
                to: heights.z + distance,
            }),
            Some(_) => None,
        };
        Some(distance)
    }

    fn next_rung(&self, direction: Direction) -> usize {
        match self.last_coarse {
            Some(press) if press.direction == direction => (press.rung + 1).min(LADDER.len() - 1),
            _ => 0,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use Direction::{Down, Up};

    fn heights(z: f64, lower: Option<f64>, upper: Option<f64>) -> Heights {
        Heights { z, lower, upper }
    }

    fn open() -> Heights {
        heights(5.0, None, None)
    }

    #[test]
    fn coarse_climbs_the_ladder_up_and_stays_at_the_maximum() {
        let mut steps = ProbeSteps::default();
        let taken: Vec<_> = (0..5).filter_map(|_| steps.press_coarse(open(), Up)).collect();
        assert_eq!(taken, [0.5, 1.0, 2.0, 2.0, 2.0]);
    }

    #[test]
    fn coarse_climbs_the_ladder_down_and_stays_at_the_maximum() {
        let mut steps = ProbeSteps::default();
        let taken: Vec<_> = (0..4).filter_map(|_| steps.press_coarse(open(), Down)).collect();
        assert_eq!(taken, [-0.5, -1.0, -2.0, -2.0]);
    }

    #[test]
    fn the_button_shows_the_rung_the_next_press_takes() {
        let mut steps = ProbeSteps::default();
        assert_eq!(steps.coarse_target(open(), Up), Some(5.5));
        steps.press_coarse(open(), Up);
        assert_eq!(steps.coarse_target(open(), Up), Some(6.0));
        assert_eq!(steps.coarse_target(open(), Down), Some(4.5));
    }

    #[test]
    fn a_direction_change_starts_the_ladder_over() {
        let mut steps = ProbeSteps::default();
        steps.press_coarse(open(), Up);
        steps.press_coarse(open(), Up);
        assert_eq!(steps.press_coarse(open(), Down), Some(-0.5));
        assert_eq!(steps.press_coarse(open(), Up), Some(0.5));
    }

    #[test]
    fn any_other_move_starts_the_ladder_over() {
        let mut steps = ProbeSteps::default();
        steps.press_coarse(open(), Up);
        steps.reset();
        assert_eq!(steps.press_coarse(open(), Up), Some(0.5));
    }

    #[test]
    fn the_ladder_goes_on_while_the_height_is_where_the_press_started_or_ends() {
        let mut steps = ProbeSteps::default();
        let start = heights(5.0, None, None);
        assert_eq!(steps.press_coarse(start, Down), Some(-0.5));
        steps.observe(5.0);
        steps.observe(4.5);
        assert_eq!(steps.press_coarse(heights(4.5, None, Some(5.0)), Down), Some(-1.0));
    }

    #[test]
    fn a_move_made_elsewhere_starts_the_ladder_over() {
        let mut steps = ProbeSteps::default();
        steps.press_coarse(heights(5.0, None, None), Down);
        steps.observe(4.4);
        assert_eq!(steps.press_coarse(heights(4.4, None, Some(5.0)), Down), Some(-0.5));
    }

    #[test]
    fn towards_a_height_tried_coarse_goes_halfway() {
        let steps = ProbeSteps::default();
        assert_eq!(steps.coarse_distance(heights(1.0, None, Some(2.0)), Up), Some(0.5));
        assert_eq!(steps.coarse_distance(heights(1.0, Some(0.0), None), Down), Some(-0.5));
        assert_eq!(steps.coarse_target(heights(1.0, None, Some(2.0)), Up), Some(1.5));
    }

    #[test]
    fn towards_a_height_tried_coarse_is_capped_at_two_millimetres() {
        let steps = ProbeSteps::default();
        assert_eq!(steps.coarse_distance(heights(0.0, None, Some(10.0)), Up), Some(2.0));
        assert_eq!(steps.coarse_distance(heights(0.0, Some(-10.0), None), Down), Some(-2.0));
    }

    #[test]
    fn coarse_is_off_when_it_would_do_what_fine_does() {
        let steps = ProbeSteps::default();
        assert_eq!(steps.coarse_distance(heights(1.0, None, Some(1.4)), Up), None);
        assert_eq!(steps.coarse_distance(heights(1.0, None, Some(1.1)), Up), None);
        assert_eq!(steps.coarse_distance(heights(1.0, Some(0.6), None), Down), None);
        assert_eq!(steps.coarse_distance(heights(1.0, None, Some(1.41)), Up).map(|d| (d * 1e4).round()), Some(2050.0));
    }

    #[test]
    fn a_press_towards_a_height_tried_leaves_the_ladder_at_its_start() {
        let mut steps = ProbeSteps::default();
        steps.press_coarse(open(), Up);
        steps.press_coarse(heights(5.0, None, Some(9.0)), Up);
        assert_eq!(steps.press_coarse(open(), Up), Some(0.5));
    }

    #[test]
    fn fine_goes_to_the_next_height_tried_or_at_most_two_tenths() {
        assert_eq!(ProbeSteps::fine_target(heights(1.0, None, Some(1.2)), Up), 1.1);
        assert_eq!(ProbeSteps::fine_target(heights(1.0, None, Some(2.0)), Up), 1.2);
        assert_eq!(ProbeSteps::fine_target(heights(1.0, Some(0.8), None), Down), 0.9);
        assert_eq!(ProbeSteps::fine_target(heights(1.0, Some(0.0), None), Down), 0.8);
    }

    #[test]
    fn fine_without_a_height_tried_takes_the_largest_step() {
        assert_eq!(ProbeSteps::fine_target(open(), Up), 5.2);
        assert_eq!(ProbeSteps::fine_target(open(), Down), 4.8);
    }
}
