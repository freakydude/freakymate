//! What only Android needs: the activity's window flags (Android's status bar, keeping the screen on), fitting the
//! layout into the part of the screen the system leaves free, and a fresh connection after the background.

use crate::AppWindow;
use crate::display;
use slint::android::android_activity::{AndroidApp, MainEvent, PollEvent, WindowManagerFlags};
use slint::{ComponentHandle, PhysicalSize, Timer, TimerMode};
use std::cell::Cell;
use std::sync::OnceLock;
use std::time::{Duration, Instant};

/// Slint reports no size change on Android, so the size is looked at twice a second (rotation, split screen, system
/// bars appearing).
const SIZE_POLL: Duration = Duration::from_millis(500);
/// A connection that lived through a shorter background is kept; Moonraker's keepalive notices a dead one within
/// 15 s anyway (`moonraker::connection`).
const RECONNECT_AFTER_BACKGROUND: Duration = Duration::from_secs(15);

pub(crate) mod log_export;

type ReturnAction = Box<dyn Fn() + Send + Sync>;

static ACTIVITY: OnceLock<AndroidApp> = OnceLock::new();
static ON_RETURN: OnceLock<ReturnAction> = OnceLock::new();

/// Keeps the activity for its window flags; called by `android_main` before Slint takes it.
pub fn init(app: AndroidApp) {
    // Set only once per process; a second call would mean a second activity, which the manifest does not allow.
    let _ = ACTIVITY.set(app);
}

/// Listens to the activity's life cycle (for `slint::android::init_with_event_listener`).
pub fn event_listener() -> impl Fn(&PollEvent<'_>) + 'static {
    let paused_at = Cell::new(None::<Instant>);
    move |event| match event {
        PollEvent::Main(MainEvent::Pause) => paused_at.set(Some(Instant::now())),
        PollEvent::Main(MainEvent::Resume { .. }) => {
            let away = paused_at.take().map(|paused| paused.elapsed());
            if away.is_some_and(|away| away >= RECONNECT_AFTER_BACKGROUND)
                && let Some(on_return) = ON_RETURN.get()
            {
                on_return();
            }
        }
        _ => {}
    }
}

/// What to do when the app comes back after a long background (connect again); set once.
pub(crate) fn on_return_from_background(action: impl Fn() + Send + Sync + 'static) {
    let _ = ON_RETURN.set(Box::new(action));
}

/// Hides Android's status bar (clock, battery); the navigation bar and a camera cutout stay and are left free (safe
/// area). The window flag needs no Java code and works on every supported Android version.
pub(crate) fn hide_status_bar() {
    set_window_flag(WindowManagerFlags::FULLSCREEN, true);
}

/// Keeps Android's keyboard away: FreakyMate brings its own, as on the touch screen at the printer. With the flag the
/// window does not interact with the input method (`WindowManager.LayoutParams.FLAG_ALT_FOCUSABLE_IM` on a focusable
/// window); a hardware keyboard still types.
pub(crate) fn hide_system_keyboard() {
    set_window_flag(WindowManagerFlags::ALT_FOCUSABLE_IM, true);
}

pub(crate) fn keep_screen_on(on: bool) {
    set_window_flag(WindowManagerFlags::KEEP_SCREEN_ON, on);
}

fn set_window_flag(flag: WindowManagerFlags, on: bool) {
    let Some(activity) = ACTIVITY.get() else {
        log::warn!("no activity for the window flag {flag:?}");
        return;
    };
    if on {
        activity.set_window_flags(flag, WindowManagerFlags::empty());
    } else {
        activity.set_window_flags(WindowManagerFlags::empty(), flag);
    }
}

/// Fits the layout whenever the window size or the edges covered by the system change.
pub(crate) fn watch_window_size(window: &AppWindow) {
    let weak = window.as_weak();
    let last = Cell::new((PhysicalSize::default(), PhysicalSize::default()));
    let timer = Timer::default();
    timer.start(TimerMode::Repeated, SIZE_POLL, move || {
        let Some(window) = weak.upgrade() else { return };
        let current = (window.window().size(), covered(&window));
        if current != last.replace(current) {
            fit(&window);
        }
    });
    // The timer lives as long as the application.
    std::mem::forget(timer);
}

/// The edges the system draws over, in physical pixels.
fn covered(window: &AppWindow) -> PhysicalSize {
    let scale = window.window().scale_factor();
    let pixels = |length: f32| (length * scale).round() as u32;
    PhysicalSize::new(pixels(window.get_covered_width()), pixels(window.get_covered_height()))
}

/// An Android app always fills the screen: the design size is fitted into the part the system leaves free.
fn fit(window: &AppWindow) {
    if display::apply_fit_inside(window.window(), covered(window)) {
        refresh_safe_area(window.window());
    }
    let size = window.window().size();
    crate::logging::header(
        "Display",
        &format!("Android {}x{}, scale factor {:.3}", size.width, size.height, window.window().scale_factor()),
    );
}

/// Slint converts the safe area to logical pixels only when the system reports a change, not when the scale changes
/// from here; switching its fullscreen on and off converts it again at once (`update_window_properties` and `resize`
/// in i-slint-backend-android-activity 1.19). Its fullscreen does not hide the system bars, it only drops the safe
/// area, so it stays off.
fn refresh_safe_area(window: &slint::Window) {
    window.set_fullscreen(true);
    window.set_fullscreen(false);
}
