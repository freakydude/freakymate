//! The framebuffer device `/dev/fbN`: its size and pixel format, mapped into memory.
//!
//! Drivers with deferred I/O such as fbtft send only the lines whose memory pages were written, so every write here
//! covers only the rectangle that changed.

use memmap2::{MmapMut, MmapOptions};
use slint::Rgb8Pixel;
use std::fs::OpenOptions;
use std::os::fd::AsRawFd;

/// A rectangle in physical pixels.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Rect {
    pub x: usize,
    pub y: usize,
    pub width: usize,
    pub height: usize,
}

impl Rect {
    /// The smallest rectangle that holds both.
    pub fn union(self, other: Rect) -> Rect {
        let x = self.x.min(other.x);
        let y = self.y.min(other.y);
        let right = (self.x + self.width).max(other.x + other.width);
        let bottom = (self.y + self.height).max(other.y + other.height);
        Rect { x, y, width: right - x, height: bottom - y }
    }
}

/// Where a colour channel lies in a pixel of the framebuffer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Channel {
    pub offset: u32,
    pub length: u32,
}

impl Channel {
    fn place(self, value: u8) -> u32 {
        u32::from(value >> (8 - self.length)) << self.offset
    }
}

/// The layout of a pixel in the framebuffer, as the driver reports it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PixelFormat {
    pub bytes: usize,
    pub red: Channel,
    pub green: Channel,
    pub blue: Channel,
}

impl PixelFormat {
    /// Checks a layout of 16, 24 or 32 bits with channels of up to 8 bits inside the pixel.
    pub fn new(bits: u32, red: Channel, green: Channel, blue: Channel) -> Result<Self, String> {
        let fits = |channel: Channel| (1..=8).contains(&channel.length) && channel.offset + channel.length <= bits;
        if !matches!(bits, 16 | 24 | 32) || ![red, green, blue].into_iter().all(fits) {
            return Err(format!(
                "unsupported pixel format: {bits} bits, red {}/{}, green {}/{}, blue {}/{}",
                red.offset, red.length, green.offset, green.length, blue.offset, blue.length
            ));
        }
        Ok(Self { bytes: bits as usize / 8, red, green, blue })
    }

    /// Appends `pixels` in the byte order of the framebuffer (the machine's own, as the kernel stores it).
    pub fn encode(&self, pixels: &[Rgb8Pixel], out: &mut Vec<u8>) {
        for pixel in pixels {
            let value = self.red.place(pixel.r) | self.green.place(pixel.g) | self.blue.place(pixel.b);
            out.extend_from_slice(&value.to_ne_bytes()[..self.bytes]);
        }
    }
}

/// An opened framebuffer, mapped into memory.
pub struct FrameBuffer {
    path: String,
    map: MmapMut,
    width: usize,
    height: usize,
    line_length: usize,
    format: PixelFormat,
}

impl FrameBuffer {
    /// The first of `/dev/fb0` to `/dev/fb9` that can be used.
    pub fn open_first() -> Result<Self, String> {
        let mut errors = Vec::new();
        for number in 0..10 {
            let path = format!("/dev/fb{number}");
            match Self::open(&path) {
                Ok(device) => return Ok(device),
                Err(error) => errors.push(format!("{path}: {error}")),
            }
        }
        Err(format!("no usable framebuffer: {}", errors.join("; ")))
    }

    fn open(path: &str) -> Result<Self, String> {
        let file = OpenOptions::new().read(true).write(true).open(path).map_err(|error| error.to_string())?;
        let (var, fix) = screen_info(file.as_raw_fd())?;
        let channel = |field: &FbBitfield| Channel { offset: field.offset, length: field.length };
        let format = PixelFormat::new(var.bits_per_pixel, channel(&var.red), channel(&var.green), channel(&var.blue))?;
        let (width, height, line_length) = (var.xres as usize, var.yres as usize, fix.line_length as usize);
        if line_length < width * format.bytes {
            return Err(format!("line length {line_length} is shorter than {width} pixels"));
        }
        // SAFETY: the mapping lives as long as this value; only this program writes the framebuffer while it runs.
        let map = unsafe { MmapOptions::new().len(line_length * height).map_mut(&file) }
            .map_err(|error| format!("cannot map: {error}"))?;
        Ok(Self { path: path.to_owned(), map, width, height, line_length, format })
    }

    pub fn size(&self) -> (usize, usize) {
        (self.width, self.height)
    }

    pub fn format(&self) -> PixelFormat {
        self.format
    }

    /// For the log: device, size and pixel size.
    pub fn describe(&self) -> String {
        format!("{} {}x{}, {} bits per pixel", self.path, self.width, self.height, self.format.bytes * 8)
    }

    /// Writes `rect`, given as encoded rows one after the other without gaps.
    pub fn write(&mut self, rect: Rect, encoded: &[u8]) {
        let row_bytes = rect.width * self.format.bytes;
        for (row, source) in encoded.chunks_exact(row_bytes).enumerate() {
            let start = (rect.y + row) * self.line_length + rect.x * self.format.bytes;
            self.map[start..start + row_bytes].copy_from_slice(source);
        }
    }
}

// The kernel's `struct fb_bitfield`, `fb_var_screeninfo` and `fb_fix_screeninfo` (linux/fb.h); the layout needs every
// field, the program reads only a few.
const FBIOGET_VSCREENINFO: u32 = 0x4600;
const FBIOGET_FSCREENINFO: u32 = 0x4602;

#[repr(C)]
#[derive(Default)]
#[allow(dead_code)]
struct FbBitfield {
    offset: u32,
    length: u32,
    msb_right: u32,
}

#[repr(C)]
#[derive(Default)]
#[allow(dead_code)]
struct FbVarScreeninfo {
    xres: u32,
    yres: u32,
    xres_virtual: u32,
    yres_virtual: u32,
    xoffset: u32,
    yoffset: u32,
    bits_per_pixel: u32,
    grayscale: u32,
    red: FbBitfield,
    green: FbBitfield,
    blue: FbBitfield,
    transp: FbBitfield,
    nonstd: u32,
    activate: u32,
    height: u32,
    width: u32,
    accel_flags: u32,
    /// pixclock, left/right/upper/lower margin, hsync_len, vsync_len, sync, vmode, rotate
    timings: [u32; 10],
    colorspace: u32,
    reserved: [u32; 4],
}

#[repr(C)]
#[derive(Default)]
#[allow(dead_code)]
struct FbFixScreeninfo {
    id: [u8; 16],
    smem_start: libc::c_ulong,
    smem_len: u32,
    kind: u32,
    type_aux: u32,
    visual: u32,
    xpanstep: u16,
    ypanstep: u16,
    ywrapstep: u16,
    line_length: u32,
    mmio_start: libc::c_ulong,
    mmio_len: u32,
    accel: u32,
    capabilities: u16,
    reserved: [u16; 2],
}

fn screen_info(fd: i32) -> Result<(FbVarScreeninfo, FbFixScreeninfo), String> {
    let mut var = FbVarScreeninfo::default();
    let mut fix = FbFixScreeninfo::default();
    // SAFETY: both requests fill exactly these kernel structs, which live until the calls return.
    let results = unsafe {
        [
            libc::ioctl(fd, FBIOGET_VSCREENINFO as _, &mut var as *mut FbVarScreeninfo),
            libc::ioctl(fd, FBIOGET_FSCREENINFO as _, &mut fix as *mut FbFixScreeninfo),
        ]
    };
    if results.iter().any(|&result| result < 0) {
        return Err(format!("cannot read the screen info: {}", std::io::Error::last_os_error()));
    }
    Ok((var, fix))
}

#[cfg(test)]
mod tests {
    use super::*;

    const RGB565: (Channel, Channel, Channel) =
        (Channel { offset: 11, length: 5 }, Channel { offset: 5, length: 6 }, Channel { offset: 0, length: 5 });

    #[test]
    fn rgb565_keeps_the_upper_bits_of_each_channel() {
        let (red, green, blue) = RGB565;
        let format = PixelFormat::new(16, red, green, blue).unwrap();
        let mut out = Vec::new();
        format.encode(&[Rgb8Pixel { r: 0xff, g: 0x80, b: 0x08 }], &mut out);
        assert_eq!(out, (0xf800u16 | (0x20 << 5) | 0x01).to_ne_bytes());
    }

    #[test]
    fn bgr888_puts_blue_into_the_high_byte() {
        let channel = |offset| Channel { offset, length: 8 };
        let format = PixelFormat::new(24, channel(0), channel(8), channel(16)).unwrap();
        let mut out = Vec::new();
        format.encode(&[Rgb8Pixel { r: 1, g: 2, b: 3 }], &mut out);
        assert_eq!(out, 0x0003_0201u32.to_ne_bytes()[..3]);
    }

    #[test]
    fn a_channel_outside_the_pixel_is_refused() {
        let (red, green, _) = RGB565;
        assert!(PixelFormat::new(16, red, green, Channel { offset: 12, length: 5 }).is_err());
        assert!(PixelFormat::new(8, red, green, red).is_err());
    }

    #[test]
    fn union_covers_both_rectangles() {
        let a = Rect { x: 10, y: 20, width: 5, height: 5 };
        let b = Rect { x: 0, y: 22, width: 4, height: 10 };
        assert_eq!(a.union(b), Rect { x: 0, y: 20, width: 15, height: 12 });
    }
}
