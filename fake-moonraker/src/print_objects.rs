//! The objects of the simulated prints, as OrcaSlicer defines them with `EXCLUDE_OBJECT_DEFINE NAME= CENTER= POLYGON=`
//! (Klipper upper-cases the names). The fake does not read them from the file: each demo file says how much its
//! slicer defined, the objects themselves are the same.

use serde_json::{Value, json};

/// How much the slicer wrote into a file.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Definitions {
    /// Label objects switched off or not supported: no `EXCLUDE_OBJECT_DEFINE`, `objects` stays empty.
    None,
    /// Only `NAME=`, without `CENTER` and `POLYGON`.
    NamesOnly,
    /// `NAME`, `CENTER` and `POLYGON`, as OrcaSlicer writes them.
    Located,
}

struct PrintObject {
    name: &'static str,
    center: [f64; 2],
    polygon: &'static [[f64; 2]],
}

/// Two boats side by side, a cube in front and a small spacer that is only a few pixels on the map.
const PRINT_OBJECTS: [PrintObject; 4] = [
    PrintObject {
        name: "BENCHY_0",
        center: [100.0, 160.0],
        polygon: &[
            [70.0, 145.0],
            [112.0, 145.0],
            [124.0, 150.0],
            [130.0, 160.0],
            [124.0, 170.0],
            [112.0, 175.0],
            [70.0, 175.0],
        ],
    },
    PrintObject {
        name: "BENCHY_1",
        center: [200.0, 160.0],
        polygon: &[
            [170.0, 145.0],
            [212.0, 145.0],
            [224.0, 150.0],
            [230.0, 160.0],
            [224.0, 170.0],
            [212.0, 175.0],
            [170.0, 175.0],
        ],
    },
    PrintObject {
        name: "CALIBRATION_CUBE",
        center: [150.0, 80.0],
        polygon: &[[140.0, 70.0], [160.0, 70.0], [160.0, 90.0], [140.0, 90.0]],
    },
    PrintObject {
        name: "SPACER_M3",
        center: [230.0, 80.0],
        polygon: &[[228.0, 78.0], [232.0, 78.0], [232.0, 82.0], [228.0, 82.0]],
    },
];

pub fn count() -> usize {
    PRINT_OBJECTS.len()
}

/// The `objects` field of `exclude_object`.
pub fn objects(definitions: Definitions) -> Value {
    let entry = |object: &PrintObject| match definitions {
        Definitions::None => None,
        Definitions::NamesOnly => Some(json!({ "name": object.name })),
        Definitions::Located => {
            Some(json!({ "name": object.name, "center": object.center, "polygon": object.polygon }))
        }
    };
    PRINT_OBJECTS.iter().filter_map(entry).collect()
}

/// The object Klipper prints during a layer (`EXCLUDE_OBJECT_START`, written with the definitions): those not
/// excluded take turns from layer to layer.
pub fn current(definitions: Definitions, layer: u32, excluded: &[String]) -> Option<&'static str> {
    if definitions == Definitions::None {
        return None;
    }
    let remaining: Vec<&str> = PRINT_OBJECTS
        .iter()
        .map(|object| object.name)
        .filter(|name| !excluded.iter().any(|excluded| excluded == name))
        .collect();
    (!remaining.is_empty()).then(|| remaining[layer as usize % remaining.len()])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_remaining_objects_take_turns() {
        assert_eq!(current(Definitions::Located, 1, &[]), Some("BENCHY_1"));
        assert_eq!(current(Definitions::NamesOnly, 1, &["BENCHY_1".to_owned()]), Some("CALIBRATION_CUBE"));
        let all: Vec<String> = PRINT_OBJECTS.iter().map(|object| object.name.to_owned()).collect();
        assert_eq!(current(Definitions::Located, 1, &all), None);
        assert_eq!(current(Definitions::None, 1, &[]), None, "nothing defined, nothing started");
    }

    #[test]
    fn the_definitions_decide_what_objects_tell() {
        let located = objects(Definitions::Located);
        assert_eq!(located.as_array().map(Vec::len), Some(count()));
        assert_eq!(located[2]["polygon"][1], json!([160.0, 70.0]));
        assert_eq!(located[2]["center"], json!([150.0, 80.0]));
        assert_eq!(objects(Definitions::NamesOnly)[2], json!({ "name": "CALIBRATION_CUBE" }));
        assert_eq!(objects(Definitions::None), json!([]));
    }
}
