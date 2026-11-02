//! When the screen goes off and on again: a pure state machine, driven with explicit times so tests need no clock.
//!
//! The first touch on a dark screen only wakes it; for [`WAKE_LOCK`] afterwards touches still do nothing, so a
//! double tap does not press whatever lies under the finger.

use std::time::{Duration, Instant};

/// After waking, touches are ignored this long.
pub const WAKE_LOCK: Duration = Duration::from_millis(500);

/// Time without a touch until the screen goes off; `None` is never.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct StandbyTimeouts {
    pub idle: Option<Duration>,
    /// While a print runs or is paused.
    pub printing: Option<Duration>,
}

/// What the UI shows: the content, a black screen, or the content that does not take touches yet.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Screen {
    #[default]
    On,
    Off,
    /// Just woken: lit, but touches are swallowed until the wake lock ends.
    Waking,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Phase {
    On { last_activity: Instant },
    Off,
    Waking { until: Instant },
}

#[derive(Debug, Clone)]
pub struct Standby {
    timeouts: StandbyTimeouts,
    printing: bool,
    phase: Phase,
}

impl Standby {
    pub fn new(timeouts: StandbyTimeouts, now: Instant) -> Self {
        Self { timeouts, printing: false, phase: Phase::On { last_activity: now } }
    }

    pub fn screen(&self) -> Screen {
        match self.phase {
            Phase::On { .. } => Screen::On,
            Phase::Off => Screen::Off,
            Phase::Waking { .. } => Screen::Waking,
        }
    }

    /// A touch anywhere: wakes a dark screen, otherwise restarts the idle time.
    pub fn touch(&mut self, now: Instant) {
        self.wake(now);
    }

    /// Something on the printer needs attention (print ended, pause, error, shutdown): like a touch.
    pub fn wake(&mut self, now: Instant) {
        match self.phase {
            Phase::Off => self.phase = Phase::Waking { until: now + WAKE_LOCK },
            Phase::On { .. } => self.phase = Phase::On { last_activity: now },
            // The wake lock runs to its end; touches during it do not extend it.
            Phase::Waking { .. } => {}
        }
    }

    /// The "Off" button: dark at once, whatever the timeouts say.
    pub fn sleep(&mut self) {
        self.phase = Phase::Off;
    }

    /// A print started or ended; either wakes the screen, then the matching timeout applies.
    pub fn set_printing(&mut self, printing: bool, now: Instant) {
        if self.printing != printing {
            self.printing = printing;
            self.wake(now);
        }
    }

    /// New timeouts count from now.
    pub fn set_timeouts(&mut self, timeouts: StandbyTimeouts, now: Instant) {
        self.timeouts = timeouts;
        if let Phase::On { .. } = self.phase {
            self.phase = Phase::On { last_activity: now };
        }
    }

    /// Moves on once a deadline has passed: ends the wake lock or switches the screen off.
    pub fn update(&mut self, now: Instant) {
        match self.phase {
            Phase::Waking { until } if now >= until => self.phase = Phase::On { last_activity: until },
            Phase::On { .. } if self.deadline().is_some_and(|deadline| now >= deadline) => self.phase = Phase::Off,
            _ => {}
        }
    }

    /// When [`update`](Self::update) has something to do next; `None` while nothing is due.
    pub fn deadline(&self) -> Option<Instant> {
        match self.phase {
            Phase::On { last_activity } => self.timeout().map(|timeout| last_activity + timeout),
            Phase::Off => None,
            Phase::Waking { until } => Some(until),
        }
    }

    fn timeout(&self) -> Option<Duration> {
        if self.printing {
            self.timeouts.printing
        } else {
            self.timeouts.idle
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const MINUTE: Duration = Duration::from_secs(60);

    fn timeouts(idle: Option<u32>, printing: Option<u32>) -> StandbyTimeouts {
        StandbyTimeouts { idle: idle.map(|m| m * MINUTE), printing: printing.map(|m| m * MINUTE) }
    }

    /// Calls `update` at `start + offset` and returns the screen.
    fn screen_at(standby: &mut Standby, start: Instant, offset: Duration) -> Screen {
        standby.update(start + offset);
        standby.screen()
    }

    #[test]
    fn never_goes_off_by_default() {
        let start = Instant::now();
        let mut standby = Standby::new(StandbyTimeouts::default(), start);
        assert_eq!(standby.deadline(), None);
        assert_eq!(screen_at(&mut standby, start, 600 * MINUTE), Screen::On);
        standby.set_printing(true, start);
        assert_eq!(screen_at(&mut standby, start, 600 * MINUTE), Screen::On);
    }

    #[test]
    fn goes_off_after_the_idle_time_and_a_touch_restarts_it() {
        let start = Instant::now();
        let mut standby = Standby::new(timeouts(Some(5), None), start);
        assert_eq!(screen_at(&mut standby, start, 4 * MINUTE), Screen::On);
        standby.touch(start + 4 * MINUTE);
        assert_eq!(screen_at(&mut standby, start, 8 * MINUTE), Screen::On);
        assert_eq!(standby.deadline(), Some(start + 9 * MINUTE));
        assert_eq!(screen_at(&mut standby, start, 9 * MINUTE), Screen::Off);
        assert_eq!(standby.deadline(), None);
    }

    #[test]
    fn first_touch_only_wakes_and_the_lock_swallows_more_touches() {
        let start = Instant::now();
        let mut standby = Standby::new(timeouts(Some(1), None), start);
        standby.sleep();
        standby.touch(start);
        assert_eq!(standby.screen(), Screen::Waking);
        standby.touch(start + WAKE_LOCK / 2);
        assert_eq!(standby.deadline(), Some(start + WAKE_LOCK), "touches in the lock do not extend it");
        assert_eq!(screen_at(&mut standby, start, WAKE_LOCK / 2), Screen::Waking);
        assert_eq!(screen_at(&mut standby, start, WAKE_LOCK), Screen::On);
        assert_eq!(standby.deadline(), Some(start + WAKE_LOCK + MINUTE), "the idle time starts after the lock");
    }

    #[test]
    fn off_button_works_without_timeouts() {
        let start = Instant::now();
        let mut standby = Standby::new(StandbyTimeouts::default(), start);
        standby.sleep();
        assert_eq!(screen_at(&mut standby, start, 600 * MINUTE), Screen::Off);
        standby.touch(start + 600 * MINUTE);
        assert_eq!(standby.screen(), Screen::Waking);
    }

    #[test]
    fn printing_has_its_own_timeout() {
        let start = Instant::now();
        let mut standby = Standby::new(timeouts(Some(2), Some(10)), start);
        standby.set_printing(true, start);
        assert_eq!(screen_at(&mut standby, start, 5 * MINUTE), Screen::On);
        assert_eq!(screen_at(&mut standby, start, 10 * MINUTE), Screen::Off);
    }

    #[test]
    fn print_start_and_end_wake_the_screen() {
        let start = Instant::now();
        let mut standby = Standby::new(timeouts(Some(1), None), start);
        assert_eq!(screen_at(&mut standby, start, MINUTE), Screen::Off);
        standby.set_printing(true, start + 2 * MINUTE);
        assert_eq!(standby.screen(), Screen::Waking);
        assert_eq!(screen_at(&mut standby, start, 2 * MINUTE + WAKE_LOCK), Screen::On);
        // Printing has no timeout here: stays on.
        assert_eq!(standby.deadline(), None);
        standby.sleep();
        standby.set_printing(false, start + 90 * MINUTE);
        assert_eq!(standby.screen(), Screen::Waking);
    }

    #[test]
    fn unchanged_print_state_is_no_wake() {
        let start = Instant::now();
        let mut standby = Standby::new(timeouts(Some(1), None), start);
        standby.sleep();
        standby.set_printing(false, start);
        assert_eq!(standby.screen(), Screen::Off);
    }

    #[test]
    fn printer_event_restarts_the_idle_time_on_a_lit_screen() {
        let start = Instant::now();
        let mut standby = Standby::new(timeouts(Some(5), None), start);
        standby.wake(start + 4 * MINUTE);
        assert_eq!(standby.deadline(), Some(start + 9 * MINUTE));
    }

    #[test]
    fn new_timeouts_count_from_now() {
        let start = Instant::now();
        let mut standby = Standby::new(StandbyTimeouts::default(), start);
        standby.set_timeouts(timeouts(Some(2), None), start + 30 * MINUTE);
        assert_eq!(standby.deadline(), Some(start + 32 * MINUTE));
    }
}
