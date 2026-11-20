//! Durations as the two largest units they reach (`45 s`, `12 min 05 s`, `2 h 07 min`, `1 d 03 h`).

/// A unit of time and its length in seconds, largest first.
pub struct TimeUnit {
    symbol: &'static str,
    seconds: u64,
}

pub const DAY: TimeUnit = TimeUnit { symbol: "d", seconds: 86_400 };
pub const HOUR: TimeUnit = TimeUnit { symbol: "h", seconds: 3_600 };
pub const MINUTE: TimeUnit = TimeUnit { symbol: "min", seconds: 60 };
pub const SECOND: TimeUnit = TimeUnit { symbol: "s", seconds: 1 };

const UNITS: [&TimeUnit; 4] = [&DAY, &HOUR, &MINUTE, &SECOND];

/// The duration down to whole units of `finest`; the second unit is zero-padded so a countdown keeps its width.
pub fn duration_in(seconds: f64, finest: &TimeUnit) -> String {
    let units: Vec<&TimeUnit> = UNITS.into_iter().filter(|unit| unit.seconds >= finest.seconds).collect();
    let total = (seconds.max(0.0) / finest.seconds as f64).round() as u64 * finest.seconds;
    let largest = units.iter().position(|unit| total >= unit.seconds).unwrap_or(units.len() - 1);
    let first = units[largest];
    match units.get(largest + 1) {
        Some(second) => {
            let rest = total % first.seconds / second.seconds;
            format!("{} {} {rest:02} {}", total / first.seconds, first.symbol, second.symbol)
        }
        None => format!("{} {}", total / first.seconds, first.symbol),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_duration_shows_its_two_largest_units() {
        assert_eq!(duration_in(45.4, &SECOND), "45 s");
        assert_eq!(duration_in(725.0, &SECOND), "12 min 05 s");
        assert_eq!(duration_in(7_620.0, &SECOND), "2 h 07 min");
        assert_eq!(duration_in(97_200.0, &SECOND), "1 d 03 h");
    }

    #[test]
    fn nothing_and_less_than_nothing_is_zero_of_the_finest_unit() {
        assert_eq!(duration_in(0.0, &SECOND), "0 s");
        assert_eq!(duration_in(-3.0, &SECOND), "0 s");
        assert_eq!(duration_in(20.0, &MINUTE), "0 min");
    }

    #[test]
    fn a_coarser_finest_unit_rounds_and_drops_the_finer_ones() {
        assert_eq!(duration_in(300.0, &MINUTE), "5 min");
        assert_eq!(duration_in(5_430.0, &MINUTE), "1 h 31 min");
        assert_eq!(duration_in(3_599.6, &SECOND), "1 h 00 min");
    }
}
