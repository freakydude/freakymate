//! Reading single fields of the printer objects (Klipper, Happy Hare); a missing or mistyped field gives the default.

use serde_json::Value;

pub(super) fn text<'a>(object: &'a Value, field: &str) -> &'a str {
    object.get(field).and_then(Value::as_str).unwrap_or_default()
}

pub(super) fn number(object: &Value, field: &str) -> f64 {
    number_or(object, field, 0.0)
}

pub(super) fn number_or(object: &Value, field: &str, default: f64) -> f64 {
    object.get(field).and_then(Value::as_f64).unwrap_or(default)
}

/// A boolean; Happy Hare also reports flags as 0 and 1.
pub(super) fn flag(object: &Value, field: &str) -> bool {
    match object.get(field) {
        Some(Value::Bool(value)) => *value,
        Some(Value::Number(value)) => value.as_i64().is_some_and(|number| number != 0),
        _ => false,
    }
}

/// A non-negative whole number that fits the target type.
pub(super) fn count<T: TryFrom<u64>>(object: &Value, field: &str) -> Option<T> {
    object.get(field).and_then(Value::as_u64).and_then(|value| T::try_from(value).ok())
}

/// A whole number that fits `i32`, or `default`.
pub(super) fn integer(object: &Value, field: &str, default: i32) -> i32 {
    object.get(field).and_then(Value::as_i64).and_then(|value| i32::try_from(value).ok()).unwrap_or(default)
}

pub(super) fn vector(object: &Value, field: &str) -> [f64; 4] {
    let mut values = [0.0; 4];
    if let Some(items) = object.get(field).and_then(Value::as_array) {
        for (slot, item) in values.iter_mut().zip(items) {
            *slot = item.as_f64().unwrap_or_default();
        }
    }
    values
}
