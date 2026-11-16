//! Fits the layout, which is made for 800x480, to the real display: until the pages respond to the screen size,
//! fullscreen shows the same layout scaled up or down on every other resolution. A window keeps the scale of the
//! system instead; it is never smaller than 800x480 (`AppWindow`), and a larger one shows more room.

use slint::platform::WindowEvent;
use slint::{LogicalSize, PhysicalSize, Window};

/// The size the pages are laid out for: the BTT HDMI5.
pub const DESIGN_WIDTH: f32 = 800.0;
pub const DESIGN_HEIGHT: f32 = 480.0;

/// The largest factor at which the design size still fits into the display. The window then is as large as the
/// design size on one axis and larger on the other, where the pages simply have more room. `None` without a size.
pub fn fit_scale(width: u32, height: u32) -> Option<f32> {
    (width > 0 && height > 0).then(|| (width as f32 / DESIGN_WIDTH).min(height as f32 / DESIGN_HEIGHT))
}

/// Fullscreen fits the layout to the display. A window keeps the scale of the system, unless it is smaller than the
/// layout at that scale: a kiosk compositor such as cage sizes every window to its display, also without fullscreen
/// (the setting can be switched off at runtime), and a 480x272 display would cut the pages off.
pub fn fits_to_display(fullscreen: bool, width: u32, height: u32, system_scale: f32) -> bool {
    fullscreen || (width as f32) < DESIGN_WIDTH * system_scale || (height as f32) < DESIGN_HEIGHT * system_scale
}

/// [`fit_scale`] for the part of a window that the system does not draw over (`covered`: the sum of both edges per
/// axis, e.g. Android's navigation bar and a camera cutout).
pub fn fit_scale_inside(size: PhysicalSize, covered: PhysicalSize) -> Option<f32> {
    fit_scale(size.width.saturating_sub(covered.width), size.height.saturating_sub(covered.height))
}

/// Scales the layout to the current size of the window (fullscreen). Does nothing at the design size, so the
/// primary display is not touched.
pub fn apply_fit(window: &Window) {
    apply_fit_inside(window, PhysicalSize::default());
}

/// Scales the layout into the part of the window the system leaves free (see [`fit_scale_inside`]); whether the
/// scale changed.
pub fn apply_fit_inside(window: &Window, covered: PhysicalSize) -> bool {
    fit_scale_inside(window.size(), covered).is_some_and(|scale| apply_scale(window, scale))
}

/// Back to the scale of the system (a window, also after leaving fullscreen).
pub fn apply_system_scale(window: &Window, system_scale: f32) {
    apply_scale(window, system_scale);
}

fn apply_scale(window: &Window, scale: f32) -> bool {
    let size = window.size();
    if (scale - window.scale_factor()).abs() < 0.001 {
        return false;
    }
    let logical = LogicalSize::new(size.width as f32 / scale, size.height as f32 / scale);
    for event in [WindowEvent::ScaleFactorChanged { scale_factor: scale }, WindowEvent::Resized { size: logical }] {
        if let Err(error) = window.dispatch_event_with_result(event) {
            log::warn!("cannot scale the window ({}x{}): {error}", size.width, size.height);
            return false;
        }
    }
    log::info!("window {}x{} shown with scale factor {scale:.3}", size.width, size.height);
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_design_size_needs_no_scaling() {
        assert_eq!(fit_scale(800, 480), Some(1.0));
    }

    #[test]
    fn other_displays_are_scaled_by_the_axis_that_limits() {
        // 7" 1024x600: the height limits (1.25 against 1.28).
        assert_eq!(fit_scale(1024, 600), Some(1.25));
        // 4.3" 480x272: the height limits.
        assert!((fit_scale(480, 272).unwrap() - 0.566_67).abs() < 1e-4);
        assert_eq!(fit_scale(640, 400).map(|scale| (scale * 100.0).round()), Some(80.0));
        assert_eq!(fit_scale(1280, 720), Some(1.5));
    }

    #[test]
    fn the_scaled_window_is_never_smaller_than_the_design_size() {
        for (width, height) in [(1024, 600), (480, 272), (640, 400), (1920, 1080), (800, 400), (500, 480)] {
            let scale = fit_scale(width, height).unwrap();
            assert!(width as f32 / scale >= DESIGN_WIDTH - 0.01, "{width}x{height}");
            assert!(height as f32 / scale >= DESIGN_HEIGHT - 0.01, "{width}x{height}");
        }
    }

    #[test]
    fn the_layout_fits_beside_the_edges_the_system_covers() {
        // A 20:9 phone in landscape with the navigation bar (126 px) at the side: the height limits, the free width
        // shows more room.
        let scale = fit_scale_inside(PhysicalSize::new(2400, 1080), PhysicalSize::new(126, 0)).unwrap();
        assert_eq!(scale, 2.25);
        assert!((2400.0 - 126.0) / scale > DESIGN_WIDTH);
        // A bar so large that it leaves less than the width: the width limits.
        let scale = fit_scale_inside(PhysicalSize::new(1024, 600), PhysicalSize::new(224, 0)).unwrap();
        assert_eq!(scale, 1.0);
        assert_eq!(fit_scale_inside(PhysicalSize::new(100, 480), PhysicalSize::new(200, 0)), None);
    }

    #[test]
    fn only_fullscreen_or_a_window_below_the_layout_is_fitted() {
        assert!(fits_to_display(true, 1024, 600, 1.0));
        assert!(!fits_to_display(false, 1024, 600, 1.0), "a larger window shows more room");
        assert!(!fits_to_display(false, 1600, 960, 2.0), "800x480 at the system's scale of 2");
        assert!(fits_to_display(false, 480, 272, 1.0), "cage sizes the window to a small display");
        assert!(fits_to_display(false, 1024, 600, 1.5), "smaller than the layout at the system's scale");
    }

    #[test]
    fn an_empty_window_has_no_scale() {
        assert_eq!(fit_scale(0, 480), None);
        assert_eq!(fit_scale(800, 0), None);
    }
}
