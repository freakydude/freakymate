//! Simulated Spoolman behind Moonraker's `spoolman` component: spools, the active spool and the answers of the proxy.
//!
//! The spools take their material, colour and temperature from the demo filaments of the simulated Happy Hare, so a
//! gate and its spool describe the same filament. Field names are those of Spoolman 0.2x as a real one returned them.

use crate::happy_hare::{BASIC_FILAMENTS, SPOOLS};
use serde_json::{json, Value};

const SPOOL_COUNT: u32 = 8;
const START_WEIGHT: f64 = 1000.0;
/// Weight of the empty spool in grams.
const SPOOL_WEIGHT: f64 = 180.0;
/// Grams left per spool; the spools 2 and 7 are nearly empty, spool 5 was never used.
const REMAINING: [f64; SPOOL_COUNT as usize] = [812.0, 40.0, 905.0, 613.1, 1000.0, 720.0, 38.0, 802.0];
const NEVER_USED: u32 = 5;
/// Spools of a filament with two and with three colours.
const MULTI_COLORS: [(u32, &str); 2] = [(7, "E31F2B,2E7D32"), (8, "FF0004,FFA424,000000")];

#[derive(Debug, Clone)]
pub struct Spoolman {
    remaining: [f64; SPOOL_COUNT as usize],
    active: Option<u32>,
}

/// Moonraker keeps the active spool across restarts, so a printer starts with one.
const STARTING_SPOOL: u32 = 4;

impl Default for Spoolman {
    fn default() -> Self {
        Self { remaining: REMAINING, active: Some(STARTING_SPOOL) }
    }
}

impl Spoolman {
    pub fn status(&self) -> Value {
        json!({ "spoolman_connected": true, "pending_reports": [], "spool_id": self.active })
    }

    /// Sets the active spool; true if that changed it.
    pub fn set_active(&mut self, id: Option<u32>) -> Result<bool, String> {
        if let Some(id) = id {
            self.check(id)?;
        }
        let changed = self.active != id;
        self.active = id;
        Ok(changed)
    }

    /// Answers a proxied request like Spoolman: the body, or the status code and the message of the error.
    pub fn proxy(&mut self, method: &str, path: &str, body: &Value) -> Result<Value, (u16, String)> {
        let path = path.trim_end_matches('/');
        let rest = path.strip_prefix("/v1/spool").ok_or((404, format!("Not found: {path}")))?;
        let segments: Vec<&str> = rest.split('/').filter(|segment| !segment.is_empty()).collect();
        match (method, segments.as_slice()) {
            ("GET", []) => Ok(self.list()),
            ("GET", [id]) => self.spool(parse_id(id)?),
            ("PUT", [id, "measure"]) => {
                let id = parse_id(id)?;
                let weight = body.get("weight").and_then(Value::as_f64).ok_or((422, "Missing weight".to_owned()))?;
                self.check(id).map_err(|message| (404, message))?;
                self.remaining[(id - 1) as usize] = (weight - SPOOL_WEIGHT).max(0.0);
                self.spool(id)
            }
            _ => Err((404, format!("Not found: {method} {path}"))),
        }
    }

    fn check(&self, id: u32) -> Result<(), String> {
        if (1..=SPOOL_COUNT).contains(&id) {
            Ok(())
        } else {
            Err(format!("No spool with ID {id} found."))
        }
    }

    fn spool(&self, id: u32) -> Result<Value, (u16, String)> {
        self.check(id).map_err(|message| (404, message))?;
        let filament = &SPOOLS[filament_index(id)];
        let (bed_temp, density) = match filament.material {
            "PETG" => (80, 1.27),
            "ABS" => (105, 1.04),
            _ => (60, 1.24),
        };
        let remaining = self.remaining[(id - 1) as usize];
        let mut spool = json!({
            "id": id,
            "registered": "2026-01-10T09:00:00Z",
            "filament": {
                "id": id,
                "name": filament.name,
                "vendor": { "id": 1, "name": filament.vendor },
                "material": filament.material,
                "density": density,
                "diameter": 1.75,
                "weight": START_WEIGHT,
                "spool_weight": SPOOL_WEIGHT,
                "settings_extruder_temp": filament.temperature,
                "settings_bed_temp": bed_temp,
                "color_hex": format!("{:02X}{:02X}{:02X}", channel(filament.rgb[0]), channel(filament.rgb[1]), channel(filament.rgb[2])),
            },
            "remaining_weight": remaining,
            "used_weight": START_WEIGHT - remaining,
            "initial_weight": START_WEIGHT,
            "spool_weight": SPOOL_WEIGHT,
            "location": "",
            "archived": false,
            "extra": {},
        });
        // Two multi-colour filaments, as Spoolman has them: no `color_hex`, the colours separated by commas. The two
        // colours follow each other along the filament (a gradient), the three lie side by side (a silk).
        if let Some(hexes) = MULTI_COLORS.iter().find(|(spool_id, _)| *spool_id == id).map(|(_, hexes)| hexes) {
            spool["filament"]["color_hex"] = Value::Null;
            spool["filament"]["multi_color_hexes"] = json!(hexes);
            let direction = if hexes.split(',').count() == 2 { "longitudinal" } else { "coaxial" };
            spool["filament"]["multi_color_direction"] = json!(direction);
        }
        if id != NEVER_USED {
            spool["first_used"] = json!("2026-02-02T22:17:26Z");
            spool["last_used"] = json!(format!("2026-10-{:02}T12:00:00Z", 10 + id));
        }
        Ok(spool)
    }

    /// Most recently used first; the spool nobody used comes last.
    fn list(&self) -> Value {
        let mut ids: Vec<u32> = (1..=SPOOL_COUNT).collect();
        ids.sort_by_key(|id| std::cmp::Reverse(if *id == NEVER_USED { 0 } else { *id }));
        Value::Array(ids.into_iter().filter_map(|id| self.spool(id).ok()).collect())
    }
}

/// A spool as Spoolman answers `GET /v1/spool/<id>` before anything was measured, for plugins that read it themselves
/// (AFC); `None` for an id Spoolman does not know.
pub(crate) fn initial_spool(id: u32) -> Option<Value> {
    Spoolman::default().spool(id).ok()
}

/// The demo filament of a spool: the multi-colour spools 7 and 8 have their own, the others cycle through the basic ones.
pub(crate) fn filament_index(id: u32) -> usize {
    match id {
        7 => BASIC_FILAMENTS,
        8 => BASIC_FILAMENTS + 1,
        _ => (id as usize - 1) % BASIC_FILAMENTS,
    }
}

fn parse_id(text: &str) -> Result<u32, (u16, String)> {
    text.parse().map_err(|_| (422, format!("Invalid spool ID {text}")))
}

fn channel(value: f64) -> u8 {
    (value.clamp(0.0, 1.0) * 255.0).round() as u8
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_spool_has_the_fields_of_spoolman_and_the_temperatures_of_its_filament() {
        let spool = Spoolman::default().proxy("GET", "/v1/spool/2", &Value::Null).unwrap();

        assert_eq!(spool["filament"]["material"], "PETG");
        assert_eq!(spool["filament"]["settings_extruder_temp"], 235);
        assert_eq!(spool["filament"]["settings_bed_temp"], 80);
        assert_eq!(spool["remaining_weight"], 40.0);
        assert!(spool["filament"]["color_hex"].as_str().unwrap().len() == 6);
    }

    #[test]
    fn multi_colour_filaments_list_their_colours_instead_of_one() {
        let mut spoolman = Spoolman::default();
        let two = spoolman.proxy("GET", "/v1/spool/7", &Value::Null).unwrap();
        let three = spoolman.proxy("GET", "/v1/spool/8", &Value::Null).unwrap();

        assert_eq!(two["filament"]["multi_color_hexes"], "E31F2B,2E7D32");
        assert_eq!(two["filament"]["multi_color_direction"], "longitudinal");
        assert_eq!(three["filament"]["multi_color_direction"], "coaxial");
        assert_eq!(three["filament"]["multi_color_hexes"].as_str().unwrap().split(',').count(), 3);
        assert!(three["filament"]["color_hex"].is_null());
    }

    #[test]
    fn the_list_comes_most_recently_used_first_and_the_unused_spool_last() {
        let list = Spoolman::default().proxy("GET", "/v1/spool", &Value::Null).unwrap();
        let ids: Vec<u64> = list.as_array().unwrap().iter().map(|spool| spool["id"].as_u64().unwrap()).collect();

        assert_eq!(ids, [8, 7, 6, 4, 3, 2, 1, 5]);
    }

    #[test]
    fn measuring_sets_the_remaining_weight_from_the_gross_weight() {
        let mut spoolman = Spoolman::default();

        let spool = spoolman.proxy("PUT", "/v1/spool/1/measure", &json!({ "weight": 780.0 })).unwrap();

        assert_eq!(spool["remaining_weight"], 600.0);
    }

    #[test]
    fn an_unknown_spool_is_an_error_like_spoolman_reports_it() {
        let mut spoolman = Spoolman::default();

        assert_eq!(spoolman.proxy("GET", "/v1/spool/99", &Value::Null).unwrap_err().0, 404);
        assert!(spoolman.set_active(Some(99)).is_err());
        assert_eq!(spoolman.status()["spool_id"], 4);
        assert_eq!(spoolman.set_active(Some(6)), Ok(true));
        assert_eq!(spoolman.set_active(Some(6)), Ok(false));
        assert_eq!(spoolman.status()["spool_id"], 6);
    }
}
