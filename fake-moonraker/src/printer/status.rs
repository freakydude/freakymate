//! The printer objects as Klipper reports them, one function per object group.

use super::*;

impl Printer {
    pub fn object_names(&self) -> Vec<String> {
        match self.status() {
            Value::Object(objects) => objects.into_iter().map(|(name, _)| name).collect(),
            _ => Vec::new(),
        }
    }

    pub fn status(&self) -> Value {
        let mut objects = self.core_status();
        self.add_modules(&mut objects);
        self.add_sensors(&mut objects);
        self.add_toolchanger_status(&mut objects);
        if let Some(retraction) = self.retraction {
            objects.insert("firmware_retraction".to_owned(), retraction_json(retraction));
        }
        for name in MACROS {
            objects.insert(format!("gcode_macro {name}"), json!({}));
        }
        objects.insert(format!("gcode_macro {PRINT_STATS_INFO_MACRO}"), self.layer_pause.macro_status());
        Value::Object(objects)
    }

    /// The objects that a machine profile or a module adds: gantry, objects to exclude, calibration, MMU, devices.
    fn add_modules(&self, objects: &mut Map<String, Value>) {
        if self.profile.quad_gantry_level {
            objects.insert("quad_gantry_level".to_owned(), json!({ "applied": self.gantry_leveled }));
        }
        if self.profile.exclude_object {
            objects.insert("exclude_object".to_owned(), self.exclude_object_json());
        }
        objects.extend(self.calibration.status());
        if let Some(mmu) = &self.mmu {
            objects.extend(mmu.status(self.print_state.as_str()));
        }
        if let Some(devices) = &self.devices {
            let inputs = DeviceInputs {
                nozzle_temperature: self.extruder.temperature,
                steppers_enabled: self.homed.iter().any(|homed| *homed),
            };
            objects.extend(devices.status(inputs));
        }
    }

    /// Temperature and filament sensors beside the heaters.
    fn add_sensors(&self, objects: &mut Map<String, Value>) {
        if let Some(chamber) = self.chamber {
            objects.insert("heater_generic chamber".to_owned(), chamber.to_json());
        }
        let host = round(self.host_temperature(), 2);
        objects.insert("temperature_sensor raspberry_pi".to_owned(), json!({ "temperature": host }));
        // Like Klipper: the sensor type `temperature_host` adds its own object beside the section.
        objects.insert("temperature_host raspberry_pi".to_owned(), json!({ "temperature": host }));
        let sensor_names: &[&str] = if self.mmu.is_some() { &MMU_FILAMENT_SENSORS } else { &FILAMENT_SENSORS };
        for (index, name) in sensor_names.iter().take(self.profile.filament_sensors).enumerate() {
            objects.insert((*name).to_owned(), json!({ "enabled": true, "filament_detected": index != 1 }));
        }
    }

    fn exclude_object_json(&self) -> Value {
        json!({
            "objects": print_objects::objects(self.object_definitions()),
            "excluded_objects": self.excluded_objects,
            "current_object": self.current_object(),
        })
    }

    fn core_status(&self) -> Map<String, Value> {
        let Value::Object(objects) = json!({
            "webhooks": { "state": self.klippy.as_str(), "state_message": self.klippy.message() },
            "idle_timeout": { "state": self.idle_state(), "printing_time": 0.0 },
            "heater_bed": self.bed.to_json(),
            "extruder": self.extruder_json(),
            "toolhead": self.toolhead_json(),
            "fan": { "speed": self.fan_speed, "rpm": null },
            "gcode_move": {
                "homing_origin": [0.0, 0.0, self.z_offset, 0.0],
                "gcode_position": self.gcode_position(),
                "speed": 1500.0,
                "speed_factor": self.speed_factor,
                "extrude_factor": self.extrude_factor,
            },
            "print_stats": self.print_stats_json(),
            "display_status": { "progress": round(self.progress(), 4), "message": null },
            "motion_report": {
                "live_position": self.rounded_position(),
                "live_velocity": 0.0,
                "live_extruder_velocity": 0.0,
                "steppers": ["stepper_x", "stepper_y", "stepper_z", "extruder"],
                "trapq": ["toolhead", "extruder"],
            },
            "configfile": self.configfile_json(),
        }) else {
            unreachable!("json! object literal");
        };
        objects
    }

    fn extruder_json(&self) -> Value {
        self.heater_json(&self.extruder)
    }

    pub(super) fn heater_json(&self, heater: &Heater) -> Value {
        json!({
            "temperature": round(heater.temperature, 2),
            "target": heater.target,
            "power": if heater.target > heater.temperature { 1.0 } else { 0.0 },
            "pressure_advance": self.pressure_advance,
            "smooth_time": self.smooth_time,
            "can_extrude": heater.temperature >= MIN_EXTRUDE_TEMPERATURE,
        })
    }

    fn toolhead_json(&self) -> Value {
        let mut toolhead = json!({
            "position": self.rounded_position(),
            "homed_axes": self.homed_axes(),
            "max_accel": self.limits.max_accel,
            "axis_minimum": AXIS_MINIMUM,
            "axis_maximum": AXIS_MAXIMUM,
            "max_velocity": self.limits.max_velocity,
            "square_corner_velocity": self.limits.square_corner_velocity,
            "minimum_cruise_ratio": self.limits.minimum_cruise_ratio,
        });
        if let Some(toolchanger) = &self.toolchanger {
            toolhead["extruder"] = json!(toolchanger.active_extruder_name());
        }
        toolhead
    }

    fn print_stats_json(&self) -> Value {
        let (current_layer, total_layer) = self.layers();
        json!({
            "filename": self.filename,
            "total_duration": round(self.total_duration, 2),
            "print_duration": round(self.print_duration, 2),
            "filament_used": round(self.filament_used(), 2),
            "state": self.print_state.as_str(),
            "message": self.message,
            "info": { "current_layer": current_layer, "total_layer": total_layer },
        })
    }
}

fn retraction_json(retraction: Retraction) -> Value {
    json!({
        "retract_length": retraction.retract_length,
        "retract_speed": retraction.retract_speed,
        "unretract_extra_length": retraction.unretract_extra_length,
        "unretract_speed": retraction.unretract_speed,
    })
}
