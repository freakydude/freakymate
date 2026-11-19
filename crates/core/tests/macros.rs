//! The macro reader against a configuration written for these tests (`tests/fixtures/klipper-configs/`): the
//! section names and templates go through a small reader that follows Klipper's `configfile.py`
//! (`ConfigFileReader`, Python's `RawConfigParser`), then through the same steps as the service.

use sailhand_core::sources::Sources;
use sailhand_core::sources::klipper::{MacroDetails, MacroForm, offered_macros};
use sailhand_core::sources::moonraker_server::server_info::ServerComponents;
use serde_json::{Map, Value, json};
use std::collections::BTreeMap;
use std::fs;
use std::path::PathBuf;

fn fixture(name: &str) -> String {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/klipper-configs").join(name);
    fs::read_to_string(&path).unwrap_or_else(|error| panic!("{}: {error}", path.display()))
}

/// Sections of a configuration as `(name, options)`: what Klipper's `RawConfigParser` makes of the text. Comments
/// start at any `#` (Klipper strips them before parsing) or at a `;` after a space; a line indented deeper than its
/// option continues the value; blank lines inside a value stay, trailing ones go; option names are lower case.
type Options = Vec<(String, String)>;
/// An option whose value is still a list of lines.
type RawSection = (String, Vec<(String, Vec<String>)>);

fn parse_config(text: &str) -> Vec<(String, Options)> {
    let mut sections: Vec<RawSection> = Vec::new();
    let mut option_indent = 0;
    for raw in text.lines() {
        let line = raw.split('#').next().unwrap_or_default();
        let line = match line
            .char_indices()
            .find(|(index, c)| *c == ';' && (*index == 0 || line[..*index].ends_with(char::is_whitespace)))
        {
            Some((index, _)) => &line[..index],
            None => line,
        };
        let value = line.trim();
        let indent = line.len() - line.trim_start().len();
        let Some((_, options)) = sections.last_mut() else {
            if let Some(name) = section_name(value) {
                sections.push((name, Vec::new()));
            }
            continue;
        };
        if value.is_empty() {
            if let Some((_, lines)) = options.last_mut() {
                lines.push(String::new());
            }
            continue;
        }
        if indent > option_indent
            && let Some((_, lines)) = options.last_mut()
        {
            lines.push(value.to_owned());
            continue;
        }
        option_indent = indent;
        if let Some(name) = section_name(value) {
            sections.push((name, Vec::new()));
        } else if let Some(position) = value.find([':', '=']) {
            options.push((
                value[..position].trim_end().to_ascii_lowercase(),
                vec![value[position + 1..].trim().to_owned()],
            ));
        }
    }
    sections
        .into_iter()
        .map(|(name, options)| {
            let options = options.into_iter().map(|(key, lines)| (key, lines.join("\n").trim_end().to_owned()));
            (name, options.collect())
        })
        .collect()
}

fn section_name(line: &str) -> Option<String> {
    line.strip_prefix('[')?.strip_suffix(']').map(str::to_owned)
}

/// What the service has after the first subscription answer: the objects of the macros (names as written in the file)
/// and `configfile.settings` (lower case section names).
struct Printer {
    sources: Sources,
}

impl Printer {
    fn from_config(name: &str) -> Self {
        let sections = parse_config(&fixture(name));
        let mut objects = Vec::new();
        let mut settings = Map::new();
        for (section, options) in sections {
            if section.starts_with("gcode_macro ") {
                objects.push(section.clone());
            }
            let options: Map<String, Value> = options.into_iter().map(|(key, value)| (key, json!(value))).collect();
            settings.insert(section.to_ascii_lowercase(), Value::Object(options));
        }
        let mut sources = Sources::detect(&objects, ServerComponents::default());
        sources.apply_settings(Some(&Value::Object(settings)));
        Self { sources }
    }

    fn details(&self) -> &BTreeMap<String, MacroDetails> {
        &self.sources.klipper.macro_details
    }

    /// The parameters of a macro as `NAME` or `NAME=default`, in the order the form shows them.
    fn params(&self, macro_name: &str) -> Vec<String> {
        let details = self.details().get(macro_name).unwrap_or_else(|| panic!("{macro_name} not read"));
        details
            .params
            .iter()
            .map(|param| match &param.default {
                Some(default) => format!("{}={default}", param.name),
                None => param.name.clone(),
            })
            .collect()
    }

    fn offered(&self) -> Vec<String> {
        offered_macros(&self.sources.klipper.macros, self.details(), &[])
            .into_iter()
            .map(|offered| offered.name)
            .collect()
    }
}

const FIXTURE: &str = "macro-patterns.cfg";

fn printer() -> Printer {
    Printer::from_config(FIXTURE)
}

#[test]
fn reads_the_macros_and_skips_other_sections() {
    let printer = printer();

    for name in ["WARM_UP", "MOVE_AWAY", "COOL_DOWN", "PICK_TOOL", "PAUSE", "RESUME"] {
        assert!(printer.sources.klipper.has_macro(name), "{name} missing");
    }
    // Sections that are no macros do not become macros.
    for other in ["STEPPER_X", "PRINTER", "REFRESH", "NOTHING"] {
        assert!(!printer.details().contains_key(other), "{other} read as macro");
    }
    // Helpers (`_...`) are no macros the UI lists.
    assert!(!printer.sources.klipper.has_macro("_HIDDEN"));
    assert!(printer.params("COOL_DOWN").is_empty());
}

#[test]
fn literal_defaults_become_placeholders_in_order_of_first_use() {
    let printer = printer();

    assert_eq!(printer.params("WARM_UP"), ["BED=60", "NOZZLE=200"]);
    assert_eq!(printer.params("MOVE_AWAY"), ["X", "Y", "SPEED=3000"]);
    assert_eq!(printer.params("RESUME"), ["VELOCITY=50"]);
}

#[test]
fn parameters_tested_with_in_params_are_found() {
    let printer = printer();

    assert_eq!(printer.params("PICK_TOOL"), ["TOOL", "SLOT", "FORCE"]);
}

#[test]
fn defined_tests_are_found() {
    assert_eq!(printer().params("CHECK_LIMITS"), ["LOW", "HIGH"]);
}

#[test]
fn method_calls_on_params_are_no_parameters() {
    // `params.get(...)` and `params.__setitem__(...)` are calls; only `params.ONE` is a parameter.
    assert_eq!(printer().params("SWAP_VALUES"), ["ONE", "TWO"]);
}

#[test]
fn a_runtime_default_gets_no_placeholder() {
    // `default(state.layer)` is no literal, and a later literal default does not belong to the parameter.
    assert_eq!(printer().params("PAUSE_AT"), ["LAYER", "CALL", "RATE"]);
}

#[test]
fn parameters_passed_on_by_a_loop_are_not_seen() {
    // Only `LEN` stands in the template; what `{% for key in params %}` passes on cannot be scanned.
    assert_eq!(printer().params("LOAD_THING"), ["LEN=50"]);
}

#[test]
fn lower_case_macro_names_are_found_in_any_case() {
    let printer = printer();

    assert!(printer.sources.klipper.has_macro("LOAD_THING"));
    assert!(printer.sources.klipper.has_macro("M701"));
    assert!(printer.sources.klipper.has_macro("M702"));
    assert_eq!(printer.params("M701"), ["U=40", "L=10"]);
    assert_eq!(printer.params("M702"), ["L=5", "U=15"]);
}

#[test]
fn a_macro_that_overrides_a_command_is_not_offered() {
    let printer = printer();
    let offered = printer.offered();

    assert!(printer.details()["PAUSE"].renamed);
    assert!(printer.details()["RESUME"].renamed);
    assert!(!offered.contains(&"PAUSE".to_owned()));
    assert!(!offered.contains(&"RESUME".to_owned()));
    assert!(offered.contains(&"WARM_UP".to_owned()));
    assert!(!printer.details()["WARM_UP"].renamed);
}

#[test]
fn a_form_for_a_macro_builds_the_command() {
    let printer = printer();
    let mut form = MacroForm::open("WARM_UP", printer.details().get("WARM_UP"));

    form.set_value(0, "65").unwrap();

    assert_eq!(form.fields().iter().map(|field| field.placeholder.as_str()).collect::<Vec<_>>(), ["60", "200"]);
    assert_eq!(form.script(), "WARM_UP BED=65");
}
