//! Spoolman, through Moonraker's `spoolman` component: the spools, the active spool and the connection state. Where
//! the spools sit in the printer is answered in [`crate::domain::spools`].

use crate::color::{parse_hex, Rgba};
use crate::moonraker::Call;
use serde_json::{json, Value};
use std::collections::HashMap;

/// A spool with less than this share of its start weight left is flagged.
const LOW_SHARE: f64 = 0.15;
/// Spoolman has up to three colours per filament.
const MAX_COLORS: usize = 3;

#[derive(Debug, Clone, PartialEq, Default)]
pub struct Spool {
    pub id: u32,
    pub vendor: String,
    pub filament_name: String,
    pub material: String,
    /// One to three colours: Spoolman's multi-colour filaments (`multi_color_hexes`) have up to three; empty if unknown.
    pub colors: Vec<Rgba>,
    /// The colours follow each other along the filament (`multi_color_direction` longitudinal); otherwise a
    /// multi-colour filament has them side by side (coaxial), which is also the common case without the field.
    pub longitudinal: bool,
    /// Grams of filament left.
    pub remaining_weight: Option<f64>,
    /// Grams of filament at the start.
    pub initial_weight: Option<f64>,
    /// Weight of the empty spool in grams.
    pub spool_weight: Option<f64>,
    /// g/cm³.
    pub density: Option<f64>,
    /// Filament diameter in mm.
    pub diameter: Option<f64>,
    /// Recommended temperatures in °C (`settings_extruder_temp`, `settings_bed_temp`), if the filament has them.
    pub extruder_temp: Option<f64>,
    pub bed_temp: Option<f64>,
    /// Where Spoolman thinks the spool is, e.g. `voron @ MMU Gate:0`.
    pub location: String,
    /// ISO time, sorts as text; empty if the spool was never used.
    pub last_used: String,
}

impl Spool {
    /// `None` without an id, so a broken entry is skipped.
    pub fn from_json(value: &Value) -> Option<Self> {
        let id = u32::try_from(value.get("id")?.as_u64()?).ok()?;
        let filament = value.get("filament").unwrap_or(&Value::Null);
        let number = |object: &Value, field: &str| object.get(field).and_then(Value::as_f64);
        let text =
            |object: &Value, field: &str| object.get(field).and_then(Value::as_str).unwrap_or_default().to_owned();
        Some(Self {
            id,
            vendor: filament.pointer("/vendor/name").and_then(Value::as_str).unwrap_or_default().to_owned(),
            filament_name: text(filament, "name"),
            material: text(filament, "material"),
            colors: filament_colors(filament),
            longitudinal: filament.get("multi_color_direction").and_then(Value::as_str) == Some("longitudinal"),
            remaining_weight: number(value, "remaining_weight"),
            initial_weight: number(value, "initial_weight").or_else(|| number(filament, "weight")),
            spool_weight: number(value, "spool_weight").or_else(|| number(filament, "spool_weight")),
            density: number(filament, "density"),
            diameter: number(filament, "diameter"),
            extruder_temp: number(filament, "settings_extruder_temp"),
            bed_temp: number(filament, "settings_bed_temp"),
            location: text(value, "location"),
            last_used: text(value, "last_used"),
        })
    }

    /// The first colour, for places that show one.
    pub fn color(&self) -> Option<Rgba> {
        self.colors.first().copied()
    }

    /// Vendor and filament name, e.g. `Geeetech Red`.
    pub fn title(&self) -> String {
        [self.vendor.as_str(), self.filament_name.as_str()]
            .into_iter()
            .filter(|part| !part.is_empty())
            .collect::<Vec<_>>()
            .join(" ")
    }

    /// Share of the start weight that is left, 0 to 1.
    pub fn remaining_share(&self) -> Option<f64> {
        let (left, start) = (self.remaining_weight?, self.initial_weight.filter(|weight| *weight > 0.0)?);
        Some((left / start).clamp(0.0, 1.0))
    }

    pub fn is_low(&self) -> bool {
        self.remaining_share().is_some_and(|share| share < LOW_SHARE)
    }

    /// Weight in grams of a length of this filament; `None` without diameter and density.
    pub fn weight_of_length(&self, millimeters: f64) -> Option<f64> {
        let (diameter, density) = (self.diameter.filter(|d| *d > 0.0)?, self.density.filter(|d| *d > 0.0)?);
        let cubic_millimeters = std::f64::consts::PI * (diameter / 2.0).powi(2) * millimeters.max(0.0);
        Some(cubic_millimeters / 1000.0 * density)
    }
}

/// The colours of a filament: `multi_color_hexes` (comma separated) for a multi-colour filament, else `color_hex`.
fn filament_colors(filament: &Value) -> Vec<Rgba> {
    let text = |field: &str| filament.get(field).and_then(Value::as_str).filter(|text| !text.trim().is_empty());
    let hexes = text("multi_color_hexes").or_else(|| text("color_hex")).unwrap_or_default();
    hexes.split(',').filter_map(parse_hex).take(MAX_COLORS).collect()
}

/// What Moonraker knows about Spoolman, and the spools the printer needs now.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct SpoolmanState {
    /// Moonraker is connected to Spoolman.
    pub connected: bool,
    /// Usage Moonraker could not report yet.
    pub pending_reports: usize,
    /// The spool Moonraker tracks the filament use of.
    pub active_spool_id: Option<u32>,
    pub spools: HashMap<u32, Spool>,
}

/// `server.spoolman.status`.
pub fn parse_status(result: &Value) -> (bool, usize, Option<u32>) {
    (
        result.get("spoolman_connected").and_then(Value::as_bool).unwrap_or_default(),
        result.get("pending_reports").and_then(Value::as_array).map_or(0, Vec::len),
        result.get("spool_id").and_then(Value::as_u64).and_then(|id| u32::try_from(id).ok()),
    )
}

/// The answer of `server.spoolman.proxy`: the version 2 response has `response` and `error`, the old one is the
/// answer of Spoolman itself.
pub fn proxy_response(result: &Value) -> Result<Value, String> {
    if result.get("response").is_some() || result.get("error").is_some() {
        return match result.get("error").filter(|error| !error.is_null()) {
            Some(error) => Err(error.get("message").and_then(Value::as_str).unwrap_or("Spoolman error").to_owned()),
            None => Ok(result.get("response").cloned().unwrap_or(Value::Null)),
        };
    }
    Ok(result.clone())
}

pub fn parse_spool_list(value: &Value) -> Vec<Spool> {
    value.as_array().map(|spools| spools.iter().filter_map(Spool::from_json).collect()).unwrap_or_default()
}

/// The spool Moonraker tracks the filament use of; `None` for none.
pub fn set_active_spool(spool_id: Option<u32>) -> Call {
    Call::with("server.spoolman.post_spool_id", json!({ "spool_id": spool_id }))
}

/// Sets the weight of a spool from a measurement; `weight` is the gross weight of the spool in grams.
pub fn measure_spool(spool_id: u32, weight: f64) -> Call {
    Call::with(
        "server.spoolman.proxy",
        json!({
            "request_method": "PUT",
            "path": format!("/v1/spool/{spool_id}/measure"),
            "body": { "weight": weight },
            "use_v2_response": true
        }),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// Shape of `GET /v1/spool/18` of Spoolman 0.2x as the printer of the author returned it (values rounded).
    fn spool_18() -> Value {
        json!({
            "id": 18, "registered": "2026-02-02T22:10:00Z", "first_used": "2026-02-02T22:17:26Z",
            "last_used": "2026-10-26T12:12:18Z",
            "filament": {
                "id": 7, "name": "Red", "vendor": { "id": 3, "name": "Geeetech" }, "material": "ABS+",
                "density": 1.04, "diameter": 1.75, "weight": 1000.0, "spool_weight": 192.0,
                "settings_extruder_temp": 240, "settings_bed_temp": 120, "color_hex": "E31F2B"
            },
            "remaining_weight": 613.11, "used_weight": 386.89, "remaining_length": 245_099.2,
            "initial_weight": 1000.0, "spool_weight": 180.0, "location": "voron @ MMU Gate:0", "archived": false,
            "extra": { "printer_name": "\"voron\"", "mmu_gate_map": "\"{}\"" }
        })
    }

    fn spool(id: u32, material: &str, left: f64, color: [u8; 3]) -> Spool {
        Spool {
            id,
            material: material.into(),
            remaining_weight: Some(left),
            initial_weight: Some(1000.0),
            colors: vec![[color[0], color[1], color[2], 0xff]],
            ..Spool::default()
        }
    }

    #[test]
    fn reads_a_spool_as_spoolman_returns_it() {
        let spool = Spool::from_json(&spool_18()).unwrap();

        assert_eq!((spool.id, spool.title().as_str(), spool.material.as_str()), (18, "Geeetech Red", "ABS+"));
        assert_eq!(spool.colors, [[0xE3, 0x1F, 0x2B, 0xFF]]);
        assert_eq!(spool.color(), Some([0xE3, 0x1F, 0x2B, 0xFF]));
        assert_eq!((spool.extruder_temp, spool.bed_temp), (Some(240.0), Some(120.0)));
        assert_eq!((spool.density, spool.diameter, spool.spool_weight), (Some(1.04), Some(1.75), Some(180.0)));
        assert_eq!(spool.location, "voron @ MMU Gate:0");
        assert!((spool.remaining_share().unwrap() - 0.6131).abs() < 1e-3);
        assert!(!spool.is_low());
    }

    #[test]
    fn a_spool_without_start_weight_takes_that_of_its_filament() {
        let mut value = spool_18();
        value["initial_weight"] = Value::Null;

        assert_eq!(Spool::from_json(&value).unwrap().initial_weight, Some(1000.0));
    }

    #[test]
    fn a_spool_under_15_percent_is_low_and_without_start_weight_it_is_not() {
        assert!(spool(1, "PLA", 120.0, [0; 3]).is_low());
        assert!(!spool(1, "PLA", 150.0, [0; 3]).is_low());
        assert!(!Spool { remaining_weight: Some(1.0), ..Spool::default() }.is_low());
    }

    #[test]
    fn entries_without_an_id_are_skipped() {
        assert_eq!(parse_spool_list(&json!([{ "filament": {} }, spool_18()])).len(), 1);
        assert!(parse_spool_list(&json!({ "not": "a list" })).is_empty());
    }

    #[test]
    fn a_multi_colour_filament_has_up_to_three_colours() {
        let mut value = spool_18();
        value["filament"]["color_hex"] = Value::Null;
        value["filament"]["multi_color_hexes"] = json!("FF0004,FFA424,000000");
        value["filament"]["multi_color_direction"] = json!("coaxial");

        assert_eq!(
            Spool::from_json(&value).unwrap().colors,
            [[0xFF, 0x00, 0x04, 0xFF], [0xFF, 0xA4, 0x24, 0xFF], [0, 0, 0, 0xFF]]
        );
        assert!(!Spool::from_json(&value).unwrap().longitudinal);
        value["filament"]["multi_color_direction"] = json!("longitudinal");
        assert!(Spool::from_json(&value).unwrap().longitudinal);
        value["filament"]["multi_color_hexes"] = json!("111111,222222,333333,444444");
        assert_eq!(Spool::from_json(&value).unwrap().colors.len(), 3);
        value["filament"]["multi_color_hexes"] = json!("");
        value["filament"]["color_hex"] = json!("E31F2B80");
        assert_eq!(Spool::from_json(&value).unwrap().colors, [[0xE3, 0x1F, 0x2B, 0x80]]);
    }

    #[test]
    fn the_status_names_connection_pending_reports_and_active_spool() {
        let status = parse_status(&json!({ "spoolman_connected": true, "pending_reports": [{}, {}], "spool_id": 18 }));
        assert_eq!(status, (true, 2, Some(18)));
        assert_eq!(
            parse_status(&json!({ "spoolman_connected": false, "pending_reports": [], "spool_id": null })),
            (false, 0, None)
        );
    }

    #[test]
    fn the_proxy_answers_in_version_2_or_raw() {
        assert_eq!(proxy_response(&json!({ "response": { "id": 1 }, "error": null })), Ok(json!({ "id": 1 })));
        assert_eq!(
            proxy_response(
                &json!({ "response": null, "error": { "status_code": 404, "message": "No spool with ID 5 found" } })
            ),
            Err("No spool with ID 5 found".to_owned())
        );
        assert_eq!(proxy_response(&json!({ "id": 1 })), Ok(json!({ "id": 1 })));
    }

    #[test]
    fn the_weight_of_a_length_follows_diameter_and_density() {
        let spool = Spool::from_json(&spool_18()).unwrap();

        // 1 m of 1.75 mm ABS+ (1.04 g/cm³) weighs about 2.5 g.
        assert!((spool.weight_of_length(1000.0).unwrap() - 2.5).abs() < 0.01);
        assert_eq!(Spool { diameter: None, ..spool }.weight_of_length(1000.0), None);
    }

    #[test]
    fn spool_commands_go_through_moonrakers_component() {
        assert_eq!(set_active_spool(Some(18)), Call::with("server.spoolman.post_spool_id", json!({ "spool_id": 18 })));
        assert_eq!(set_active_spool(None).params.unwrap(), json!({ "spool_id": null }));
        let measure = measure_spool(18, 793.5);
        assert_eq!(measure.method, "server.spoolman.proxy");
        assert_eq!(
            measure.params.unwrap(),
            json!({ "request_method": "PUT", "path": "/v1/spool/18/measure", "body": { "weight": 793.5 }, "use_v2_response": true })
        );
    }
}
