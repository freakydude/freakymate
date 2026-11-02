//! Connects the `Backlight` global with the backlight service: shows its state, keeps touches from reaching a dark
//! screen, and tells the standby what the printer is doing.

use crate::presenter::Presenter;
use crate::{AppWindow, Backlight};
use freakymate_core::backlight::{Availability, BacklightService, BacklightState, Screen};
use freakymate_core::domain::screen_wake::{console_line_wake, WakeFacts};
use freakymate_core::Snapshot;
use slint::ComponentHandle;
use std::cell::{Cell, RefCell};
use std::rc::Rc;
use tokio::sync::watch;

pub struct BacklightController {
    window: slint::Weak<AppWindow>,
    service: BacklightService,
    screen: Cell<Screen>,
    /// What the printer showed in the last snapshot; `None` before the first one.
    facts: RefCell<Option<WakeFacts>>,
    printing: Cell<bool>,
}

impl BacklightController {
    pub fn bind(window: &AppWindow, service: BacklightService, presenter: &Presenter) -> Rc<Self> {
        let controller = Rc::new(Self {
            window: window.as_weak(),
            service,
            screen: Cell::new(Screen::On),
            facts: RefCell::new(None),
            printing: Cell::new(false),
        });
        let weak = Rc::downgrade(&controller);
        presenter.on_snapshot(move |snapshot| {
            if let Some(controller) = weak.upgrade() {
                controller.observe_printer(snapshot);
            }
        });
        let weak = Rc::downgrade(&controller);
        presenter.on_console_line(move |line| {
            if let (Some(controller), Some(reason)) = (weak.upgrade(), console_line_wake(line)) {
                controller.service.wake(reason);
            }
        });
        let service = controller.service.clone();
        window.global::<Backlight>().on_sleep(move || service.sleep());
        controller
    }

    /// Follows the state of the service in the Slint event loop.
    pub fn follow(self: &Rc<Self>, mut states: watch::Receiver<BacklightState>) -> Result<(), slint::EventLoopError> {
        let controller = Rc::clone(self);
        slint::spawn_local(async move {
            loop {
                let state = states.borrow_and_update().clone();
                controller.apply_state(&state);
                if states.changed().await.is_err() {
                    break;
                }
            }
        })?;
        Ok(())
    }

    pub fn apply_state(&self, state: &BacklightState) {
        self.screen.set(state.screen);
        let Some(window) = self.window.upgrade() else { return };
        let backlight = window.global::<Backlight>();
        backlight.set_available(matches!(state.availability, Availability::Available { .. }));
        backlight.set_brightness(f32::from(state.brightness));
        backlight.set_off(state.screen == Screen::Off);
    }

    /// An input event of the window. A press, touch or key restarts the idle time or wakes the screen; returns true
    /// while the screen is not fully on, so the event must not reach the UI (the touch that wakes presses nothing).
    pub fn input(&self, activity: bool) -> bool {
        if activity {
            self.service.touch();
        }
        self.screen.get() != Screen::On
    }

    fn observe_printer(&self, snapshot: &Snapshot) {
        let facts = WakeFacts::read(snapshot.connection, &snapshot.status);
        // The first snapshot is no change: the screen is lit at the start anyway.
        if let Some(before) = self.facts.replace(Some(facts.clone())) {
            if let Some(reason) = facts.wake_after(&before) {
                self.service.wake(reason);
            }
        }
        if self.printing.replace(facts.printing()) != facts.printing() {
            self.service.set_printing(facts.printing());
        }
    }
}
