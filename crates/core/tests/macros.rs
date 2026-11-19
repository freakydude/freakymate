//! The macro reader against real public configurations (`tests/fixtures/klipper-configs/`, source and commit at the top
//! of each file): the section names and templates go through a small reader that follows Klipper's `configfile.py`
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

#[test]
fn reads_the_macros_of_klippers_sample_macros() {
    let printer = Printer::from_config("klipper-sample-macros.cfg");

    assert_eq!(
        printer.sources.klipper.macros,
        ["END_PRINT", "G130", "M117", "M300", "M486", "M600", "M808", "QUERY_BME280", "QUERY_HTU21D", "START_PRINT"]
    );
    assert_eq!(printer.params("START_PRINT"), ["BED_TEMP=60", "EXTRUDER_TEMP=190"]);
    assert_eq!(printer.params("M300"), ["S=1000", "P=100"]);
    assert_eq!(printer.params("M600"), ["X=50", "Y=0", "Z=10"]);
    assert!(printer.params("END_PRINT").is_empty());
}

#[test]
fn parameters_tested_with_in_params_are_found() {
    let printer = Printer::from_config("klipper-sample-macros.cfg");

    // M486 tests `'T' in params` and uses `params.P`, `params.S`, `params.U` and `params.C`/`params.T`.
    let names: Vec<String> =
        printer.params("M486").iter().map(|param| param.split('=').next().unwrap().to_owned()).collect();
    for expected in ["T", "C", "P", "S", "U"] {
        assert!(names.iter().any(|name| name == expected), "M486 lacks {expected}: {names:?}");
    }
    // M808 combines `is defined` and `is not defined`.
    assert_eq!(printer.params("M808"), ["K", "L"]);
}

#[test]
fn a_macro_that_overrides_a_command_is_not_offered() {
    let printer = Printer::from_config("klipper-sample-macros.cfg");

    assert!(printer.details()["M117"].renamed);
    assert!(!printer.offered().contains(&"M117".to_owned()));
    assert!(printer.offered().contains(&"M300".to_owned()));
}

#[test]
fn reads_the_macros_of_the_voron_two_configuration() {
    let printer = Printer::from_config("voron2-octopus.cfg");

    assert_eq!(printer.sources.klipper.macros, ["G32", "PARK", "PRINT_END", "PRINT_START"]);
    assert!(printer.params("PRINT_START").is_empty());
    // Sections that are no macros do not become macros.
    assert!(!printer.details().contains_key("STEPPER_X"));
}

#[test]
fn reads_the_macros_of_mainsail_config() {
    let printer = Printer::from_config("mainsail-client.cfg");

    for name in
        ["CANCEL_PRINT", "PAUSE", "RESUME", "SET_PAUSE_AT_LAYER", "SET_PAUSE_NEXT_LAYER", "SET_PRINT_STATS_INFO"]
    {
        assert!(printer.sources.klipper.has_macro(name), "{name} missing");
    }
    // These replace commands of Klipper and are not offered; the helpers (`_...`) are no objects the UI lists.
    let offered = printer.offered();
    for renamed in ["CANCEL_PRINT", "PAUSE", "RESUME", "SET_PRINT_STATS_INFO"] {
        assert!(!offered.contains(&renamed.to_owned()), "{renamed} is offered");
    }
    assert!(offered.contains(&"SET_PAUSE_AT_LAYER".to_owned()));
    assert_eq!(printer.params("RESUME"), ["VELOCITY"]);
    assert_eq!(printer.params("SET_PAUSE_AT_LAYER"), ["ENABLE", "LAYER", "MACRO"]);
    assert_eq!(printer.params("SET_PRINT_STATS_INFO"), ["CURRENT_LAYER"]);
}

#[test]
fn reads_the_macros_of_a_macro_collection() {
    let printer = Printer::from_config("jschuh-filament.cfg");

    assert!(printer.sources.klipper.has_macro("LOAD_FILAMENT"));
    assert!(printer.sources.klipper.has_macro("UNLOAD_FILAMENT"));
    // The names of the collection are lower case in the file; Klipper's commands are not case sensitive.
    assert!(printer.sources.klipper.has_macro("M701"));
    assert!(printer.sources.klipper.has_macro("M702"));
    // `L` and `U` stand in the template (first use decides the order); `Z` and `T` of the usage line are passed on by
    // `{% for k in params %}`, which no scan of the template can see.
    assert_eq!(printer.params("M701"), ["U", "L"]);
    assert_eq!(printer.params("M702"), ["L", "U"]);
}

#[test]
fn a_form_for_a_real_macro_builds_the_command() {
    let printer = Printer::from_config("klipper-sample-macros.cfg");
    let mut form = MacroForm::open("START_PRINT", printer.details().get("START_PRINT"));

    form.set_value(0, "65").unwrap();

    assert_eq!(form.fields().iter().map(|field| field.placeholder.as_str()).collect::<Vec<_>>(), ["60", "190"]);
    assert_eq!(form.script(), "START_PRINT BED_TEMP=65");
}
