//! The map of the active bed mesh. A small mesh is drawn as cells with their values; a dense one as one pixel per
//! probed point, which the GPU scales up without smoothing, so every point stays a sharp cell. Both are drawn by the
//! GPU. Blue is lower, red higher, on a scale symmetric around 0. Points Klipper only copied (corners of a round
//! bed) stay empty, faulty regions are framed.

use crate::{BedMeshMap, MeshCellItem, MeshFrameItem};
use freakymate_core::sources::klipper::commands::MeshNameProblem;
use freakymate_core::sources::klipper::{BedMesh, MeshPoint, MeshSpan};
use slint::{Color, Image, Rgba8Pixel, SharedPixelBuffer};

/// Up to this many columns and rows the mesh is drawn as cells with values; the software renderer cannot scale a
/// picture without smoothing, so this also keeps usual meshes sharp without a GPU.
const CELLS_MAX: usize = 10;
// Colours of theme.slint: the neutral cell (surface-active), low (info), high (danger), and the two text colours.
const NEUTRAL: [u8; 3] = [0x3b, 0x42, 0x4b];
const LOW: [u8; 3] = [0x4d, 0xab, 0xf7];
const HIGH: [u8; 3] = [0xff, 0x6b, 0x6b];
const LIGHT_TEXT: [u8; 3] = [0xf1, 0xf3, 0xf5];
const DARK_TEXT: [u8; 3] = [0x12, 0x14, 0x17];
/// Share of the full colour at the ends of the scale, so the strongest cells do not glare.
const STRENGTH: f64 = 0.9;

pub struct MeshPicture {
    pub map: BedMeshMap,
    pub cells: Vec<MeshCellItem>,
    pub frames: Vec<MeshFrameItem>,
}

pub fn picture(mesh: &BedMesh) -> MeshPicture {
    let (columns, rows) = (mesh.columns(), mesh.rows.len());
    let limit = mesh.lowest().unwrap_or_default().abs().max(mesh.highest().unwrap_or_default().abs());
    let as_cells = columns <= CELLS_MAX && rows <= CELLS_MAX;
    let map = BedMeshMap {
        shown: true,
        image: if as_cells { Image::default() } else { pixels(mesh, limit) },
        columns: columns as i32,
        rows: rows as i32,
        aspect: ((mesh.max[0] - mesh.min[0]) / (mesh.max[1] - mesh.min[1])) as f32,
        range: mesh.range().map(|range| format!("{range:.3} mm")).unwrap_or_default().into(),
        lowest: mesh.lowest().map(|lowest| signed(lowest, 3)).unwrap_or_default().into(),
        highest: mesh.highest().map(|highest| signed(highest, 3)).unwrap_or_default().into(),
        limit: format!("{limit:.3}").into(),
        averaged: mesh.averaged() as i32,
        filled: mesh.rows.iter().flatten().any(|point| *point == MeshPoint::Filled),
    };
    MeshPicture {
        map,
        cells: if as_cells { cells(mesh, limit) } else { Vec::new() },
        frames: mesh.faulty.iter().map(|span| frame(span, rows)).collect(),
    }
}

/// Row 0 of the matrix is the front; the map shows the back at the top.
fn top_down(mesh: &BedMesh) -> impl Iterator<Item = (usize, &Vec<MeshPoint>)> {
    mesh.rows.iter().rev().enumerate()
}

fn cells(mesh: &BedMesh, limit: f64) -> Vec<MeshCellItem> {
    top_down(mesh)
        .flat_map(|(row, points)| {
            points.iter().enumerate().filter_map(move |(column, point)| {
                let height = point.height()?;
                let [red, green, blue] = cell_colour(height, limit);
                Some(MeshCellItem {
                    column: column as i32,
                    row: row as i32,
                    text: signed(height, 2).into(),
                    colour: Color::from_rgb_u8(red, green, blue),
                    dark: wants_dark_text([red, green, blue]),
                })
            })
        })
        .collect()
}

fn pixels(mesh: &BedMesh, limit: f64) -> Image {
    let columns = mesh.columns();
    let mut pixels = SharedPixelBuffer::<Rgba8Pixel>::new(columns as u32, mesh.rows.len() as u32);
    for ((_, row), pixel_row) in top_down(mesh).zip(pixels.make_mut_slice().chunks_mut(columns)) {
        for (point, pixel) in row.iter().zip(pixel_row) {
            *pixel = match point.height() {
                Some(height) => {
                    let [r, g, b] = cell_colour(height, limit);
                    Rgba8Pixel { r, g, b, a: 255 }
                }
                None => Rgba8Pixel { r: 0, g: 0, b: 0, a: 0 },
            };
        }
    }
    Image::from_rgba8(pixels)
}

/// Why Klipper would not save a profile under a name; `Empty` says nothing while the field is still empty.
pub fn mesh_name_problem_text(problem: MeshNameProblem) -> &'static str {
    match problem {
        MeshNameProblem::Empty => "",
        MeshNameProblem::Whitespace => "No spaces in the name",
        MeshNameProblem::Reserved => "\"default\" is kept for Measure",
    }
}

fn frame(span: &MeshSpan, rows: usize) -> MeshFrameItem {
    MeshFrameItem {
        column: span.column as i32,
        row: (rows - span.row - span.rows) as i32,
        columns: span.columns as i32,
        rows: span.rows as i32,
    }
}

/// The neutral cell colour blended towards blue or red by the share of `limit`, the largest deviation from 0.
fn cell_colour(height: f64, limit: f64) -> [u8; 3] {
    let share = if limit > 0.0 { (height.abs() / limit).min(1.0) * STRENGTH } else { 0.0 };
    let target = if height < 0.0 { LOW } else { HIGH };
    std::array::from_fn(|channel| {
        let (from, to) = (f64::from(NEUTRAL[channel]), f64::from(target[channel]));
        (from + (to - from) * share).round() as u8
    })
}

/// Dark text where it contrasts more with the cell than light text (WCAG contrast ratio).
fn wants_dark_text(cell: [u8; 3]) -> bool {
    let cell = luminance(cell);
    let contrast = |text: f64| (cell.max(text) + 0.05) / (cell.min(text) + 0.05);
    contrast(luminance(DARK_TEXT)) > contrast(luminance(LIGHT_TEXT))
}

fn luminance(colour: [u8; 3]) -> f64 {
    let linear = |channel: u8| {
        let value = f64::from(channel) / 255.0;
        if value <= 0.040_45 { value / 12.92 } else { ((value + 0.055) / 1.055).powf(2.4) }
    };
    0.2126 * linear(colour[0]) + 0.7152 * linear(colour[1]) + 0.0722 * linear(colour[2])
}

/// `+0.05`, `−0.12` with a real minus sign, `0.00`.
fn signed(value: f64, decimals: usize) -> String {
    let text = format!("{:.*}", decimals, value.abs());
    if text.bytes().all(|byte| byte == b'0' || byte == b'.') {
        text
    } else if value < 0.0 {
        format!("−{text}")
    } else {
        format!("+{text}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mesh(rows: Vec<Vec<MeshPoint>>) -> BedMesh {
        BedMesh { min: [0.0, 0.0], max: [200.0, 100.0], rows, faulty: Vec::new() }
    }

    #[test]
    fn colours_run_from_blue_over_neutral_to_red() {
        assert_eq!(cell_colour(0.0, 0.2), NEUTRAL);
        assert_eq!(cell_colour(-0.2, 0.2), [0x4b, 0xa1, 0xe6]);
        assert_eq!(cell_colour(0.4, 0.2), cell_colour(0.2, 0.2));
        assert_eq!(cell_colour(0.1, 0.0), NEUTRAL);
    }

    #[test]
    fn text_stays_readable_on_strong_cells() {
        assert!(!wants_dark_text(NEUTRAL));
        assert!(wants_dark_text(cell_colour(-0.2, 0.2)));
    }

    #[test]
    fn signs_are_written_out() {
        assert_eq!(signed(0.051, 2), "+0.05");
        assert_eq!(signed(-0.12, 2), "−0.12");
        assert_eq!(signed(-0.001, 2), "0.00");
    }

    #[test]
    fn the_back_is_at_the_top_and_copies_stay_empty() {
        let front = vec![MeshPoint::Filled, MeshPoint::Measured(-0.1)];
        let back = vec![MeshPoint::Measured(0.1), MeshPoint::Averaged(0.0)];
        let picture = picture(&mesh(vec![front, back]));
        assert_eq!(picture.map.aspect, 2.0);
        assert_eq!(picture.map.range.as_str(), "0.200 mm");
        let texts: Vec<(i32, i32, &str)> =
            picture.cells.iter().map(|cell| (cell.column, cell.row, cell.text.as_str())).collect();
        assert_eq!(texts, [(0, 0, "+0.10"), (1, 0, "0.00"), (1, 1, "−0.10")]);
        assert!(picture.map.filled);
        assert_eq!(picture.map.averaged, 1);
    }

    #[test]
    fn dense_meshes_are_a_picture() {
        let picture = picture(&mesh(vec![vec![MeshPoint::Measured(0.0); 50]; 50]));
        assert!(picture.cells.is_empty());
        assert_eq!((picture.map.columns, picture.map.rows), (50, 50));
        assert_eq!((picture.map.image.size().width, picture.map.image.size().height), (50, 50));
    }

    #[test]
    fn frames_count_rows_from_the_top() {
        let span = MeshSpan { column: 2, row: 3, columns: 1, rows: 1 };
        let frame = frame(&span, 5);
        assert_eq!((frame.column, frame.row), (2, 1));
    }
}
