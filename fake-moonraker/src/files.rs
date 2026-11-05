//! Virtual G-code files with metadata and thumbnail.

use crate::print_objects::Definitions;
use serde_json::{json, Value};
use std::borrow::Cow;

const PLACEHOLDER_SIZE: u32 = 300;
/// Layer height of every fake file; the first layer is as high.
pub const LAYER_HEIGHT: f64 = 0.2;

/// The benchy's thumbnails as preFlight 1.4.1 rendered them in FreakyMate's accent colour, so screenshots show a real
/// print preview. #3DBenchy (3DBenchy.com) is licensed CC0 1.0.
const BENCHY_THUMBNAILS: [(u32, u32, &[u8]); 3] = [
    (32, 32, include_bytes!("../assets/benchy-32x32.png")),
    (300, 300, include_bytes!("../assets/benchy-300x300.png")),
    (400, 300, include_bytes!("../assets/benchy-400x300.png")),
];

/// A thumbnail image as Moonraker serves it from `.thumbs/`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ThumbnailImage {
    Png(&'static [u8]),
    Svg(String),
}

impl ThumbnailImage {
    pub fn content_type(&self) -> &'static str {
        match self {
            Self::Png(_) => "image/png",
            Self::Svg(_) => "image/svg+xml",
        }
    }

    pub fn into_bytes(self) -> Cow<'static, [u8]> {
        match self {
            Self::Png(bytes) => Cow::Borrowed(bytes),
            Self::Svg(text) => Cow::Owned(text.into_bytes()),
        }
    }

    fn len(&self) -> usize {
        match self {
            Self::Png(bytes) => bytes.len(),
            Self::Svg(text) => text.len(),
        }
    }
}

#[derive(Debug, Clone)]
struct Thumbnail {
    width: u32,
    height: u32,
    image: ThumbnailImage,
}

#[derive(Debug, Clone)]
pub struct GcodeFile {
    pub path: String,
    pub modified: f64,
    pub estimated_time: f64,
    /// Print duration of the last completed print; `None` if the file was never printed.
    pub last_print_duration: Option<f64>,
    pub layer_count: u32,
    pub first_layer_extruder_temperature: f64,
    pub first_layer_bed_temperature: f64,
}

impl GcodeFile {
    fn sample(path: &str, modified: f64, estimated_time: f64, layer_count: u32) -> Self {
        Self {
            path: path.to_owned(),
            modified,
            estimated_time,
            last_print_duration: None,
            layer_count,
            first_layer_extruder_temperature: 210.0,
            first_layer_bed_temperature: 60.0,
        }
    }

    fn printed(mut self, print_duration: f64) -> Self {
        self.last_print_duration = Some(print_duration);
        self
    }

    /// Weight per filament; the long job is printed in two materials.
    fn filament_weights(&self) -> Vec<f64> {
        let total = self.filament_total() * 0.003;
        if self.path == "long_job.gcode" {
            vec![total * 0.4, 55.0]
        } else {
            vec![total]
        }
    }

    fn filament_temps(&self) -> Vec<u32> {
        if self.path == "long_job.gcode" {
            vec![210, 235]
        } else {
            vec![210]
        }
    }

    fn filament_colors(&self) -> Vec<&'static str> {
        if self.path == "long_job.gcode" {
            vec!["#E6261F", "#2E7D32"]
        } else {
            vec!["#E6261F"]
        }
    }

    /// What the slicer defined for `exclude_object`: the cube was sliced with label objects off, the long job by a
    /// slicer that only names them.
    pub fn object_definitions(&self) -> Definitions {
        match self.path.as_str() {
            "calibration_cube.gcode" => Definitions::None,
            "long_job.gcode" => Definitions::NamesOnly,
            _ => Definitions::Located,
        }
    }

    pub fn filament_total(&self) -> f64 {
        self.estimated_time * 2.5
    }

    fn size(&self) -> u64 {
        (self.estimated_time * 800.0) as u64
    }

    fn stem(&self) -> &str {
        self.path.strip_suffix(".gcode").unwrap_or(&self.path)
    }

    /// The benchy has real slicer thumbnails, the cube was sliced without any (the screen shows its own placeholder),
    /// the long job has an SVG with its name.
    fn thumbnail_images(&self) -> Vec<Thumbnail> {
        match self.path.as_str() {
            "benchy.gcode" => BENCHY_THUMBNAILS
                .iter()
                .map(|&(width, height, png)| Thumbnail { width, height, image: ThumbnailImage::Png(png) })
                .collect(),
            "calibration_cube.gcode" => Vec::new(),
            _ => {
                let image = ThumbnailImage::Svg(self.placeholder_svg());
                vec![Thumbnail { width: PLACEHOLDER_SIZE, height: PLACEHOLDER_SIZE, image }]
            }
        }
    }

    fn thumbnail_path(&self, thumbnail: &Thumbnail) -> String {
        format!(".thumbs/{}-{}x{}.png", self.stem(), thumbnail.width, thumbnail.height)
    }

    /// The thumbnails as Moonraker lists them, the path under `path_key`.
    fn thumbnail_entries(&self, path_key: &str) -> Value {
        let entries = self.thumbnail_images().into_iter().map(|thumbnail| {
            json!({
                "width": thumbnail.width,
                "height": thumbnail.height,
                "size": thumbnail.image.len(),
                path_key: self.thumbnail_path(&thumbnail),
            })
        });
        Value::Array(entries.collect())
    }

    fn placeholder_svg(&self) -> String {
        format!(
            r##"<svg xmlns="http://www.w3.org/2000/svg" width="{size}" height="{size}" viewBox="0 0 {size} {size}"><rect width="{size}" height="{size}" fill="#2a2f36"/><text x="{middle}" y="{middle}" font-size="24" fill="#9aa0a6" text-anchor="middle">{label}</text></svg>"##,
            size = PLACEHOLDER_SIZE,
            middle = PLACEHOLDER_SIZE / 2,
            label = self.stem(),
        )
    }

    fn list_entry(&self) -> Value {
        json!({ "path": self.path, "modified": self.modified, "size": self.size(), "permissions": "rw" })
    }

    pub fn thumbnails(&self) -> Value {
        self.thumbnail_entries("thumbnail_path")
    }

    pub fn metadata(&self) -> Value {
        json!({
            "filename": self.path,
            "size": self.size(),
            "modified": self.modified,
            "slicer": "FakeSlicer",
            "slicer_version": "1.0.0",
            "estimated_time": self.estimated_time,
            "layer_count": self.layer_count,
            "layer_height": LAYER_HEIGHT,
            "first_layer_height": LAYER_HEIGHT,
            "object_height": f64::from(self.layer_count) * LAYER_HEIGHT,
            "nozzle_diameter": 0.4,
            "filament_name": "Fake PLA",
            "filament_type": if self.path == "long_job.gcode" { "PLA;PETG" } else { "PLA" },
            "filament_total": self.filament_total(),
            "filament_weight_total": self.filament_weights().iter().sum::<f64>(),
            "filament_weights": self.filament_weights(),
            "filament_temps": self.filament_temps(),
            "filament_colors": self.filament_colors(),
            "referenced_tools": (0..self.filament_weights().len()).collect::<Vec<_>>(),
            "first_layer_extr_temp": self.first_layer_extruder_temperature,
            "first_layer_bed_temp": self.first_layer_bed_temperature,
            "gcode_start_byte": 0,
            "gcode_end_byte": self.size(),
            "job_id": null,
            "print_start_time": null,
            "uuid": format!("fake-{}", self.stem()),
            "thumbnails": self.thumbnail_entries("relative_path"),
        })
    }
}

#[derive(Debug, Clone)]
pub struct Catalog {
    files: Vec<GcodeFile>,
}

impl Catalog {
    pub fn demo() -> Self {
        Self {
            files: vec![
                GcodeFile::sample("calibration_cube.gcode", 1_700_000_000.0, 1_500.0, 100).printed(1_380.0),
                GcodeFile::sample("benchy.gcode", 1_700_200_000.0, 7_200.0, 250).printed(7_650.0),
                GcodeFile::sample("long_job.gcode", 1_700_100_000.0, 36_000.0, 600),
            ],
        }
    }

    pub fn find(&self, path: &str) -> Option<&GcodeFile> {
        self.files.iter().find(|file| file.path == path)
    }

    /// `server.files.delete_file`: false if there is no such file.
    pub fn remove(&mut self, path: &str) -> bool {
        let before = self.files.len();
        self.files.retain(|file| file.path != path);
        self.files.len() != before
    }

    pub fn list(&self) -> Value {
        Value::Array(self.files.iter().map(GcodeFile::list_entry).collect())
    }

    /// Job history, newest first: a completed job for every printed file, and a newer cancelled one for the benchy.
    pub fn history(&self) -> Value {
        let completed = self.files.iter().filter_map(|file| {
            let duration = file.last_print_duration?;
            Some(json!({
                "job_id": format!("{:06X}", file.modified as u64),
                "filename": file.path,
                "status": "completed",
                "start_time": file.modified + 60.0,
                "end_time": file.modified + 60.0 + duration,
                "print_duration": duration,
                "total_duration": duration + 240.0,
                "filament_used": file.filament_total() * 0.97,
                "exists": true,
                "auxiliary_data": [{ "provider": "spoolman", "name": "spool_ids", "value": [1], "description": "Spool IDs used", "units": null }],
            }))
        });
        let cancelled = json!({
            "job_id": "0000FF",
            "filename": "benchy.gcode",
            "status": "cancelled",
            "start_time": 1_700_300_000.0,
            "end_time": 1_700_300_600.0,
            "print_duration": 580.0,
            "total_duration": 600.0,
            "filament_used": 1_400.0,
            "exists": true,
        });
        let jobs: Vec<Value> = std::iter::once(cancelled).chain(completed).collect();
        json!({ "count": jobs.len(), "jobs": jobs })
    }

    pub fn thumbnail(&self, thumbnail_path: &str) -> Option<ThumbnailImage> {
        self.files.iter().find_map(|file| {
            file.thumbnail_images()
                .into_iter()
                .find(|thumbnail| file.thumbnail_path(thumbnail) == thumbnail_path)
                .map(|thumbnail| thumbnail.image)
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn removed_files_are_no_longer_listed() {
        let mut catalog = Catalog::demo();

        assert!(catalog.remove("benchy.gcode"));
        assert!(!catalog.remove("benchy.gcode"));
        assert!(catalog.find("benchy.gcode").is_none());
    }

    #[test]
    fn finds_files_by_path() {
        let catalog = Catalog::demo();

        assert!(catalog.find("benchy.gcode").is_some());
        assert!(catalog.find("missing.gcode").is_none());
    }

    #[test]
    fn metadata_points_to_servable_thumbnails() {
        let catalog = Catalog::demo();
        let thumbnail_of = |file: &str, index: usize| {
            let metadata = catalog.find(file).unwrap().metadata();
            catalog.thumbnail(metadata["thumbnails"][index]["relative_path"].as_str().unwrap())
        };

        assert!(
            matches!(thumbnail_of("benchy.gcode", 1), Some(ThumbnailImage::Png(png)) if png.starts_with(b"\x89PNG"))
        );
        assert!(matches!(thumbnail_of("long_job.gcode", 0), Some(ThumbnailImage::Svg(_))));
        assert_eq!(catalog.find("calibration_cube.gcode").unwrap().metadata()["thumbnails"], json!([]));
    }

    #[test]
    fn history_lists_completed_jobs_after_the_newest_cancelled_one() {
        let history = Catalog::demo().history();

        assert_eq!(history["count"], 3);
        assert_eq!(history["jobs"][0]["status"], "cancelled");
        assert_eq!(history["jobs"][1]["status"], "completed");
    }

    #[test]
    fn list_contains_all_files_with_moonraker_fields() {
        let list = Catalog::demo().list();

        assert_eq!(list.as_array().unwrap().len(), 3);
        assert_eq!(list[0]["permissions"], "rw");
    }
}
