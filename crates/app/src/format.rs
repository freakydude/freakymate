//! Display text formats (durations, lengths, weights, time of day, names). Every quantity the UI shows goes through
//! here; the functions below name each kind of value and how it fits its place (width, precision, unit).

mod duration;
mod quantity;

use chrono::{DateTime, Local, TimeZone};
use duration::{MINUTE, SECOND, duration_in};
use quantity::{Fit, LENGTH, WEIGHT, quantity, quantity_pair};

/// Print times and estimates to the second: `45 s`, `12 min 05 s`, `2 h 07 min`, `1 d 03 h`.
pub fn duration(seconds: f64) -> String {
    duration_in(seconds, &SECOND)
}

/// Whole minutes, for the time axis of a chart (`5 min`, `1 h 30 min`).
pub fn duration_minutes(seconds: f64) -> String {
    duration_in(seconds, &MINUTE)
}

/// Filament of a print: tenths of a metre show the progress (`0.4 m`, `12.3 m`).
const FILAMENT_LENGTH: Fit = Fit::width(4, 100.0);

/// Filament length of a print, e.g. `12.3 m`.
pub fn filament_length(millimetres: f64) -> String {
    quantity(millimetres.max(0.0), LENGTH, FILAMENT_LENGTH)
}

/// Used of planned filament in one unit, e.g. `1.9 / 5.2 m`.
pub fn filament_progress(used_millimetres: f64, total_millimetres: f64) -> String {
    quantity_pair(used_millimetres.max(0.0), total_millimetres.max(0.0), LENGTH, FILAMENT_LENGTH)
}

/// Filament weight: a decimal only below 10 g.
const FILAMENT_WEIGHT: Fit = Fit { width: 4, significant: 2, resolution: 0.1 };

/// Filament weight; a decimal only below 10 g (`4.6 g`, `742 g`, `12 kg`).
pub fn filament_weight(grams: f64) -> String {
    quantity(grams.max(0.0), WEIGHT, FILAMENT_WEIGHT)
}

/// Filament weight a print used of what its file plans, e.g. `12 / 54 g`.
pub fn filament_weight_progress(used_grams: f64, planned_grams: f64) -> String {
    quantity_pair(used_grams.max(0.0), planned_grams.max(0.0), WEIGHT, FILAMENT_WEIGHT)
}

/// Where the filament is on its path: to a tenth of a millimetre while it fits, metres once printing adds up
/// (`412.6 mm`, `4821 mm`, `48.21 m`).
pub fn path_distance(millimetres: f64) -> String {
    quantity(millimetres, LENGTH, Fit::width(4, 0.1))
}

/// Heights of the print: each layer step shows (`0.28 mm`), above 100 mm one decimal is left (`245.0 mm`).
const HEIGHT: Fit = Fit::width(4, 0.01);

pub fn height(millimetres: f64) -> String {
    quantity(millimetres, LENGTH, HEIGHT)
}

/// Current of object height, e.g. `0.20 / 245.0 mm`.
pub fn height_of(current_millimetres: f64, object_millimetres: f64) -> String {
    quantity_pair(current_millimetres, object_millimetres, LENGTH, HEIGHT)
}

/// What the encoder leaves of the length it watches, e.g. `7.5 / 12.0 mm`.
pub fn encoder_headroom(headroom_millimetres: f64, detection_millimetres: f64) -> String {
    quantity_pair(headroom_millimetres, detection_millimetres, LENGTH, Fit::width(3, 0.1))
}

/// Deviation of a bed mesh to the micrometre, e.g. `0.123 mm`.
pub fn mesh_deviation(millimetres: f64) -> String {
    quantity(millimetres, LENGTH, Fit::width(4, 0.001))
}

/// A nozzle diameter as given (`0.4 mm`, `0.25 mm`).
pub fn nozzle_diameter(millimetres: f64) -> String {
    format!("{millimetres} mm")
}

// Temperatures follow `units.slint` of the UI, which formats the live values itself.

/// `215 °C`, or to tenths for a measured value (`214.8 °C`).
pub fn temperature(celsius: f64, decimals: usize) -> String {
    format!("{celsius:.decimals$} °C")
}

/// Measured and target temperature in one unit, the target in whole degrees: `210.3 / 215 °C`.
pub fn temperature_of(celsius: f64, target_celsius: f64) -> String {
    format!("{celsius:.1} / {target_celsius:.0} °C")
}

/// A range in whole degrees: `180–300 °C`.
pub fn temperature_range(low_celsius: f64, high_celsius: f64) -> String {
    format!("{low_celsius:.0}–{high_celsius:.0} °C")
}

/// Compact whole degrees where the context says Celsius (file facts, hints): `215°`.
pub fn degrees(celsius: f64) -> String {
    format!("{celsius:.0}°")
}

pub fn clock(time: DateTime<Local>) -> String {
    time.format("%H:%M").to_string()
}

/// Time of day at which a remaining duration ends; with date if it is no longer today.
pub fn finish_time(now: DateTime<Local>, remaining_seconds: f64) -> String {
    moment(now, now + chrono::Duration::seconds(remaining_seconds.max(0.0).round() as i64))
}

/// Time of day of a moment given in Unix seconds; with date if it is not today.
pub fn unix_moment(now: DateTime<Local>, unix_seconds: f64) -> String {
    Local.timestamp_opt(unix_seconds as i64, 0).single().map_or_else(String::new, |time| moment(now, time))
}

fn moment(now: DateTime<Local>, time: DateTime<Local>) -> String {
    if time.date_naive() == now.date_naive() { clock(time) } else { time.format("%d.%m. %H:%M").to_string() }
}

pub fn unix_clock(unix_seconds: f64) -> String {
    Local
        .timestamp_opt(unix_seconds as i64, 0)
        .single()
        .map_or_else(String::new, |time| time.format("%H:%M:%S").to_string())
}

pub fn unix_date(unix_seconds: f64) -> String {
    Local
        .timestamp_opt(unix_seconds as i64, 0)
        .single()
        .map_or_else(String::new, |time| time.format("%d.%m.%Y %H:%M").to_string())
}

/// Display name of a G-code file: without folder and extension.
pub fn job_name(path: &str) -> String {
    let file = path.rsplit('/').next().unwrap_or(path);
    let lower = file.to_ascii_lowercase();
    [".gcode", ".gco", ".g"]
        .iter()
        .find(|extension| lower.ends_with(*extension))
        .map_or(file, |extension| &file[..file.len() - extension.len()])
        .to_owned()
}

/// Display name of a service or update-manager entry (`klipper` → `Klipper`): first letter upper case, the rest as is.
pub fn service_name(name: &str) -> String {
    let mut chars = name.chars();
    chars.next().map_or_else(String::new, |first| first.to_uppercase().chain(chars).collect())
}

/// A certificate fingerprint (`AB:CD:…`, 32 pairs) as lines of 8 pairs, so all of it is readable on a small display.
pub fn fingerprint_lines(fingerprint: &str) -> String {
    let pairs: Vec<&str> = fingerprint.split(':').collect();
    pairs.chunks(8).map(|line| line.join(":")).collect::<Vec<_>>().join("\n")
}

/// A certificate fingerprint as its first and last four pairs, to recognise it (`3F:A1:9C:07 … 5E:22:B0:4D`).
pub fn fingerprint_short(fingerprint: &str) -> String {
    let pairs: Vec<&str> = fingerprint.split(':').collect();
    if pairs.len() <= 8 {
        return fingerprint.to_owned();
    }
    format!("{} … {}", pairs[..4].join(":"), pairs[pairs.len() - 4..].join(":"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_fingerprint() -> String {
        (0..32).map(|index| format!("{:02X}", index * 7 + 3)).collect::<Vec<_>>().join(":")
    }

    #[test]
    fn a_fingerprint_is_split_into_four_lines_of_eight_pairs() {
        let lines = fingerprint_lines(&sample_fingerprint());

        assert_eq!(lines.lines().count(), 4);
        assert!(lines.lines().all(|line| line.split(':').count() == 8));
        assert_eq!(lines.replace('\n', ":"), sample_fingerprint());
        assert_eq!(lines.lines().next(), Some("03:0A:11:18:1F:26:2D:34"));
    }

    #[test]
    fn a_short_fingerprint_keeps_the_first_and_last_four_pairs() {
        assert_eq!(fingerprint_short(&sample_fingerprint()), "03:0A:11:18 … C7:CE:D5:DC");
        assert_eq!(fingerprint_short("AB:CD"), "AB:CD");
    }

    #[test]
    fn temperatures_share_one_unit() {
        assert_eq!(temperature(214.84, 1), "214.8 °C");
        assert_eq!(temperature(215.0, 0), "215 °C");
        assert_eq!(temperature_of(210.34, 215.0), "210.3 / 215 °C");
        assert_eq!(temperature_range(0.0, 300.0), "0–300 °C");
        assert_eq!(degrees(59.6), "60°");
    }

    #[test]
    fn durations_to_the_second_or_minute() {
        assert_eq!(duration(725.0), "12 min 05 s");
        assert_eq!(duration_minutes(1_800.0), "30 min");
    }

    #[test]
    fn finish_time_adds_date_only_on_other_days() {
        let now = Local.with_ymd_and_hms(2026, 10, 21, 10, 0, 0).unwrap();

        assert_eq!(finish_time(now, 5_400.0), "11:30");
        assert_eq!(finish_time(now, 20.0 * 3600.0), "22.10. 06:00");
    }

    #[test]
    fn a_past_moment_carries_its_date_unless_it_is_today() {
        let now = Local.with_ymd_and_hms(2026, 10, 21, 10, 0, 0).unwrap();
        let earlier_today = Local.with_ymd_and_hms(2026, 10, 21, 8, 15, 0).unwrap().timestamp() as f64;
        let yesterday = Local.with_ymd_and_hms(2026, 10, 20, 23, 5, 0).unwrap().timestamp() as f64;

        assert_eq!(unix_moment(now, earlier_today), "08:15");
        assert_eq!(unix_moment(now, yesterday), "20.10. 23:05");
    }

    #[test]
    fn service_names_start_with_a_capital() {
        assert_eq!(service_name("klipper"), "Klipper");
        assert_eq!(service_name("crowsnest"), "Crowsnest");
        assert_eq!(service_name("KlipperScreen"), "KlipperScreen");
        assert_eq!(service_name(""), "");
    }

    #[test]
    fn lengths_weights_and_names() {
        assert_eq!(filament_length(1234.0), "1.2 m");
        assert_eq!(filament_progress(1_920.0, 5_200.0), "1.9 / 5.2 m");
        assert_eq!(filament_weight(4.56), "4.6 g");
        assert_eq!(filament_weight(23.4), "23 g");
        assert_eq!(path_distance(48_213.7), "48.21 m");
        assert_eq!(height_of(0.2, 245.0), "0.20 / 245.0 mm");
        assert_eq!(encoder_headroom(7.5, 12.0), "7.5 / 12.0 mm");
        assert_eq!(mesh_deviation(0.1234), "0.123 mm");
        assert_eq!(nozzle_diameter(0.25), "0.25 mm");
        assert_eq!(job_name("parts/Voron Cube.gcode"), "Voron Cube");
        assert_eq!(job_name("plain"), "plain");
    }
}
