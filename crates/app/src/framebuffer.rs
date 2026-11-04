//! Fallback for displays without DRM/KMS, such as small displays with only a framebuffer driver (fbtft): there is no
//! device cage could draw on, so Slint draws with its software renderer straight to `/dev/fbN` (backend `linuxkms`)
//! and reads touch, pointer and keys through libinput. cage with the GPU stays the standard; the installer chooses this
//! fallback only when the system has no `/dev/dri/card*`.
//!
//! Without winit the window gets no events of a window system. The layout is fitted once it is shown (a framebuffer
//! keeps its size), and the standby sees the input through a libinput hook instead of
//! [`crate::app::watch_window_events`].

use crate::backlight_controller::BacklightController;
use crate::AppWindow;
use input::event::keyboard::{KeyState, KeyboardEventTrait};
use input::event::pointer::ButtonState;
use input::event::{KeyboardEvent, PointerEvent, TouchEvent};
use input::Event;
use slint::{ComponentHandle, Timer};
use std::cell::OnceCell;
use std::rc::Rc;
use std::time::Duration;

/// Where the input goes: the backlight exists only after the window, and the backend must be chosen before it.
pub type InputTarget = Rc<OnceCell<Rc<BacklightController>>>;

/// Chooses the backend; call it before the window exists. Fails when neither DRM nor a framebuffer can be opened.
pub fn select_backend(target: InputTarget) -> Result<(), slint::PlatformError> {
    slint::BackendSelector::new()
        .backend_name("linuxkms".to_owned())
        .renderer_name("software".to_owned())
        .with_libinput_event_hook(move |event| {
            // While the screen is dark or just lit, the input wakes it and must not reach the UI.
            let Some(backlight) = target.get() else { return false };
            input_activity(event).is_some_and(|activity| backlight.input(activity))
        })
        .select()
}

/// Fits the layout to the framebuffer once the window is shown, as `Resized` does with winit.
pub fn fit_when_shown(window: &AppWindow) {
    let weak = window.as_weak();
    Timer::single_shot(Duration::ZERO, move || {
        if let Some(window) = weak.upgrade() {
            crate::app::scale_to_mode(window.window());
        }
    });
}

/// Whether an input event is a press, a touch or a key, which counts as activity for the standby; `None` for events
/// that are no input. A release or a scroll is input, but no activity of its own (as with winit).
fn input_activity(event: &Event) -> Option<bool> {
    match event {
        Event::Pointer(PointerEvent::Button(button)) => Some(button.button_state() == ButtonState::Pressed),
        Event::Pointer(PointerEvent::Motion(_) | PointerEvent::MotionAbsolute(_)) => None,
        Event::Pointer(_) => Some(false),
        Event::Keyboard(KeyboardEvent::Key(key)) => Some(key.key_state() == KeyState::Pressed),
        Event::Touch(TouchEvent::Frame(_)) => None,
        Event::Touch(_) => Some(true),
        _ => None,
    }
}
