//! The `configfile` object: the effective settings and their raw text.

use super::*;

impl Printer {
    pub(super) fn configfile_json(&self) -> Value {
        let settings = self.config_sections();
        let mut raw = raw_config(&settings);
        raw.extend(self.calibration.raw_config_sections());
        json!({
            "config": raw,
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
                "nozzle_diameter": crate::files::NOZZLE_DIAMETER,
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
        // `control` and the PID values of printer.cfg, lower-case keys like Klipper's `settings`.
        for section in ["extruder", "heater_bed"] {
            if let (Some(Value::Object(options)), Some(Value::Object(pid))) =
                (sections.get_mut(section), self.pid_config.settings(section))
            {
                options.extend(pid);
            }
        }
        // Like Klipper: `settings` lists min_temp and max_temp of every temperature sensor with its defaults.
        for (name, maximum) in
            [("temperature_sensor raspberry_pi", MAX_HOST_TEMPERATURE), ("temperature_sensor mcu", MAX_MCU_TEMPERATURE)]
        {
            sections.insert(name.to_owned(), json!({ "min_temp": MIN_SENSOR_TEMPERATURE, "max_temp": maximum }));
        }
        if self.has_printer_fan() {
            sections.insert("fan".to_owned(), json!({}));
        }
        if self.chamber.is_some() {
            sections.insert(
                "heater_generic chamber".to_owned(),
                json!({ "min_temp": MIN_HEATER_TEMPERATURE, "max_temp": MAX_CHAMBER_TARGET, "control": "watermark" }),
            );
        }
        if self.profile.quad_gantry_level {
            sections.insert("quad_gantry_level".to_owned(), json!({ "max_adjust": 10 }));
        }
        if self.profile.z_thermal_adjust {
            sections.insert(
                "z_thermal_adjust".to_owned(),
                json!({
                    "temp_coeff": Z_THERMAL_TEMP_COEFF,
                    "sensor_type": "temperature_combined",
                    "sensor_list": "extruder",
                    "combination_method": "mean",
                    "maximum_deviation": 999.9,
                }),
            );
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
