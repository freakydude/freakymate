//! Fans, LEDs, LED effects and extra temperature sensors of the `voron` and `happy-hare` profiles. Object and field
//! names follow Klipper's Status Reference and the led_effect plugin (`enabled`); values are made up.

use crate::gcode::{CommandError, DeviceCommand};
use serde_json::{Map, Value, json};
use std::collections::BTreeSet;

/// Generic fans: name and the speed its tachometer reports at 100 %, `None` without a tachometer.
const GENERIC_FANS: [(&str, Option<f64>); 2] = [("nevermore", None), ("exhaust", Some(2800.0))];
const HOTEND_FAN_RPM: f64 = 7200.0;
const BOARD_FAN_RPM: f64 = 3100.0;
/// Instance names of the fans with a tachometer, which `FAKE_FAN_STALL` can block.
const TACHOMETER_FANS: [&str; 3] = ["exhaust", "hotend", "board"];
/// The hotend fan of Klipper runs from this nozzle temperature on (`heater_temp` default).
const HOTEND_FAN_TEMPERATURE: f64 = 50.0;
/// Stepper drivers only report a temperature while they are enabled.
const DRIVER_TEMPERATURES: [(&str, f64); 3] = [("stepper_x", 36.5), ("stepper_y", 37.2), ("stepper_z", 33.9)];
const EFFECTS: [&str; 2] = ["rainbow", "critical_error"];

#[derive(Debug, Clone)]
struct FakeLed {
    name: &'static str,
    chips: usize,
    /// RGBW (`color_order` GRBW) or RGB only.
    white: bool,
    color: [f64; 4],
}

/// What the machine reads from the rest of the printer to drive the automatic fans and the driver temperatures.
#[derive(Debug, Clone, Copy)]
pub struct DeviceInputs {
    pub nozzle_temperature: f64,
    pub steppers_enabled: bool,
}

#[derive(Debug, Clone)]
pub struct Devices {
    generic_speeds: [f64; GENERIC_FANS.len()],
    leds: Vec<FakeLed>,
    effects: [bool; EFFECTS.len()],
    /// Fans whose tachometer reports 0 rpm whatever their speed.
    stalled: BTreeSet<String>,
}

impl Default for Devices {
    fn default() -> Self {
        Self {
            generic_speeds: [0.0; GENERIC_FANS.len()],
            leds: vec![
                FakeLed { name: "chamber", chips: 2, white: true, color: [0.0, 0.0, 0.0, 0.8] },
                FakeLed { name: "status", chips: 3, white: false, color: [0.0, 0.0, 0.0, 0.0] },
            ],
            effects: [false; EFFECTS.len()],
            stalled: BTreeSet::new(),
        }
    }
}

impl Devices {
    pub fn apply(&mut self, command: DeviceCommand) -> Result<(), CommandError> {
        match command {
            DeviceCommand::SetFanSpeed { fan, speed } => {
                let index = GENERIC_FANS
                    .iter()
                    .position(|(name, _)| *name == fan)
                    .ok_or_else(|| CommandError::new(format!("Unknown fan '{fan}'")))?;
                self.generic_speeds[index] = speed.clamp(0.0, 1.0);
            }
            DeviceCommand::SetLed { led, color } => {
                let led = self
                    .leds
                    .iter_mut()
                    .find(|candidate| candidate.name == led)
                    .ok_or_else(|| CommandError::new(format!("Unknown LED '{led}'")))?;
                led.color = color;
            }
            DeviceCommand::SetLedEffect { effect, stop } => {
                let index = EFFECTS
                    .iter()
                    .position(|name| *name == effect)
                    .ok_or_else(|| CommandError::new(format!("Unknown led effect '{effect}'")))?;
                self.effects[index] = !stop;
            }
            DeviceCommand::StopLedEffects => self.effects = [false; EFFECTS.len()],
            DeviceCommand::FakeFanStall { fan, stall } => {
                if !TACHOMETER_FANS.contains(&fan.as_str()) {
                    return Err(CommandError::new(format!("Unknown fan with a tachometer '{fan}'")));
                }
                if stall {
                    self.stalled.insert(fan);
                } else {
                    self.stalled.remove(&fan);
                }
            }
        }
        Ok(())
    }

    pub fn status(&self, inputs: DeviceInputs) -> Map<String, Value> {
        let mut objects = Map::new();
        for ((name, tachometer), speed) in GENERIC_FANS.iter().zip(self.generic_speeds) {
            objects.insert(format!("fan_generic {name}"), self.fan(name, speed, *tachometer));
        }
        let hotend = if inputs.nozzle_temperature >= HOTEND_FAN_TEMPERATURE { 1.0 } else { 0.0 };
        objects.insert("heater_fan hotend".to_owned(), self.fan("hotend", hotend, Some(HOTEND_FAN_RPM)));
        let board = if inputs.steppers_enabled { 1.0 } else { 0.0 };
        objects.insert("controller_fan board".to_owned(), self.fan("board", board, Some(BOARD_FAN_RPM)));
        for led in &self.leds {
            // Klipper reports four channels per chip, also for RGB-only LEDs.
            let mut color = led.color;
            if !led.white {
                color[3] = 0.0;
            }
            objects.insert(format!("neopixel {}", led.name), json!({ "color_data": vec![color; led.chips] }));
        }
        for (name, enabled) in EFFECTS.iter().zip(self.effects) {
            objects.insert(format!("led_effect {name}"), json!({ "enabled": enabled }));
        }
        for (stepper, temperature) in DRIVER_TEMPERATURES {
            let temperature = inputs.steppers_enabled.then_some(temperature);
            objects.insert(format!("tmc2240 {stepper}"), json!({ "temperature": temperature }));
        }
        objects
    }

    /// A fan's status: the tachometer reports its share of the full speed, 0 rpm while stalled.
    fn fan(&self, name: &str, speed: f64, tachometer: Option<f64>) -> Value {
        let rpm = tachometer.map(|full| if self.stalled.contains(name) { 0.0 } else { (full * speed).round() });
        json!({ "speed": speed, "rpm": rpm })
    }

    pub fn config_sections(&self) -> Vec<(String, Value)> {
        let mut sections: Vec<(String, Value)> = GENERIC_FANS
            .iter()
            .map(|(name, tachometer)| {
                (
                    format!("fan_generic {name}"),
                    json!({ "tachometer_pin": tachometer.map(|_| "PB1"), "max_power": 1.0 }),
                )
            })
            .collect();
        sections.push(("heater_fan hotend".to_owned(), json!({ "heater": ["extruder"], "heater_temp": 50.0 })));
        sections.push(("controller_fan board".to_owned(), json!({ "stepper": ["stepper_x", "stepper_y"] })));
        for led in &self.leds {
            let order = if led.white { "GRBW" } else { "GRB" };
            sections
                .push((format!("neopixel {}", led.name), json!({ "color_order": [order], "chain_count": led.chips })));
        }
        sections
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const IDLE: DeviceInputs = DeviceInputs { nozzle_temperature: 22.0, steppers_enabled: false };

    #[test]
    fn generic_fans_follow_set_fan_speed_and_report_rpm_with_a_tachometer() {
        let mut devices = Devices::default();
        devices.apply(DeviceCommand::SetFanSpeed { fan: "exhaust".into(), speed: 0.5 }).unwrap();

        let status = devices.status(IDLE);

        assert_eq!(status["fan_generic exhaust"], json!({ "speed": 0.5, "rpm": 1400.0 }));
        assert_eq!(status["fan_generic nevermore"]["rpm"], Value::Null);
        assert!(devices.apply(DeviceCommand::SetFanSpeed { fan: "hotend".into(), speed: 1.0 }).is_err());
    }

    #[test]
    fn a_stalled_fan_reports_0_rpm_until_released() {
        let mut devices = Devices::default();
        let hot = DeviceInputs { nozzle_temperature: 200.0, steppers_enabled: true };
        devices.apply(DeviceCommand::FakeFanStall { fan: "hotend".into(), stall: true }).unwrap();

        assert_eq!(devices.status(hot)["heater_fan hotend"], json!({ "speed": 1.0, "rpm": 0.0 }));
        devices.apply(DeviceCommand::FakeFanStall { fan: "hotend".into(), stall: false }).unwrap();
        assert_eq!(devices.status(hot)["heater_fan hotend"]["rpm"], HOTEND_FAN_RPM);
        assert!(devices.apply(DeviceCommand::FakeFanStall { fan: "nevermore".into(), stall: true }).is_err());
    }

    #[test]
    fn automatic_fans_and_drivers_follow_the_printer() {
        let devices = Devices::default();
        let hot = devices.status(DeviceInputs { nozzle_temperature: 200.0, steppers_enabled: true });

        assert_eq!(hot["heater_fan hotend"]["speed"], 1.0);
        assert_eq!(hot["controller_fan board"]["rpm"], BOARD_FAN_RPM);
        assert_eq!(hot["tmc2240 stepper_x"]["temperature"], 36.5);
        assert_eq!(devices.status(IDLE)["tmc2240 stepper_x"]["temperature"], Value::Null);
    }

    #[test]
    fn leds_keep_four_channels_and_rgb_ones_drop_white() {
        let mut devices = Devices::default();
        devices.apply(DeviceCommand::SetLed { led: "status".into(), color: [1.0, 0.0, 0.0, 1.0] }).unwrap();
        devices.apply(DeviceCommand::SetLedEffect { effect: "rainbow".into(), stop: false }).unwrap();

        let status = devices.status(IDLE);

        assert_eq!(status["neopixel status"]["color_data"][2], json!([1.0, 0.0, 0.0, 0.0]));
        assert_eq!(status["neopixel chamber"]["color_data"][0], json!([0.0, 0.0, 0.0, 0.8]));
        assert_eq!(status["led_effect rainbow"]["enabled"], true);
        devices.apply(DeviceCommand::StopLedEffects).unwrap();
        assert_eq!(devices.status(IDLE)["led_effect rainbow"]["enabled"], false);
    }
}
