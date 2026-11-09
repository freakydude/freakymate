//! Moonraker's file manager: G-code files, their metadata and thumbnails.

use crate::moonraker::Call;
use serde::Deserialize;
use serde_json::{json, Value};

#[derive(Debug, Clone, PartialEq, Default, Deserialize)]
pub struct FileEntry {
    pub path: String,
    #[serde(default)]
    pub modified: f64,
    #[serde(default)]
    pub size: u64,
}

impl FileEntry {
    pub fn is_gcode(&self) -> bool {
        let lower = self.path.to_ascii_lowercase();
        [".gcode", ".g", ".gco"].iter().any(|extension| lower.ends_with(extension))
    }
}

/// Slicers add icon-sized previews (16x16, 32x32, 48x48) next to the real one; a shorter side than this is one of them.
const MINI_THUMBNAIL_BELOW: u32 = 100;

/// A file's preview ready to show. PNG and JPEG are decoded here, in a worker thread, so the UI only copies pixels;
/// SVG (from the fake Moonraker) stays as text for the UI toolkit.
#[derive(Debug, Clone, PartialEq)]
pub enum ThumbnailImage {
    Rgba { width: u32, height: u32, pixels: Vec<u8> },
    Svg(Vec<u8>),
}

impl ThumbnailImage {
    pub fn decode(data: &[u8]) -> Option<Self> {
        if looks_like_svg(data) {
            return Some(Self::Svg(data.to_vec()));
        }
        let rgba = image::load_from_memory(data).ok()?.into_rgba8();
        Some(Self::Rgba { width: rgba.width(), height: rgba.height(), pixels: rgba.into_raw() })
    }
}

/// Bytes at the start of a thumbnail that tell whether it is an SVG.
const SVG_SNIFF_BYTES: usize = 256;

fn looks_like_svg(data: &[u8]) -> bool {
    let start = &data[..data.len().min(SVG_SNIFF_BYTES)];
    let text = String::from_utf8_lossy(start);
    let trimmed = text.trim_start();
    trimmed.starts_with("<svg") || (trimmed.starts_with("<?xml") && text.contains("<svg"))
}

#[derive(Debug, Clone, PartialEq, Default, Deserialize)]
pub struct Thumbnail {
    #[serde(default)]
    pub width: u32,
    #[serde(default)]
    pub height: u32,
    #[serde(default)]
    pub relative_path: String,
}

#[derive(Debug, Clone, PartialEq, Default, Deserialize)]
#[serde(default)]
pub struct FileMetadata {
    pub filename: String,
    pub modified: f64,
    pub size: u64,
    pub slicer: Option<String>,
    pub estimated_time: Option<f64>,
    pub filament_total: Option<f64>,
    pub filament_weight_total: Option<f64>,
    /// Material per filament of the slicer (`filament_type`); the index is the tool.
    #[serde(rename = "filament_type", deserialize_with = "per_filament_texts")]
    pub filament_types: Vec<String>,
    /// Name of the slicer's filament profile per filament (`filament_name`), free text; the index is the tool.
    #[serde(rename = "filament_name", deserialize_with = "per_filament_texts")]
    pub filament_names: Vec<String>,
    pub layer_height: Option<f64>,
    pub object_height: Option<f64>,
    pub nozzle_diameter: Option<f64>,
    pub first_layer_extr_temp: Option<f64>,
    pub first_layer_bed_temp: Option<f64>,
    pub chamber_temp: Option<f64>,
    /// Per filament of the slicer, in grams; the index is the tool.
    pub filament_weights: Vec<f64>,
    pub filament_temps: Vec<f64>,
    /// `#RRGGBB` per filament; a transparent one is `#RRGGBB00`.
    pub filament_colors: Vec<String>,
    /// Tools the print really uses.
    pub referenced_tools: Vec<usize>,
    pub thumbnails: Vec<Thumbnail>,
}

impl FileMetadata {
    /// The thumbnail that fills the square slots best: a square one in full size before a non-square one in full
    /// size before a mini one; within each the longest short side (sharpest), then the squarest (least border).
    pub fn best_thumbnail(&self) -> Option<&Thumbnail> {
        self.thumbnails.iter().filter(|thumbnail| !thumbnail.relative_path.is_empty()).max_by_key(|thumbnail| {
            let short_side = thumbnail.width.min(thumbnail.height);
            let full_size = short_side >= MINI_THUMBNAIL_BELOW;
            let square = thumbnail.width == thumbnail.height;
            (full_size, square, short_side, std::cmp::Reverse(thumbnail.width.max(thumbnail.height)))
        })
    }

    /// HTTP path of the best thumbnail relative to the Moonraker server.
    pub fn thumbnail_path(&self) -> Option<String> {
        let thumbnail = self.best_thumbnail()?;
        let directory = self.filename.rsplit_once('/').map_or("", |(directory, _)| directory);
        let relative = if directory.is_empty() {
            thumbnail.relative_path.clone()
        } else {
            format!("{directory}/{}", thumbnail.relative_path)
        };
        Some(format!("server/files/gcodes/{}", encode_path(&relative)))
    }
}

/// Moonraker reports some lists per filament as text: a JSON list (`["PLA", "PETG"]`) for several filaments, the plain
/// value (`PLA`) for one. The slicer's own form `PLA;PETG` and a real JSON list are accepted as well.
fn per_filament_texts<'de, D: serde::Deserializer<'de>>(deserializer: D) -> Result<Vec<String>, D::Error> {
    let texts = match Value::deserialize(deserializer)? {
        Value::String(text) => split_per_filament(&text),
        Value::Array(items) => items.iter().map(|item| item.as_str().unwrap_or_default().trim().to_owned()).collect(),
        _ => Vec::new(),
    };
    Ok(texts)
}

fn split_per_filament(text: &str) -> Vec<String> {
    let text = text.trim();
    if text.starts_with('[') {
        if let Ok(list) = serde_json::from_str::<Vec<String>>(text) {
            return list.into_iter().map(|item| item.trim().to_owned()).collect();
        }
    }
    if text.is_empty() {
        return Vec::new();
    }
    text.split(';').map(|item| item.trim().trim_matches('"').to_owned()).collect()
}

/// `server.files.delete_file`: removes a G-code file; `path` is relative to the `gcodes` root, as in the file list.
pub fn delete_file(path: &str) -> Call {
    Call::with("server.files.delete_file", json!({ "path": format!("gcodes/{path}") }))
}

/// `server.files.list` for the G-code files.
pub fn list_call() -> Call {
    Call::with("server.files.list", json!({ "root": "gcodes" }))
}

/// `server.files.metadata`.
pub fn metadata_call(filename: &str) -> Call {
    Call::with("server.files.metadata", json!({ "filename": filename }))
}

pub fn parse_file_list(result: &Value) -> Vec<FileEntry> {
    let mut files: Vec<FileEntry> = serde_json::from_value::<Vec<FileEntry>>(result.clone())
        .unwrap_or_default()
        .into_iter()
        .filter(FileEntry::is_gcode)
        .collect();
    files.sort_by(|left, right| right.modified.total_cmp(&left.modified).then_with(|| left.path.cmp(&right.path)));
    files
}

pub fn parse_metadata(result: &Value) -> Option<FileMetadata> {
    serde_json::from_value(result.clone()).ok()
}

/// Encodes each path segment for the URL; slashes are kept.
fn encode_path(path: &str) -> String {
    path.split('/').map(encode_segment).collect::<Vec<_>>().join("/")
}

fn encode_segment(segment: &str) -> String {
    segment
        .bytes()
        .map(|byte| match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => (byte as char).to_string(),
            other => format!("%{other:02X}"),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn delete_names_the_file_below_the_gcodes_root() {
        let call = delete_file("my parts/cube.gcode");

        assert_eq!(call.method, "server.files.delete_file");
        assert_eq!(call.params, Some(json!({ "path": "gcodes/my parts/cube.gcode" })));
    }

    #[test]
    fn thumbnails_are_decoded_to_pixels_and_svg_stays_text() {
        let mut png = Vec::new();
        image::RgbaImage::from_pixel(2, 3, image::Rgba([255, 0, 0, 255]))
            .write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png)
            .unwrap();

        assert!(matches!(
            ThumbnailImage::decode(&png),
            Some(ThumbnailImage::Rgba { width: 2, height: 3, ref pixels }) if pixels.len() == 2 * 3 * 4
        ));
        assert!(matches!(ThumbnailImage::decode(b"  <svg xmlns='x'/>"), Some(ThumbnailImage::Svg(_))));
        assert!(matches!(ThumbnailImage::decode(b"<?xml version='1.0'?><svg/>"), Some(ThumbnailImage::Svg(_))));
        assert_eq!(ThumbnailImage::decode(b"\x89PNG broken"), None);
    }

    #[test]
    fn file_list_keeps_gcode_sorted_newest_first() {
        let files = parse_file_list(&json!([
            { "path": "old.gcode", "modified": 1.0, "size": 10 },
            { "path": "notes.txt", "modified": 5.0, "size": 1 },
            { "path": "sub/new.gcode", "modified": 3.0, "size": 20 }
        ]));

        let paths: Vec<&str> = files.iter().map(|file| file.path.as_str()).collect();
        assert_eq!(paths, ["sub/new.gcode", "old.gcode"]);
    }

    #[test]
    fn picks_the_sharpest_square_thumbnail_and_builds_encoded_path() {
        let metadata = parse_metadata(&json!({
            "filename": "my parts/cube.gcode",
            "estimated_time": 1500.0,
            "thumbnails": [
                { "width": 32, "height": 32, "relative_path": ".thumbs/cube-32x32.png" },
                { "width": 400, "height": 300, "relative_path": ".thumbs/cube-400x300.png" },
                { "width": 300, "height": 300, "relative_path": ".thumbs/cube-300x300.png" }
            ]
        }))
        .unwrap();

        assert_eq!(metadata.estimated_time, Some(1500.0));
        assert_eq!(metadata.best_thumbnail().unwrap().width, 300);
        assert_eq!(
            metadata.thumbnail_path().as_deref(),
            Some("server/files/gcodes/my%20parts/.thumbs/cube-300x300.png")
        );
    }

    #[test]
    fn a_square_thumbnail_in_full_size_wins_over_a_sharper_wide_one() {
        let metadata = parse_metadata(&json!({
            "filename": "cube.gcode",
            "thumbnails": [
                { "width": 440, "height": 240, "relative_path": ".thumbs/cube-440x240.png" },
                { "width": 200, "height": 200, "relative_path": ".thumbs/cube-200x200.png" }
            ]
        }))
        .unwrap();

        assert_eq!(metadata.best_thumbnail().unwrap().width, 200);
    }

    #[test]
    fn a_wide_thumbnail_wins_over_mini_square_ones() {
        let metadata = parse_metadata(&json!({
            "filename": "cube.gcode",
            "thumbnails": [
                { "width": 16, "height": 16, "relative_path": ".thumbs/cube-16x16.png" },
                { "width": 313, "height": 173, "relative_path": ".thumbs/cube-313x173.png" },
                { "width": 440, "height": 240, "relative_path": ".thumbs/cube-440x240.png" }
            ]
        }))
        .unwrap();

        assert_eq!(metadata.best_thumbnail().unwrap().width, 440);
    }

    #[test]
    fn metadata_without_thumbnails_has_no_path() {
        let metadata = parse_metadata(&json!({ "filename": "a.gcode" })).unwrap();

        assert_eq!(metadata.thumbnail_path(), None);
    }

    #[test]
    fn materials_per_filament_are_read_in_every_form() {
        let types = |value: Value| parse_metadata(&json!({ "filament_type": value })).unwrap().filament_types;

        // Moonraker for several filaments (OrcaSlicer 2.5, Voron 2026-11-08), for one, and the slicer's own form.
        assert_eq!(types(json!("[\"PLA\", \"PETG\"]")), ["PLA", "PETG"]);
        assert_eq!(types(json!("PLA")), ["PLA"]);
        assert_eq!(types(json!("PLA;PETG")), ["PLA", "PETG"]);
        assert_eq!(types(json!(["PLA", "PETG"])), ["PLA", "PETG"]);
        assert!(types(json!("")).is_empty());
        assert!(types(Value::Null).is_empty());
    }

    #[test]
    fn profile_names_keep_their_spaces_and_lose_the_quotes() {
        let names = |text: &str| parse_metadata(&json!({ "filament_name": text })).unwrap().filament_names;

        assert_eq!(
            names("[\"Elegoo PLA @ECC2\", \"Elegoo PETG @ECC2 - Copy\"]"),
            ["Elegoo PLA @ECC2", "Elegoo PETG @ECC2 - Copy"]
        );
        assert_eq!(names("\"Elegoo PLA @ECC2\";\"Generic ASA\""), ["Elegoo PLA @ECC2", "Generic ASA"]);
    }
}
