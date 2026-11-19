//! Rotation of picture and touch for displays whose driver turns only the picture (or neither).
//!
//! The panel is what the framebuffer is: `width` x `height` in its own orientation. The window (the layout) is what the
//! user sees upright; for 90 and 270 degrees it is the panel turned on its side. The renderer rotates the picture
//! clockwise by the angle; a touch, reported in panel coordinates, goes back the other way.

use slint::PhysicalSize;
use slint::platform::software_renderer::RenderingRotation;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Rotation {
    #[default]
    None,
    Clockwise90,
    Clockwise180,
    Clockwise270,
}

impl Rotation {
    /// From the setting `rotation` (0, 90, 180, 270); the settings reader has checked the value.
    pub fn from_degrees(degrees: u16) -> Self {
        match degrees {
            90 => Self::Clockwise90,
            180 => Self::Clockwise180,
            270 => Self::Clockwise270,
            _ => Self::None,
        }
    }

    pub fn to_renderer(self) -> RenderingRotation {
        match self {
            Self::None => RenderingRotation::NoRotation,
            Self::Clockwise90 => RenderingRotation::Rotate90,
            Self::Clockwise180 => RenderingRotation::Rotate180,
            Self::Clockwise270 => RenderingRotation::Rotate270,
        }
    }

    /// The size of the window for a panel of this size: width and height swap on its side.
    pub fn window_size(self, panel: PhysicalSize) -> PhysicalSize {
        match self {
            Self::Clockwise90 | Self::Clockwise270 => PhysicalSize::new(panel.height, panel.width),
            Self::None | Self::Clockwise180 => panel,
        }
    }

    /// The place in the window for a touch at (`x`, `y`) on a panel `width` x `height`.
    pub fn touch_to_window(self, x: f64, y: f64, width: f64, height: f64) -> (f64, f64) {
        match self {
            Self::None => (x, y),
            Self::Clockwise90 => (y, width - x),
            Self::Clockwise180 => (width - x, height - y),
            Self::Clockwise270 => (height - y, x),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // A panel of 480x320; the touch is near its top left corner.
    const PANEL: (f64, f64) = (480.0, 320.0);

    fn window_place(rotation: Rotation, x: f64, y: f64) -> (f64, f64) {
        rotation.touch_to_window(x, y, PANEL.0, PANEL.1)
    }

    #[test]
    fn no_rotation_keeps_the_place() {
        assert_eq!(window_place(Rotation::None, 10.0, 20.0), (10.0, 20.0));
    }

    #[test]
    fn a_picture_turned_clockwise_by_90_has_its_top_left_corner_at_the_panel_top_right() {
        // The picture's top left corner (window 0,0) lies at the panel's top right corner (480,0).
        assert_eq!(window_place(Rotation::Clockwise90, 480.0, 0.0), (0.0, 0.0));
        assert_eq!(window_place(Rotation::Clockwise90, 470.0, 30.0), (30.0, 10.0));
    }

    #[test]
    fn a_picture_turned_by_180_is_mirrored_in_both_axes() {
        assert_eq!(window_place(Rotation::Clockwise180, 480.0, 320.0), (0.0, 0.0));
        assert_eq!(window_place(Rotation::Clockwise180, 10.0, 20.0), (470.0, 300.0));
    }

    #[test]
    fn a_picture_turned_clockwise_by_270_has_its_top_left_corner_at_the_panel_bottom_left() {
        assert_eq!(window_place(Rotation::Clockwise270, 0.0, 320.0), (0.0, 0.0));
        assert_eq!(window_place(Rotation::Clockwise270, 10.0, 290.0), (30.0, 10.0));
    }

    #[test]
    fn every_rotation_ends_inside_the_window() {
        for (rotation, degrees) in [(Rotation::None, 0), (Rotation::Clockwise90, 90), (Rotation::Clockwise180, 180)] {
            assert_eq!(Rotation::from_degrees(degrees), rotation);
        }
        let panel = PhysicalSize::new(480, 320);
        for degrees in [0, 90, 180, 270] {
            let rotation = Rotation::from_degrees(degrees);
            let window = rotation.window_size(panel);
            for (x, y) in [(0.0, 0.0), (480.0, 0.0), (0.0, 320.0), (480.0, 320.0)] {
                let (wx, wy) = window_place(rotation, x, y);
                assert!((0.0..=f64::from(window.width)).contains(&wx), "{degrees}: x {wx}");
                assert!((0.0..=f64::from(window.height)).contains(&wy), "{degrees}: y {wy}");
            }
        }
    }

    #[test]
    fn the_window_is_on_its_side_for_90_and_270() {
        let panel = PhysicalSize::new(480, 320);
        assert_eq!(Rotation::Clockwise90.window_size(panel), PhysicalSize::new(320, 480));
        assert_eq!(Rotation::Clockwise270.window_size(panel), PhysicalSize::new(320, 480));
        assert_eq!(Rotation::Clockwise180.window_size(panel), panel);
    }
}
