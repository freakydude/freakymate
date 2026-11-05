//! Touches, pointer buttons and keys through libinput, for the framebuffer fallback.
//!
//! A touch becomes a pointer press, move and release at the same place, as Slint does for touch screens. Only the first
//! finger counts. A pointer device with absolute positions works the same way; a mouse moving relatively has no cursor
//! here and only its buttons act at the last absolute position. Keys only wake the screen.
//!
//! The standby sees every press first: while the screen is dark or just lit, the input wakes it and does not reach
//! the UI. With the verbose log every touch is logged with its place and with how late the event loop read it.

use super::InputTarget;
use input::event::keyboard::{KeyState, KeyboardEventTrait};
use input::event::pointer::ButtonState;
use input::event::touch::{TouchEventPosition, TouchEventSlot, TouchEventTrait};
use input::event::{KeyboardEvent, PointerEvent, TouchEvent};
use input::{Event, Libinput, LibinputInterface};
use slint::platform::{PointerEventButton, WindowEvent};
use slint::{LogicalPosition, Window};
use std::fs::{File, OpenOptions};
use std::os::fd::{AsFd, BorrowedFd, OwnedFd};
use std::os::unix::fs::OpenOptionsExt;
use std::path::Path;

/// The left mouse button (linux/input-event-codes.h).
const BTN_LEFT: u32 = 0x110;

pub struct Input {
    libinput: Libinput,
    target: InputTarget,
    /// The seat slot of the finger that acts as the pointer.
    finger: Option<u32>,
    position: LogicalPosition,
}

impl Input {
    pub fn open(target: InputTarget) -> Result<Self, String> {
        let mut libinput = Libinput::new_with_udev(Devices);
        libinput.udev_assign_seat("seat0").map_err(|()| "libinput cannot use seat0".to_owned())?;
        Ok(Self { libinput, target, finger: None, position: LogicalPosition::default() })
    }

    /// Reads the waiting events and hands them to `window`.
    pub fn dispatch(&mut self, window: &Window) {
        if let Err(error) = self.libinput.dispatch() {
            log::warn!("libinput: {error}");
            return;
        }
        while let Some(event) = self.libinput.next() {
            let swallowed = self.wakes_the_screen(&event);
            log_touch(&event, window, swallowed);
            if swallowed {
                continue;
            }
            for window_event in self.translate(&event, window) {
                if let Err(error) = window.dispatch_event_with_result(window_event) {
                    log::warn!("cannot hand the input to the window: {error}");
                }
            }
        }
    }

    /// Whether the event only wakes the screen and must not reach the UI.
    fn wakes_the_screen(&self, event: &Event) -> bool {
        let Some(backlight) = self.target.get() else { return false };
        input_activity(event).is_some_and(|activity| backlight.input(activity))
    }

    fn translate(&mut self, event: &Event, window: &Window) -> Vec<WindowEvent> {
        let size = window.size();
        let scale = window.scale_factor();
        let place = |x: f64, y: f64| LogicalPosition::new(x as f32 / scale, y as f32 / scale);
        let left = PointerEventButton::Left;
        match event {
            Event::Touch(TouchEvent::Down(down)) if self.finger.is_none() => {
                self.finger = Some(down.seat_slot());
                self.position = place(down.x_transformed(size.width), down.y_transformed(size.height));
                vec![WindowEvent::PointerPressed { position: self.position, button: left }]
            }
            Event::Touch(TouchEvent::Motion(motion)) if self.finger == Some(motion.seat_slot()) => {
                self.position = place(motion.x_transformed(size.width), motion.y_transformed(size.height));
                vec![WindowEvent::PointerMoved { position: self.position }]
            }
            Event::Touch(TouchEvent::Up(up)) if self.finger == Some(up.seat_slot()) => self.lift(),
            Event::Touch(TouchEvent::Cancel(cancel)) if self.finger == Some(cancel.seat_slot()) => self.lift(),
            Event::Pointer(PointerEvent::MotionAbsolute(motion)) => {
                self.position =
                    place(motion.absolute_x_transformed(size.width), motion.absolute_y_transformed(size.height));
                vec![WindowEvent::PointerMoved { position: self.position }]
            }
            Event::Pointer(PointerEvent::Button(button)) if button.button() == BTN_LEFT => {
                let position = self.position;
                match button.button_state() {
                    ButtonState::Pressed => vec![WindowEvent::PointerPressed { position, button: left }],
                    ButtonState::Released => vec![WindowEvent::PointerReleased { position, button: left }],
                }
            }
            _ => Vec::new(),
        }
    }

    /// The finger leaves: a release where it was, then the pointer leaves the window (no hover stays behind).
    fn lift(&mut self) -> Vec<WindowEvent> {
        self.finger = None;
        vec![
            WindowEvent::PointerReleased { position: self.position, button: PointerEventButton::Left },
            WindowEvent::PointerExited,
        ]
    }
}

impl AsFd for Input {
    fn as_fd(&self) -> BorrowedFd<'_> {
        self.libinput.as_fd()
    }
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

/// Verbose log of a touch going down or up: where (in layout pixels, as in the .slint files) and how long after the
/// driver the event loop read it.
fn log_touch(event: &Event, window: &Window, swallowed: bool) {
    if !log::log_enabled!(log::Level::Debug) {
        return;
    }
    let note = if swallowed { ", only wakes the screen" } else { "" };
    match event {
        Event::Touch(TouchEvent::Down(down)) => {
            let (size, scale) = (window.size(), f64::from(window.scale_factor()));
            log::debug!(
                "touch down at {:.0},{:.0}, read {} ms late{note}",
                down.x_transformed(size.width) / scale,
                down.y_transformed(size.height) / scale,
                lateness_ms(down.time_usec()),
            );
        }
        Event::Touch(TouchEvent::Up(up)) => {
            log::debug!("touch up, read {} ms late{note}", lateness_ms(up.time_usec()));
        }
        _ => {}
    }
}

/// How long ago the kernel stamped the event (CLOCK_MONOTONIC, as libinput uses).
// `time_t` and `c_long` have 32 bits on armv7, so the conversions are needed there.
#[allow(clippy::useless_conversion)]
fn lateness_ms(event_usec: u64) -> i64 {
    let mut now = libc::timespec { tv_sec: 0, tv_nsec: 0 };
    // SAFETY: `now` is a valid timespec for the call.
    unsafe { libc::clock_gettime(libc::CLOCK_MONOTONIC, &mut now) };
    let now_usec = i64::from(now.tv_sec) * 1_000_000 + i64::from(now.tv_nsec) / 1_000;
    (now_usec - event_usec as i64) / 1_000
}

/// Opens the input devices directly, as the user of the service (member of the group `input`), without a seat daemon.
struct Devices;

impl LibinputInterface for Devices {
    fn open_restricted(&mut self, path: &Path, flags: i32) -> Result<OwnedFd, i32> {
        let access = flags & libc::O_ACCMODE;
        OpenOptions::new()
            .custom_flags(flags)
            .read(access == libc::O_RDONLY || access == libc::O_RDWR)
            .write(access == libc::O_WRONLY || access == libc::O_RDWR)
            .open(path)
            .map(OwnedFd::from)
            .map_err(|error| error.raw_os_error().unwrap_or(libc::EIO))
    }

    fn close_restricted(&mut self, fd: OwnedFd) {
        drop(File::from(fd));
    }
}
