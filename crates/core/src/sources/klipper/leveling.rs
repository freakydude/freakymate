//! Levelling the gantry or bed: `[quad_gantry_level]` or `[z_tilt]`, and whether it has been applied since the last
//! homing.

use crate::sources::fields::flag;
use crate::sources::objects::ObjectStore;

/// How the gantry or bed is levelled.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Leveling {
    /// `[quad_gantry_level]`, `QUAD_GANTRY_LEVEL`; wins if both are configured.
    QuadGantry,
    /// `[z_tilt]`, `Z_TILT_ADJUST`.
    ZTilt,
}

/// The objects that report `applied`, in the order [`Leveling`] prefers them.
const OBJECTS: [&str; 2] = ["quad_gantry_level", "z_tilt"];

impl Leveling {
    /// From `printer.objects.list`: quad gantry level wins over Z tilt.
    pub(super) fn detect(objects: &[&str]) -> Option<Self> {
        if objects.contains(&OBJECTS[0]) {
            Some(Self::QuadGantry)
        } else if objects.contains(&OBJECTS[1]) {
            Some(Self::ZTilt)
        } else {
            None
        }
    }

    /// The objects that report whether levelling has been applied.
    pub(super) fn objects() -> [&'static str; 2] {
        OBJECTS
    }
}

/// `Some(applied)` if quad gantry level or Z tilt is present.
pub(super) fn read_applied(store: &ObjectStore) -> Option<bool> {
    OBJECTS.iter().find_map(|name| store.object(name)).map(|leveling| flag(leveling, "applied"))
}
