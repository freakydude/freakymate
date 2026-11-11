//! The `configfile` object: the effective settings and their raw text.

use super::*;

impl Printer {
    pub(super) fn configfile_json(&self) -> Value {
        let settings = self.config_sections();
        json!({
            "config": raw_config(&settings),
            "settings": settings,
            "save_config_pending": self.calibration.has_pending(),
            "save_config_pending_items": self.calibration.pending_json(),
        })
    }

    fn config_sections(&self) -> Map<String, Value> {
        let mut sections = Map::new();
        let limits = CONFIGURED_LIMITS;
        sections.insert(
            "printer".to_owned(),
            json!({
                "kinematics": "corexy",
                "max_velocity": limits.max_velocity,
                "max_accel": limits.max_accel,
                "square_corner_velocity": limits.square_corner_velocity,
                "minimum_cruise_ratio": limits.minimum_cruise_ratio,
            }),
        );
        sections.insert(
            "extruder".to_owned(),
            json!({
                "filament_diameter": 1.75,
                "pressure_advance": CONFIGURED_PRESSURE_ADVANCE,
                "pressure_advance_smooth_time": CONFIGURED_SMOOTH_TIME,
                "min_extrude_temp": MIN_EXTRUDE_TEMPERATURE,
                // Klipper's default; `settings` lists defaults too.
                "max_extrude_only_distance": 50.0,
                "min_temp": MIN_HEATER_TEMPERATURE,
                "max_temp": MAX_EXTRUDER_TARGET
            }),
        );
        sections
            .insert("heater_bed".to_owned(), json!({ "min_temp": MIN_HEATER_TEMPERATURE, "max_temp": MAX_BED_TARGET }));
        sections.insert("fan".to_owned(), json!({}));
        if self.chamber.is_some() {
            sections.insert(
                "heater_generic chamber".to_owned(),
                json!({ "min_temp": MIN_HEATER_TEMPERATURE, "max_temp": MAX_CHAMBER_TARGET }),
            );
        }
        if self.profile.quad_gantry_level {
            sections.insert("quad_gantry_level".to_owned(), json!({ "max_adjust": 10 }));
        }
        for (name, template) in MACRO_TEMPLATES {
            let mut section = json!({ "gcode": template });
            if name == "PAUSE" {
                section["rename_existing"] = json!("BASE_PAUSE");
            }
            if name == PRINT_STATS_INFO_MACRO {
                section["rename_existing"] = json!("SET_PRINT_STATS_INFO_BASE");
            }
            sections.insert(format!("gcode_macro {name}"), section);
        }
        sections.extend(self.calibration.config_sections());
        if let Some(toolchanger) = &self.toolchanger {
            sections.extend(toolchanger.config_sections());
        }
        if let Some(mmu) = &self.mmu {
            sections.extend(mmu.config_sections());
        }
        if let Some(afc) = &self.afc {
            sections.extend(afc.config_sections());
        }
        if let Some(devices) = &self.devices {
            sections.extend(devices.config_sections());
        }
        if self.retraction.is_some() {
            let retraction = CONFIGURED_RETRACTION;
            sections.insert(
                "firmware_retraction".to_owned(),
                json!({
                    "retract_length": retraction.retract_length,
                    "retract_speed": retraction.retract_speed,
                    "unretract_extra_length": retraction.unretract_extra_length,
                    "unretract_speed": retraction.unretract_speed,
                }),
            );
        }
        sections
    }
}

fn raw_config(settings: &Map<String, Value>) -> Map<String, Value> {
    settings
        .iter()
        .map(|(section, options)| {
            let raw_options: Map<String, Value> = options.as_object().map_or_else(Map::new, |options| {
                options.iter().map(|(name, value)| (name.clone(), Value::String(raw_value(value)))).collect()
            });
            (section.clone(), Value::Object(raw_options))
        })
        .collect()
}

fn raw_value(value: &Value) -> String {
    match value {
        Value::String(text) => text.clone(),
        other => other.to_string(),
    }
}
