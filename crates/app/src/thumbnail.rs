//! Turn file previews into Slint images. The core has already decoded PNG and JPEG in a worker thread, so this only
//! copies pixels; SVG (from the fake Moonraker) is parsed by Slint.

use sailhand_core::sources::moonraker_server::files::ThumbnailImage;
use slint::{Image, Rgba8Pixel, SharedPixelBuffer};

pub fn to_image(thumbnail: &ThumbnailImage) -> Option<Image> {
    match thumbnail {
        ThumbnailImage::Rgba { width, height, pixels } => {
            let expected = usize::try_from(u64::from(*width) * u64::from(*height) * 4).ok()?;
            if pixels.len() != expected {
                return None;
            }
            Some(Image::from_rgba8(SharedPixelBuffer::<Rgba8Pixel>::clone_from_slice(pixels, *width, *height)))
        }
        ThumbnailImage::Svg(data) => Image::load_from_svg_data(data).ok(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pixels_become_an_image_of_the_same_size() {
        let thumbnail = ThumbnailImage::Rgba { width: 2, height: 3, pixels: vec![255; 2 * 3 * 4] };

        let image = to_image(&thumbnail).expect("pixels convert");

        assert_eq!((image.size().width, image.size().height), (2, 3));
    }

    #[test]
    fn a_wrong_pixel_count_is_no_image() {
        assert!(to_image(&ThumbnailImage::Rgba { width: 2, height: 3, pixels: vec![0; 5] }).is_none());
    }
}
