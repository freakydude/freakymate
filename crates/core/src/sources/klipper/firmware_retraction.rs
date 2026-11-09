//! Firmware retraction (`[firmware_retraction]`), as `G10`/`G11` use it.

use crate::sources::fields::number;
use serde_json::Value;

#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Retraction {
    pub retract_length: f64,
    pub retract_speed: f64,
    pub unretract_extra_length: f64,
    pub unretract_speed: f64,
}

pub(super) fn read_retraction(value: &Value) -> Retraction {
    Retraction {
        retract_length: number(value, "retract_length"),
        retract_speed: number(value, "retract_speed"),
        unretract_extra_length: number(value, "unretract_extra_length"),
        unretract_speed: number(value, "unretract_speed"),
    }
}
