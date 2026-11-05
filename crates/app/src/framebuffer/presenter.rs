//! Brings the rendered picture to the framebuffer in a thread of its own.
//!
//! The event loop renders into a picture in memory and notes the changed rectangle. The thread takes the rectangle,
//! converts it to the pixel format of the device and writes it. A slow driver (fbtft over SPI) may hold that write
//! until it has sent the previous lines; the event loop keeps reading input meanwhile, and changes made while the
//! thread waits are joined into one rectangle.

use super::device::{FrameBuffer, PixelFormat, Rect};
use slint::platform::software_renderer::SoftwareRenderer;
use slint::{PhysicalSize, Rgb8Pixel};
use std::sync::{Arc, Condvar, Mutex, MutexGuard, PoisonError};
use std::time::Instant;

pub struct Presenter {
    shared: Arc<Shared>,
    width: usize,
    height: usize,
}

struct Shared {
    frame: Mutex<Frame>,
    changed: Condvar,
}

struct Frame {
    pixels: Vec<Rgb8Pixel>,
    /// Changed since the thread last took the picture.
    dirty: Option<Rect>,
}

impl Shared {
    fn frame(&self) -> MutexGuard<'_, Frame> {
        self.frame.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

impl Presenter {
    pub fn start(device: FrameBuffer) -> std::io::Result<Self> {
        let (width, height) = device.size();
        let frame = Frame { pixels: vec![Rgb8Pixel::default(); width * height], dirty: None };
        let shared = Arc::new(Shared { frame: Mutex::new(frame), changed: Condvar::new() });
        let thread_shared = shared.clone();
        std::thread::Builder::new().name("framebuffer".to_owned()).spawn(move || present(&thread_shared, device))?;
        Ok(Self { shared, width, height })
    }

    pub fn size(&self) -> PhysicalSize {
        PhysicalSize::new(self.width as u32, self.height as u32)
    }

    /// Renders what changed into the picture and hands the changed rectangle to the thread.
    pub fn render(&self, renderer: &SoftwareRenderer) {
        let mut frame = self.shared.frame();
        let region = renderer.render(&mut frame.pixels, self.width);
        let (origin, size) = (region.bounding_box_origin(), region.bounding_box_size());
        if size.width == 0 || size.height == 0 {
            return;
        }
        let changed = Rect {
            x: origin.x.max(0) as usize,
            y: origin.y.max(0) as usize,
            width: size.width as usize,
            height: size.height as usize,
        };
        frame.dirty = Some(frame.dirty.map_or(changed, |dirty| dirty.union(changed)));
        drop(frame);
        self.shared.changed.notify_one();
    }
}

/// The thread: waits for a change, converts it while holding the picture briefly, then writes it to the device.
fn present(shared: &Shared, mut device: FrameBuffer) {
    let format = device.format();
    let (width, _) = device.size();
    let mut encoded = Vec::new();
    loop {
        let rect = {
            let mut frame = shared
                .changed
                .wait_while(shared.frame(), |frame| frame.dirty.is_none())
                .unwrap_or_else(PoisonError::into_inner);
            let Some(rect) = frame.dirty.take() else { continue };
            encode(&frame.pixels, width, rect, format, &mut encoded);
            rect
        };
        let started = Instant::now();
        device.write(rect, &encoded);
        log::debug!(
            "framebuffer: {}x{} at {},{} written in {} ms",
            rect.width,
            rect.height,
            rect.x,
            rect.y,
            started.elapsed().as_millis()
        );
    }
}

/// The rows of `rect` from a picture `width` pixels wide, one after the other in the device's format.
fn encode(pixels: &[Rgb8Pixel], width: usize, rect: Rect, format: PixelFormat, out: &mut Vec<u8>) {
    out.clear();
    for row in rect.y..rect.y + rect.height {
        let start = row * width + rect.x;
        format.encode(&pixels[start..start + rect.width], out);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::framebuffer::device::Channel;

    #[test]
    fn encode_takes_only_the_rows_and_columns_of_the_rectangle() {
        let channel = |offset| Channel { offset, length: 8 };
        let format = PixelFormat::new(24, channel(16), channel(8), channel(0)).unwrap();
        let pixels: Vec<Rgb8Pixel> = (0..12u8).map(|value| Rgb8Pixel { r: value, g: 0, b: 0 }).collect();
        let mut out = Vec::new();
        // A picture of 4x3 pixels; the rectangle is the middle two pixels of the last two rows.
        encode(&pixels, 4, Rect { x: 1, y: 1, width: 2, height: 2 }, format, &mut out);
        let reds: Vec<u8> = out.chunks(3).map(|pixel| pixel[2]).collect();
        assert_eq!(reds, [5, 6, 9, 10]);
    }
}
