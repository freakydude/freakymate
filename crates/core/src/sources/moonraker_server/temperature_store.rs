//! Temperature history from Moonraker's temperature store (`server.temperature_store`): about the last 20 minutes,
//! one value per second, for heaters and temperature sensors. Live values are appended while a chart is open.

use crate::moonraker::Call;
use serde_json::{json, Value};
use std::collections::HashMap;

/// Moonraker keeps 1200 values by default (`temperature_store_size`); the live part keeps the same length.
pub const HISTORY_LENGTH: usize = 1200;

#[derive(Debug, Clone, Default, PartialEq)]
pub struct TemperatureSeries {
    pub temperatures: Vec<f64>,
    /// Empty for sensors without a target.
    pub targets: Vec<f64>,
    /// Heater power from 0 to 1; empty for sensors.
    pub powers: Vec<f64>,
}

impl TemperatureSeries {
    /// Adds the newest value and drops the oldest beyond [`HISTORY_LENGTH`].
    pub fn push(&mut self, temperature: f64, target: Option<f64>, power: Option<f64>) {
        push_bounded(&mut self.temperatures, temperature);
        if let Some(target) = target {
            push_bounded(&mut self.targets, target);
        }
        if let Some(power) = power {
            push_bounded(&mut self.powers, power);
        }
    }
}

fn push_bounded(values: &mut Vec<f64>, value: f64) {
    values.push(value);
    if values.len() > HISTORY_LENGTH {
        values.drain(..values.len() - HISTORY_LENGTH);
    }
}

/// `server.temperature_store` without the monitors.
pub fn store_call() -> Call {
    Call::with("server.temperature_store", json!({ "include_monitors": false }))
}

/// The answer of `server.temperature_store`, by Klipper object name.
pub fn parse_temperature_store(result: &Value) -> HashMap<String, TemperatureSeries> {
    let Some(objects) = result.as_object() else { return HashMap::new() };
    let series = |value: &Value, field: &str| -> Vec<f64> {
        value
            .get(field)
            .and_then(Value::as_array)
            .map(|values| values.iter().filter_map(Value::as_f64).collect())
            .unwrap_or_default()
    };
    objects
        .iter()
        .map(|(name, value)| {
            let entry = TemperatureSeries {
                temperatures: series(value, "temperatures"),
                targets: series(value, "targets"),
                powers: series(value, "powers"),
            };
            (name.clone(), entry)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn the_store_is_read_per_object() {
        let store = parse_temperature_store(&json!({
            "extruder": { "temperatures": [21.0, 22.5], "targets": [0.0, 200.0], "powers": [0.0, 1.0] },
            "temperature_sensor chamber": { "temperatures": [30.1] }
        }));

        assert_eq!(store["extruder"].targets, [0.0, 200.0]);
        assert_eq!(store["temperature_sensor chamber"].temperatures, [30.1]);
        assert!(store["temperature_sensor chamber"].powers.is_empty());
        assert!(parse_temperature_store(&json!(null)).is_empty());
    }

    #[test]
    fn live_values_keep_the_length_of_the_store() {
        let mut series = TemperatureSeries { temperatures: vec![20.0; HISTORY_LENGTH], ..TemperatureSeries::default() };
        series.push(25.0, Some(200.0), None);

        assert_eq!(series.temperatures.len(), HISTORY_LENGTH);
        assert_eq!(series.temperatures.last(), Some(&25.0));
        assert_eq!(series.targets, [200.0]);
    }
}
