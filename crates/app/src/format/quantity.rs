//! Lengths and weights in the unit that fits their place: the smallest unit whose number still fits the width, with
//! as many decimals as the width, the needed precision and the information content allow.

/// How a quantity fits the place it is shown in.
#[derive(Clone, Copy, Debug)]
pub struct Fit {
    /// Digits the place has room for, decimals included; the unit changes only when the number would not fit.
    pub width: u32,
    /// Significant digits worth showing; further decimals would only be noise (`23 g`, not `23.4 g`).
    pub significant: u32,
    /// The finest step that matters, in the base unit (millimetres, grams). Also no unit finer than this step is used,
    /// so a resolution of 100 mm shows metres from the start (`0.4 m`).
    pub resolution: f64,
}

impl Fit {
    /// Fills the width with as many digits as the resolution asks for.
    pub const fn width(width: u32, resolution: f64) -> Self {
        Self { width, significant: width, resolution }
    }
}

/// A unit of a quantity and its size in the base unit.
pub struct Unit {
    pub symbol: &'static str,
    pub factor: f64,
}

/// Lengths in millimetres; centimetres are not used in 3D printing.
pub const LENGTH: &[Unit] =
    &[Unit { symbol: "mm", factor: 1.0 }, Unit { symbol: "m", factor: 1e3 }, Unit { symbol: "km", factor: 1e6 }];

/// Weights in grams.
pub const WEIGHT: &[Unit] = &[Unit { symbol: "g", factor: 1.0 }, Unit { symbol: "kg", factor: 1e3 }];

/// The value in the unit that fits, e.g. `412.6 mm` or `48.21 m`.
pub fn quantity(value: f64, units: &[Unit], fit: Fit) -> String {
    let (unit, number) = scaled(value, units, fit);
    format!("{number} {}", unit.symbol)
}

/// Two values in one unit, the one the larger needs; each keeps its own decimals (`0.20 / 245.0 mm`).
pub fn quantity_pair(first: f64, second: f64, units: &[Unit], fit: Fit) -> String {
    let (unit, _) = scaled(first.abs().max(second.abs()), units, fit);
    let number = |value: f64| number_in(value, unit, fit).unwrap_or_else(|| fixed(value / unit.factor, 0));
    format!("{} / {} {}", number(first), number(second), unit.symbol)
}

/// The first unit the value fits in, and its number; the largest unit takes what fits nowhere.
fn scaled(value: f64, units: &[Unit], fit: Fit) -> (&Unit, String) {
    let usable = usable_units(units, fit.resolution);
    let largest = usable.last().expect("a quantity has at least one unit");
    usable
        .iter()
        .find_map(|unit| number_in(value, unit, fit).map(|number| (unit, number)))
        .unwrap_or_else(|| (largest, fixed(value / largest.factor, 0)))
}

/// Units not finer than the resolution; the largest unit if all are.
fn usable_units(units: &[Unit], resolution: f64) -> &[Unit] {
    let first = units.iter().position(|unit| unit.factor >= resolution).unwrap_or(units.len() - 1);
    &units[first..]
}

/// The number of the value in this unit, if it fits the width.
fn number_in(value: f64, unit: &Unit, fit: Fit) -> Option<String> {
    let scaled = value / unit.factor;
    (0..=decimals(scaled, unit, fit))
        .rev()
        .map(|decimals| fixed(scaled, decimals))
        .find(|text| digits(text) <= fit.width)
}

/// The most decimals the resolution, the width and the significant digits allow.
fn decimals(scaled: f64, unit: &Unit, fit: Fit) -> u32 {
    let integer_digits = digits(&fixed(scaled.trunc(), 0));
    let leading_digits = if scaled.abs() < 1.0 { 0 } else { integer_digits };
    let for_resolution = (-(fit.resolution / unit.factor).log10() - 1e-9).ceil().max(0.0) as u32;
    for_resolution.min(fit.width.saturating_sub(integer_digits)).min(fit.significant.saturating_sub(leading_digits))
}

fn fixed(value: f64, decimals: u32) -> String {
    let text = format!("{value:.0$}", decimals as usize);
    // `-0.0` rounds to `-0`; a value that small is no direction.
    if text.trim_start_matches(['-', '0', '.']).is_empty() { text.trim_start_matches('-').to_owned() } else { text }
}

fn digits(text: &str) -> u32 {
    text.chars().filter(char::is_ascii_digit).count() as u32
}

#[cfg(test)]
mod tests {
    use super::*;

    const PATH: Fit = Fit::width(4, 0.1);
    const PROGRESS: Fit = Fit::width(4, 100.0);
    const GRAMS: Fit = Fit { width: 4, significant: 2, resolution: 0.1 };

    #[test]
    fn a_length_keeps_millimetres_while_they_fit_the_width() {
        assert_eq!(quantity(412.63, LENGTH, PATH), "412.6 mm");
        assert_eq!(quantity(4_821.4, LENGTH, PATH), "4821 mm");
        assert_eq!(quantity(48_213.7, LENGTH, PATH), "48.21 m");
        assert_eq!(quantity(123_456.0, LENGTH, PATH), "123.5 m");
    }

    #[test]
    fn rounding_up_to_a_wider_number_moves_to_the_next_unit() {
        assert_eq!(quantity(9_999.6, LENGTH, PATH), "10.00 m");
        assert_eq!(quantity(999.96, LENGTH, PATH), "1000 mm");
    }

    #[test]
    fn a_coarse_resolution_skips_finer_units() {
        assert_eq!(quantity(412.0, LENGTH, PROGRESS), "0.4 m");
        assert_eq!(quantity(12_345.0, LENGTH, PROGRESS), "12.3 m");
        assert_eq!(quantity(1_234_567.0, LENGTH, PROGRESS), "1235 m");
        assert_eq!(quantity(12_345_678.0, LENGTH, PROGRESS), "12.35 km");
    }

    #[test]
    fn significant_digits_drop_decimals_that_are_only_noise() {
        assert_eq!(quantity(0.43, WEIGHT, GRAMS), "0.4 g");
        assert_eq!(quantity(4.56, WEIGHT, GRAMS), "4.6 g");
        assert_eq!(quantity(23.4, WEIGHT, GRAMS), "23 g");
        assert_eq!(quantity(1_000.0, WEIGHT, GRAMS), "1000 g");
        assert_eq!(quantity(12_400.0, WEIGHT, GRAMS), "12 kg");
    }

    #[test]
    fn a_value_beyond_every_unit_overflows_the_largest() {
        assert_eq!(quantity(5e7, WEIGHT, GRAMS), "50000 kg");
    }

    #[test]
    fn negative_values_keep_their_sign_and_a_rounded_zero_loses_it() {
        assert_eq!(quantity(-12.34, LENGTH, PATH), "-12.3 mm");
        assert_eq!(quantity(-0.01, LENGTH, PATH), "0.0 mm");
    }

    #[test]
    fn a_pair_shares_the_unit_of_the_larger_value() {
        assert_eq!(quantity_pair(412.0, 5_200.0, LENGTH, PROGRESS), "0.4 / 5.2 m");
        assert_eq!(quantity_pair(0.2, 245.0, LENGTH, Fit::width(4, 0.01)), "0.20 / 245.0 mm");
        assert_eq!(quantity_pair(7.5, 12.0, LENGTH, Fit::width(3, 0.1)), "7.5 / 12.0 mm");
    }
}
