//! The parameters of `printer.objects.subscribe`. Every source adds its objects; only those Klipper lists are kept.

use serde_json::{json, Map, Value};

pub(in crate::sources) struct Subscription<'a> {
    available: &'a [String],
    objects: Map<String, Value>,
}

impl<'a> Subscription<'a> {
    pub(in crate::sources) fn new(available: &'a [String]) -> Self {
        Self { available, objects: Map::new() }
    }

    /// Subscribes `name` with `fields`, or with all fields for `None`, if the printer has the object.
    pub(in crate::sources) fn add(&mut self, name: &str, fields: Option<&[&str]>) {
        if self.available.iter().any(|candidate| candidate == name) {
            self.objects.insert(name.to_owned(), fields.map_or(Value::Null, |fields| json!(fields)));
        }
    }

    /// Like [`Self::add`] for an object whose name comes from the configuration (`gcode_macro <NAME>`): Klipper lists it
    /// as written there, in any case.
    pub(in crate::sources) fn add_any_case(&mut self, name: &str, fields: Option<&[&str]>) {
        if let Some(listed) = self.available.iter().find(|candidate| candidate.eq_ignore_ascii_case(name)) {
            self.add(listed, fields);
        }
    }

    /// Subscribes every object the printer lists under `prefix` (the sections `[tool <name>]`, say) with `fields`.
    pub(in crate::sources) fn add_each(&mut self, prefix: &str, fields: Option<&[&str]>) {
        for name in self.available.iter().filter(|candidate| candidate.starts_with(prefix)) {
            self.objects.insert(name.clone(), fields.map_or(Value::Null, |fields| json!(fields)));
        }
    }

    pub(in crate::sources) fn into_parameters(self) -> Value {
        json!({ "objects": self.objects })
    }
}
