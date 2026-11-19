//! The bed meshes of the fake: their grid (rectangular or round bed) and made-up heights. Shapes follow
//! `klippy/extras/bed_mesh.py` (Klipper a86759335197374d2787c87477b21e2af6fa07e1, read 2026-11-19): `_init_mesh_config` (options),
//! `ProbeManager.generate_points` (the points and their distance), `BedMeshCalibrate.probe_finalize` (the matrix) and
//! `ZMesh` (the parameters the status reports).

use serde_json::{Map, Value, json};

pub type MeshPoints = Vec<Vec<f64>>;

/// `[bed_mesh]` as the default profile has it: 5 x 5 points from 30 to 270 mm; the faulty region holds the point at
/// 150, 210 (a magnet, say).
pub const MESH_MIN: f64 = 30.0;
pub const MESH_MAX: f64 = 270.0;
pub const MESH_COUNT: usize = 5;
pub const FAULTY_REGION: ([f64; 2], [f64; 2]) = ([140.0, 200.0], [160.0, 220.0]);
/// `[bed_mesh]` of a round bed: `mesh_radius`, `mesh_origin` (the bed centre of the made-up bed) and
/// `round_probe_count` (odd, `bed_mesh.py` `_init_mesh_config`). Assumed values.
pub const ROUND_RADIUS: f64 = 140.0;
pub const ROUND_ORIGIN: [f64; 2] = [150.0, 150.0];
pub const ROUND_COUNT: usize = 9;
/// A mesh with more points than this gets ripples and noise, like the scan of an Eddy probe (assumption: a probe
/// that touches the bed measures a few dozen points, a scan thousands).
const DENSE_POINTS: usize = 100;
/// Interpolation options, assumed; Klipper's defaults are `mesh_pps` 2 and `lagrange`/0.2 for `tension`.
const MESH_PPS: u32 = 2;

/// The shape of the bed the mesh covers.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum BedShape {
    Rectangular {
        min: [f64; 2],
        max: [f64; 2],
    },
    /// `mesh_radius` and `mesh_origin`.
    Round {
        radius: f64,
        origin: [f64; 2],
    },
}

/// The points of one mesh: the shape and the number of points on each axis.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MeshGrid {
    pub shape: BedShape,
    pub x_count: usize,
    pub y_count: usize,
}

impl MeshGrid {
    /// The configuration of `[bed_mesh]`.
    pub fn configured(round: bool) -> Self {
        if round {
            let radius = (ROUND_RADIUS * 10.0).floor() / 10.0;
            Self { shape: BedShape::Round { radius, origin: ROUND_ORIGIN }, x_count: ROUND_COUNT, y_count: ROUND_COUNT }
        } else {
            let shape = BedShape::Rectangular { min: [MESH_MIN; 2], max: [MESH_MAX; 2] };
            Self { shape, x_count: MESH_COUNT, y_count: MESH_COUNT }
        }
    }

    /// `BED_MESH_CALIBRATE PROBE_COUNT=<x>,<y>` (rectangular beds) or `ROUND_PROBE_COUNT=<n>` (round beds); Klipper
    /// reads only the one that fits the bed (`update_config`) and accepts at least 3.
    pub fn with_counts(self, probe_count: Option<(usize, usize)>, round_probe_count: Option<usize>) -> Self {
        match self.shape {
            BedShape::Rectangular { .. } => {
                let (x_count, y_count) = probe_count.unwrap_or((self.x_count, self.y_count));
                Self { x_count: x_count.max(3), y_count: y_count.max(3), ..self }
            }
            BedShape::Round { .. } => {
                let count = round_probe_count.unwrap_or(self.x_count).max(3);
                Self { x_count: count, y_count: count, ..self }
            }
        }
    }

    /// Distance of the points, floored to a hundredth (`generate_points`).
    fn distances(&self) -> [f64; 2] {
        let floor = |distance: f64| (distance * 100.0).floor() / 100.0;
        match self.shape {
            BedShape::Rectangular { min, max } => [
                floor((max[0] - min[0]) / (self.x_count - 1) as f64),
                floor((max[1] - min[1]) / (self.y_count - 1) as f64),
            ],
            BedShape::Round { radius, .. } => {
                let distance = floor(2.0 * radius / (self.x_count - 1) as f64);
                [distance, distance]
            }
        }
    }

    /// The position of the point in column `column` of row `row`; round beds: where it would be on the square.
    fn position(&self, column: usize, row: usize) -> [f64; 2] {
        let [x_distance, y_distance] = self.distances();
        match self.shape {
            BedShape::Rectangular { min, .. } => {
                [min[0] + column as f64 * x_distance, min[1] + row as f64 * y_distance]
            }
            BedShape::Round { origin, .. } => {
                let reach = (self.x_count / 2) as f64 * x_distance;
                [origin[0] - reach + column as f64 * x_distance, origin[1] - reach + row as f64 * y_distance]
            }
        }
    }

    /// Whether Klipper probes the point (all of a rectangular bed, inside the circle of a round one).
    fn probed(&self, column: usize, row: usize) -> bool {
        match self.shape {
            BedShape::Rectangular { .. } => true,
            BedShape::Round { radius, origin } => {
                let [x, y] = self.position(column, row);
                (x - origin[0]).hypot(y - origin[1]) <= radius
            }
        }
    }

    /// The corners the status reports as `mesh_min` and `mesh_max`: those of the probed points
    /// (`probe_finalize` stores the extent of the base points as `min_x` .. `max_y`).
    pub fn extent(&self) -> ([f64; 2], [f64; 2]) {
        let (last_column, last_row) = (self.x_count - 1, self.y_count - 1);
        let hundredth = |[x, y]: [f64; 2]| [(x * 100.0).round() / 100.0, (y * 100.0).round() / 100.0];
        (hundredth(self.position(0, 0)), hundredth(self.position(last_column, last_row)))
    }

    /// Heights of the made-up bed: higher in the middle, lower at the corners, tilted from left to right by `tilt`
    /// mm; a dense mesh adds slow ripples and noise. Round beds repeat the outermost value of a row outside the
    /// circle (`probe_finalize`: "extrapolate probed values to create a square mesh").
    pub fn heights(&self, tilt: f64) -> MeshPoints {
        let dense = self.x_count * self.y_count > DENSE_POINTS;
        let middle = (MESH_MAX + MESH_MIN) / 2.0;
        let half = middle - MESH_MIN;
        (0..self.y_count)
            .map(|row| {
                let measured: Vec<Option<f64>> = (0..self.x_count)
                    .map(|column| {
                        self.probed(column, row).then(|| {
                            let [x, y] = self.position(column, row);
                            let distance = (x - middle).hypot(y - middle) / half;
                            let mut height = 0.06 - 0.09 * distance * distance + tilt * (x - middle) / half;
                            if dense {
                                height += ripple(x, y) + noise(column, row);
                            }
                            (height * 1e6).round() / 1e6
                        })
                    })
                    .collect();
                pad_row(&measured)
            })
            .collect()
    }

    /// The `mesh_params` of the status and of a profile (`ZMesh.mesh_params`).
    pub fn params(&self) -> Value {
        let (min, max) = self.extent();
        json!({
            "min_x": min[0], "max_x": max[0], "min_y": min[1], "max_y": max[1],
            "x_count": self.x_count, "y_count": self.y_count, "mesh_x_pps": MESH_PPS, "mesh_y_pps": MESH_PPS,
            "algo": "lagrange", "tension": 0.2,
        })
    }

    /// The `[bed_mesh]` section of the configuration, as `configfile.settings` shows it.
    pub fn config_section(&self) -> Value {
        match self.shape {
            BedShape::Rectangular { min, max } => json!({
                "probe_count": [self.x_count, self.y_count],
                "mesh_min": min,
                "mesh_max": max,
                "faulty_region_1_min": FAULTY_REGION.0,
                "faulty_region_1_max": FAULTY_REGION.1,
            }),
            // Faulty regions are not handled on round beds (`bed_mesh.py` `_gen_faulty_path` comment), so none here.
            BedShape::Round { radius, origin } => json!({
                "mesh_radius": radius,
                "mesh_origin": origin,
                "round_probe_count": self.x_count,
            }),
        }
    }

    /// The options Klipper stores for a mesh profile.
    pub fn profile_section(&self, points: &MeshPoints) -> Map<String, Value> {
        let rows: Vec<String> = points
            .iter()
            .map(|row| row.iter().map(|height| format!("{height:.6}")).collect::<Vec<_>>().join(", "))
            .collect();
        let mut section = Map::new();
        section.insert("version".to_owned(), json!("1"));
        section.insert("points".to_owned(), json!(format!("\n{}", rows.join("\n"))));
        if let Value::Object(params) = self.params() {
            for (option, value) in params {
                section.insert(option, json!(value.to_string()));
            }
        }
        section
    }
}

/// Fills the unprobed values at both ends of a row with its outermost probed value.
fn pad_row(measured: &[Option<f64>]) -> Vec<f64> {
    let first = measured.iter().flatten().next().copied().unwrap_or_default();
    let last = measured.iter().flatten().next_back().copied().unwrap_or_default();
    let leading = measured.iter().take_while(|value| value.is_none()).count();
    measured
        .iter()
        .enumerate()
        .map(|(index, value)| value.unwrap_or(if index < leading { first } else { last }))
        .collect()
}

/// A slow wave over the bed, up to 0.05 mm (made up).
fn ripple(x: f64, y: f64) -> f64 {
    0.05 * (x / 37.0).sin() * (y / 29.0).cos()
}

/// Repeatable noise of up to 0.01 mm per point (made up).
fn noise(column: usize, row: usize) -> f64 {
    let hash = (column as u64).wrapping_mul(73_856_093) ^ (row as u64).wrapping_mul(19_349_663);
    ((hash % 2001) as f64 / 1000.0 - 1.0) * 0.01
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_configured_mesh_is_five_by_five_over_the_bed() {
        let grid = MeshGrid::configured(false);

        assert_eq!(grid.extent(), ([30.0, 30.0], [270.0, 270.0]));
        assert_eq!(grid.heights(0.0).len(), 5);
    }

    #[test]
    fn a_dense_mesh_stays_within_a_third_of_a_millimetre() {
        let grid = MeshGrid::configured(false).with_counts(Some((30, 40)), None);

        let points = grid.heights(0.03);

        assert_eq!((points.len(), points[0].len()), (40, 30));
        assert!(points.iter().flatten().all(|height| height.abs() <= 0.3));
        let (min, max) = grid.extent();
        assert!(min[0] == 30.0 && max[0] < 270.0 && max[0] > 269.0);
    }

    #[test]
    fn a_round_mesh_repeats_the_outermost_value_outside_the_circle() {
        let grid = MeshGrid::configured(true).with_counts(Some((30, 40)), Some(31));

        let points = grid.heights(0.0);

        assert_eq!((points.len(), points[0].len()), (31, 31));
        // The corner of the square lies outside the circle: it copies the first value of its row.
        assert_eq!(points[0][0], points[0][1]);
        assert_eq!(points[0][0], points[0][14]);
        assert!(points[15][0] != points[15][1]);
        let (min, max) = grid.extent();
        assert!((min[0] - 10.05).abs() < 1e-9 && (max[0] - 289.95).abs() < 1e-9);
    }
}
