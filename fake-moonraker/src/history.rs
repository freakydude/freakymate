//! Moonraker's temperature store (`server.temperature_store`): one value per simulated second for every heater and
//! temperature sensor, the newest last.

use serde_json::{Map, Value, json};
use std::collections::VecDeque;

/// Moonraker's default `temperature_store_size`.
const STORE_SIZE: usize = 1200;

/// One sample: temperature, and for heaters target and power.
#[derive(Debug, Clone, Copy)]
pub struct Sample {
    pub temperature: f64,
    pub heater: Option<(f64, f64)>,
}

#[derive(Debug, Clone, Default)]
pub struct TemperatureStore {
    series: Vec<(String, VecDeque<Sample>)>,
    /// Simulated seconds not yet sampled.
    pending: f64,
}

impl TemperatureStore {
    /// Takes one sample per full simulated second; `sample` is asked for the current values.
    pub fn advance(&mut self, seconds: f64, sample: impl Fn() -> Vec<(String, Sample)>) {
        self.pending += seconds;
        while self.pending >= 1.0 {
            self.pending -= 1.0;
            for (name, value) in sample() {
                let index = match self.series.iter().position(|(candidate, _)| *candidate == name) {
                    Some(index) => index,
                    None => {
                        self.series.push((name, VecDeque::new()));
                        self.series.len() - 1
                    }
                };
                let values = &mut self.series[index].1;
                values.push_back(value);
                if values.len() > STORE_SIZE {
                    values.pop_front();
                }
            }
        }
    }

    pub fn to_json(&self) -> Value {
        let mut objects = Map::new();
        for (name, samples) in &self.series {
            let round = |value: f64| (value * 100.0).round() / 100.0;
            let mut entry =
                json!({ "temperatures": samples.iter().map(|sample| round(sample.temperature)).collect::<Vec<_>>() });
            if samples.iter().any(|sample| sample.heater.is_some()) {
                let heater = |pick: fn((f64, f64)) -> f64| -> Vec<f64> {
                    samples.iter().map(|sample| sample.heater.map_or(0.0, pick)).collect()
                };
                entry["targets"] = json!(heater(|(target, _)| target));
                entry["powers"] = json!(heater(|(_, power)| power));
            }
            objects.insert(name.clone(), entry);
        }
        Value::Object(objects)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn one_sample_per_simulated_second_with_targets_for_heaters() {
        let mut store = TemperatureStore::default();
        let sample = || {
            vec![
                ("extruder".to_owned(), Sample { temperature: 21.004, heater: Some((200.0, 1.0)) }),
                ("temperature_sensor host".to_owned(), Sample { temperature: 45.0, heater: None }),
            ]
        };
        store.advance(2.5, sample);
        store.advance(0.4, sample);

        let json = store.to_json();
        assert_eq!(json["extruder"]["temperatures"], json!([21.0, 21.0]));
        assert_eq!(json["extruder"]["targets"], json!([200.0, 200.0]));
        assert_eq!(json["temperature_sensor host"].get("targets"), None);
    }
}
