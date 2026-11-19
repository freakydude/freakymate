//! Values in the spelling Klipper and Moonraker read: booleans like Python's `configparser`, numbers like `float()`,
//! lists separated by commas or one entry per line.

use std::str::FromStr;

/// How a list is written: `a, b` on the line of the key, or one entry per indented line below it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ListLayout {
    Inline,
    Lines,
}

/// The spellings of `configparser.RawConfigParser.BOOLEAN_STATES`, compared without case.
pub(crate) fn parse_bool(text: &str) -> Result<bool, String> {
    match text.to_ascii_lowercase().as_str() {
        "1" | "yes" | "true" | "on" => Ok(true),
        "0" | "no" | "false" | "off" => Ok(false),
        _ => Err(format!("`{text}` is not a boolean (True or False)")),
    }
}

/// The spelling Klipper writes.
pub(crate) fn format_bool(value: &bool) -> String {
    if *value { "True" } else { "False" }.to_owned()
}

pub(crate) fn parse_number<T: FromStr>(text: &str) -> Result<T, String> {
    text.parse().map_err(|_| format!("`{text}` is not a valid number"))
}

/// Whole numbers without a decimal point (`210`, not `210.0`), like a person writes them.
pub(crate) fn format_number<T: ToString>(value: &T) -> String {
    value.to_string()
}

pub(crate) fn parse_text(text: &str) -> Result<String, String> {
    Ok(text.to_owned())
}

/// Entries separated by commas or line breaks, without surrounding spaces and empty entries.
pub(crate) fn parse_list(text: &str) -> Vec<String> {
    text.split([',', '\n']).map(str::trim).filter(|entry| !entry.is_empty()).map(str::to_owned).collect()
}

pub(crate) fn parse_number_list<T: FromStr>(text: &str) -> Result<Vec<T>, String> {
    parse_list(text).iter().map(|entry| parse_number(entry)).collect()
}

pub(crate) fn format_list<T: ToString>(entries: &[T], layout: ListLayout) -> String {
    let separator = match layout {
        ListLayout::Inline => ", ",
        ListLayout::Lines => "\n",
    };
    entries.iter().map(ToString::to_string).collect::<Vec<_>>().join(separator)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn booleans_are_read_in_every_configparser_spelling_and_written_like_klipper() {
        for text in ["True", "true", "YES", "on", "1"] {
            assert_eq!(parse_bool(text), Ok(true), "{text}");
        }
        for text in ["False", "no", "Off", "0"] {
            assert_eq!(parse_bool(text), Ok(false), "{text}");
        }
        assert!(parse_bool("tru").is_err());
        assert_eq!(format_bool(&true), "True");
    }

    #[test]
    fn numbers_are_written_without_a_needless_decimal_point() {
        assert_eq!(format_number(&210.0), "210");
        assert_eq!(format_number(&0.025), "0.025");
        assert_eq!(parse_number::<f64>("210.0"), Ok(210.0));
        assert!(parse_number::<u8>("300").is_err());
    }

    #[test]
    fn lists_are_read_with_commas_or_lines() {
        assert_eq!(parse_list("PARK, LOAD_FILAMENT,"), ["PARK", "LOAD_FILAMENT"]);
        assert_eq!(parse_list("\nPARK\n  LOAD_FILAMENT\n"), ["PARK", "LOAD_FILAMENT"]);
        assert_eq!(parse_number_list::<f64>("0.1, 1, 10"), Ok(vec![0.1, 1.0, 10.0]));
        assert!(parse_number_list::<f64>("1, x").is_err());
    }

    #[test]
    fn lists_are_written_inline_or_one_per_line() {
        assert_eq!(format_list(&[0.1, 1.0, 10.0], ListLayout::Inline), "0.1, 1, 10");
        assert_eq!(format_list(&["PARK", "LOAD"], ListLayout::Lines), "PARK\nLOAD");
    }
}
