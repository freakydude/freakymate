//! No mouse pointer in the kiosk (`--fullscreen`, as the services start it on the touch screen).
//!
//! Hiding the pointer through the window does not last: Slint makes it visible again whenever the pointer shape
//! changes, and a tap is a pointer event to Slint. Instead the kiosk gets a cursor theme whose pointers are
//! transparent. winit loads its cursors from `XCURSOR_PATH`/`XCURSOR_THEME` (theme `default` if unset), and cage
//! looks for its own pointer in the theme `default` on `XCURSOR_PATH` as well, so whatever shape is asked for, nothing
//! shows. On the desktop and in a window the system's pointer stays.

use std::path::{Path, PathBuf};

/// Name under which the theme is found; cage asks for no other.
const THEME: &str = "default";

/// Every name winit may ask for (`cursor-icon`: the CSS name and the X11 names it falls back to), and those cage uses.
#[rustfmt::skip]
const CURSOR_NAMES: &[&str] = &[
    "default", "left_ptr", "arrow", "top_left_arrow", "left_arrow", "context-menu", "help", "question_arrow",
    "whats_this", "pointer", "hand2", "hand1", "hand", "pointing_hand", "progress", "left_ptr_watch", "half-busy",
    "wait", "watch", "cell", "plus", "crosshair", "cross", "text", "xterm", "ibeam", "vertical-text", "alias", "link",
    "copy", "move", "no-drop", "circle", "not-allowed", "crossed_circle", "forbidden", "grab", "openhand", "fleur",
    "grabbing", "closedhand", "e-resize", "right_side", "n-resize", "top_side", "ne-resize", "top_right_corner",
    "nw-resize", "top_left_corner", "s-resize", "bottom_side", "se-resize", "bottom_right_corner", "sw-resize",
    "bottom_left_corner", "w-resize", "left_side", "ew-resize", "h_double_arrow", "size_hor", "ns-resize",
    "v_double_arrow", "size_ver", "nesw-resize", "fd_double_arrow", "size_bdiag", "nwse-resize", "bd_double_arrow",
    "size_fdiag", "col-resize", "split_h", "sb_h_double_arrow", "row-resize", "split_v", "sb_v_double_arrow",
    "all-scroll", "size_all", "zoom-in", "zoom-out", "dnd-ask", "all-resize",
];

/// Writes the transparent theme and points this process to it. Call it before the window and any thread exist,
/// because it changes the environment. A failure only leaves the pointer as it was.
pub fn hide() {
    let directory = std::env::var_os("XDG_RUNTIME_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(std::env::temp_dir)
        .join("sailhand-cursors");
    match write_theme(&directory) {
        Ok(()) => {
            // SAFETY: the caller runs this before any thread or window exists (see the function docs), so
            // nothing reads or writes the environment concurrently.
            unsafe {
                std::env::set_var("XCURSOR_PATH", &directory);
                std::env::set_var("XCURSOR_THEME", THEME);
            }
        }
        Err(error) => log::warn!("cannot hide the pointer, {}: {error}", directory.display()),
    }
}

fn write_theme(directory: &Path) -> std::io::Result<()> {
    let theme = directory.join(THEME);
    let cursors = theme.join("cursors");
    std::fs::create_dir_all(&cursors)?;
    std::fs::write(theme.join("index.theme"), "[Icon Theme]\nName=Sailhand kiosk, no pointer\n")?;
    let cursor = transparent_cursor();
    for name in CURSOR_NAMES {
        std::fs::write(cursors.join(name), &cursor)?;
    }
    Ok(())
}

/// An Xcursor file with one transparent pixel at the usual nominal size of 24.
fn transparent_cursor() -> Vec<u8> {
    const IMAGE: u32 = 0xfffd_0002;
    const SIZE: u32 = 24;
    const FILE_HEADER: u32 = 16;
    const TABLE_ENTRY: u32 = 12;
    let fields = [
        // File header: magic, its own length, version, one entry in the table of contents.
        u32::from_le_bytes(*b"Xcur"),
        FILE_HEADER,
        0x0001_0000,
        1,
        // Table of contents: an image of the nominal size, right after the table.
        IMAGE,
        SIZE,
        FILE_HEADER + TABLE_ENTRY,
        // Image: header length, type, size, version, 1x1 pixel, hotspot 0/0, no animation, the pixel (ARGB 0).
        36,
        IMAGE,
        SIZE,
        1,
        1,
        1,
        0,
        0,
        0,
        0,
    ];
    fields.iter().flat_map(|field| field.to_le_bytes()).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_cursor_is_one_transparent_pixel_in_xcursor_format() {
        let images = xcursor::parser::parse_xcursor(&transparent_cursor()).expect("valid Xcursor file");

        assert_eq!(images.len(), 1);
        assert_eq!((images[0].size, images[0].width, images[0].height), (24, 1, 1));
        assert_eq!(images[0].pixels_rgba, [0, 0, 0, 0]);
    }

    #[test]
    fn the_theme_holds_every_name_as_its_own_file() {
        let directory = tempfile::tempdir().unwrap();

        write_theme(directory.path()).unwrap();

        let cursors = directory.path().join(THEME).join("cursors");
        assert!(CURSOR_NAMES.iter().all(|name| cursors.join(name).is_file()));
    }
}
