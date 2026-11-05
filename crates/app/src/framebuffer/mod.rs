//! Fallback for displays without DRM/KMS, such as small displays with only a framebuffer driver (fbtft): there is no
//! device cage could draw on, so the program brings a small Slint platform of its own. Slint's software renderer draws
//! into a picture in memory, a thread copies only the changed rectangle to `/dev/fbN` ([`presenter`]), and libinput
//! delivers touches, pointer buttons and keys ([`input`]). cage with the GPU stays the standard; the installer chooses
//! this fallback only when the system has no `/dev/dri/card*`.
//!
//! Why not Slint's `linuxkms` backend: it copies the whole picture to the framebuffer on every change. fbtft sends each
//! changed line over SPI (a whole 480x320 picture takes about 0.3 s at 12.5 MHz), and the next copy waits for that, so
//! the event loop read the touches late. A small rectangle, copied outside the event loop, keeps both short.
//!
//! Without winit the window gets no events of a window system. The layout is fitted once it is shown (a framebuffer
//! keeps its size), and the standby sees the input in [`input`] instead of [`crate::app::watch_window_events`].

mod device;
mod input;
mod presenter;

use crate::backlight_controller::BacklightController;
use crate::AppWindow;
use device::FrameBuffer;
use input::Input;
use presenter::Presenter;
use slint::platform::software_renderer::{MinimalSoftwareWindow, RepaintBufferType};
use slint::platform::{EventLoopProxy, Platform, WindowAdapter};
use slint::{ComponentHandle, EventLoopError, PlatformError, Timer};
use std::cell::{OnceCell, RefCell};
use std::io::{ErrorKind, Read, Write};
use std::os::fd::{AsFd, AsRawFd, BorrowedFd};
use std::os::unix::net::UnixStream;
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

/// Where the input goes: the backlight exists only after the window, and the platform must be set before it.
pub type InputTarget = Rc<OnceCell<Rc<BacklightController>>>;

/// How often an animation is drawn; the thread sends only the newest picture to a slow display anyway.
const ANIMATION_FRAME: Duration = Duration::from_millis(16);

/// Sets the platform; call it before the window exists. Fails when no framebuffer or no input can be opened.
pub fn install_platform(target: InputTarget) -> Result<(), PlatformError> {
    let device = FrameBuffer::open_first().map_err(PlatformError::Other)?;
    log::info!("framebuffer {}", device.describe());
    let presenter = Presenter::start(device).map_err(|error| PlatformError::Other(error.to_string()))?;
    let input = Input::open(target).map_err(PlatformError::Other)?;
    let platform =
        FramebufferPlatform::new(presenter, input).map_err(|error| PlatformError::Other(error.to_string()))?;
    slint::platform::set_platform(Box::new(platform)).map_err(|error| PlatformError::Other(error.to_string()))
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

struct FramebufferPlatform {
    window: Rc<MinimalSoftwareWindow>,
    presenter: Presenter,
    input: RefCell<Input>,
    wake: Arc<Wake>,
    wake_receiver: UnixStream,
}

impl FramebufferPlatform {
    fn new(presenter: Presenter, input: Input) -> std::io::Result<Self> {
        let (sender, wake_receiver) = UnixStream::pair()?;
        sender.set_nonblocking(true)?;
        wake_receiver.set_nonblocking(true)?;
        Ok(Self {
            window: MinimalSoftwareWindow::new(RepaintBufferType::ReusedBuffer),
            presenter,
            input: RefCell::new(input),
            wake: Arc::new(Wake { queue: Mutex::default(), quit: AtomicBool::new(false), sender }),
            wake_receiver,
        })
    }

    /// Sleeps until input arrives, another thread wakes the loop or `timeout` ends (`None`: no timer is due).
    fn wait(&self, timeout: Option<Duration>) -> Result<(), PlatformError> {
        let input = self.input.borrow();
        let fds = [input.as_fd(), self.wake_receiver.as_fd()];
        poll(&fds, timeout).map_err(|error| PlatformError::Other(format!("cannot wait for input: {error}")))?;
        drop(input);
        // Only the wake-up matters, not the bytes.
        let mut drained = [0u8; 64];
        while matches!((&self.wake_receiver).read(&mut drained), Ok(n) if n > 0) {}
        Ok(())
    }
}

impl Platform for FramebufferPlatform {
    fn create_window_adapter(&self) -> Result<Rc<dyn WindowAdapter>, PlatformError> {
        Ok(self.window.clone())
    }

    fn run_event_loop(&self) -> Result<(), PlatformError> {
        self.wake.quit.store(false, Ordering::Relaxed);
        self.window.set_size(self.presenter.size());
        loop {
            slint::platform::update_timers_and_animations();
            self.wake.run_queued();
            if self.wake.quit.load(Ordering::Relaxed) {
                return Ok(());
            }
            self.window.draw_if_needed(|renderer| self.presenter.render(renderer));
            let timeout = if self.window.has_active_animations() {
                Some(ANIMATION_FRAME)
            } else {
                slint::platform::duration_until_next_timer_update()
            };
            self.wait(timeout)?;
            self.input.borrow_mut().dispatch(self.window.window());
        }
    }

    fn new_event_loop_proxy(&self) -> Option<Box<dyn EventLoopProxy>> {
        Some(Box::new(WakeProxy(self.wake.clone())))
    }
}

/// What other threads hand to the event loop, and the socket that wakes it.
struct Wake {
    queue: Mutex<Vec<Box<dyn FnOnce() + Send>>>,
    quit: AtomicBool,
    sender: UnixStream,
}

impl Wake {
    fn run_queued(&self) {
        let queued = std::mem::take(&mut *self.queue.lock().unwrap_or_else(PoisonError::into_inner));
        for job in queued {
            job();
        }
    }

    fn notify(&self) {
        // A full socket already holds a wake-up.
        if let Err(error) = (&self.sender).write(&[1]) {
            if error.kind() != ErrorKind::WouldBlock {
                log::warn!("cannot wake the event loop: {error}");
            }
        }
    }
}

struct WakeProxy(Arc<Wake>);

impl EventLoopProxy for WakeProxy {
    fn quit_event_loop(&self) -> Result<(), EventLoopError> {
        self.0.quit.store(true, Ordering::Relaxed);
        self.0.notify();
        Ok(())
    }

    fn invoke_from_event_loop(&self, event: Box<dyn FnOnce() + Send>) -> Result<(), EventLoopError> {
        self.0.queue.lock().unwrap_or_else(PoisonError::into_inner).push(event);
        self.0.notify();
        Ok(())
    }
}

/// Waits until one of `fds` can be read or `timeout` ends; an interrupting signal counts as a wake-up.
fn poll(fds: &[BorrowedFd<'_>], timeout: Option<Duration>) -> std::io::Result<()> {
    let mut pollfds: Vec<libc::pollfd> =
        fds.iter().map(|fd| libc::pollfd { fd: fd.as_raw_fd(), events: libc::POLLIN, revents: 0 }).collect();
    // Rounded up: a timer due in 0.4 ms must not turn into a busy loop of zero timeouts.
    let timeout_ms =
        timeout.map_or(-1, |timeout| i32::try_from(timeout.as_micros().div_ceil(1000)).unwrap_or(i32::MAX));
    // SAFETY: `pollfds` is a valid array of `pollfds.len()` entries for the duration of the call.
    let result = unsafe { libc::poll(pollfds.as_mut_ptr(), pollfds.len() as libc::nfds_t, timeout_ms) };
    if result < 0 {
        let error = std::io::Error::last_os_error();
        if error.kind() != ErrorKind::Interrupted {
            return Err(error);
        }
    }
    Ok(())
}
