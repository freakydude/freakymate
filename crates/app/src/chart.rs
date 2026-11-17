//! The temperature chart: lines as SVG path commands, and the scale and grid around them. Free of Slint types and
//! therefore testable. The view box is 1000 high and as wide as the chart's aspect ratio makes it: Slint scales a
//! path evenly, so a box of another shape would squeeze the chart into the middle.

use sailhand_core::sources::moonraker_server::temperature_store::HISTORY_LENGTH;

/// Height of the view box; its width is this times the aspect ratio of the chart.
pub const VIEW_HEIGHT: f64 = 1000.0;
/// Points per line at most; more than the chart has pixels would only cost time on the Pi.
const MAX_POINTS: usize = 400;
/// The scale is rounded outwards to this many degrees.
const SCALE_STEP: f64 = 10.0;
/// At least this span, so the noise of a sensor (a degree or so) does not fill the chart.
const MIN_SPAN: f64 = 30.0;
/// The window of the chart: the whole history Moonraker keeps, as the dialogs show it.
pub const FULL_WINDOW_SECONDS: usize = HISTORY_LENGTH;
/// A short window for a quick process, the PID tuning, whose swings 20 minutes would squeeze.
pub const SHORT_WINDOW_SECONDS: usize = 300;
/// Vertical grid lines every five minutes, in a window up to ten minutes every minute.
const TIME_STEP_SECONDS: usize = 300;
const SHORT_TIME_STEP_SECONDS: usize = 60;
const SHORT_WINDOW_LIMIT_SECONDS: usize = 600;
/// Heater power takes the lowest quarter of the chart.
const POWER_HEIGHT: f64 = 0.25;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ChartScale {
    pub low: f64,
    pub high: f64,
}

/// The newest `window` seconds of a series, one value per second.
pub fn visible(values: &[f64], window: usize) -> &[f64] {
    &values[values.len().saturating_sub(window)..]
}

/// From the lowest to the highest value of both lines, rounded outwards to 10 °C.
pub fn scale(temperatures: &[f64], targets: &[f64]) -> ChartScale {
    let values = temperatures.iter().chain(targets.iter().filter(|target| **target > 0.0)).copied();
    let (low, high) =
        values.fold((f64::INFINITY, f64::NEG_INFINITY), |(low, high), value| (low.min(value), high.max(value)));
    if !low.is_finite() {
        return ChartScale { low: 0.0, high: 100.0 };
    }
    // Short of the minimum span, the values stay in the middle.
    let missing = (MIN_SPAN - (high - low)).max(0.0) / 2.0;
    let low = ((low - missing) / SCALE_STEP).floor() * SCALE_STEP;
    let high = ((high + missing) / SCALE_STEP).ceil() * SCALE_STEP;
    ChartScale { low: low.max(0.0).min(high - SCALE_STEP), high }
}

/// Horizontal lines at round temperatures and vertical lines every five minutes (every minute in a short window), with their labels: the text and
/// the share of the height from the top or of the width from the left.
pub struct Grid {
    pub path: String,
    pub temperatures: Vec<(String, f32)>,
    pub times: Vec<(String, f32)>,
}

pub fn grid(scale: ChartScale, aspect: f64, window: usize) -> Grid {
    let span = scale.high - scale.low;
    let width = VIEW_HEIGHT * aspect;
    let step = [10.0, 20.0, 25.0, 50.0, 100.0].into_iter().find(|step| span / step <= 6.0).unwrap_or(100.0);
    let mut commands = Vec::new();
    let mut temperatures = Vec::new();
    let mut degrees = (scale.low / step).ceil() * step;
    while degrees <= scale.high + f64::EPSILON {
        let share = (scale.high - degrees) / span;
        let y = share * VIEW_HEIGHT;
        commands.push(format!("M 0 {y:.1} L {width:.1} {y:.1}"));
        temperatures.push((format!("{degrees:.0} °C"), share as f32));
        degrees += step;
    }
    let mut times = Vec::new();
    let time_step = if window <= SHORT_WINDOW_LIMIT_SECONDS { SHORT_TIME_STEP_SECONDS } else { TIME_STEP_SECONDS };
    let mut seconds = time_step;
    while seconds < window {
        let share = 1.0 - seconds as f64 / (window - 1) as f64;
        let x = share * width;
        commands.push(format!("M {x:.1} 0 L {x:.1} {VIEW_HEIGHT}"));
        times.push((format!("−{} min", seconds / 60), share as f32));
        seconds += time_step;
    }
    Grid { path: commands.join(" "), temperatures, times }
}

/// A line through the newest `window` seconds of the values, the newest at the right edge; a shorter history fills
/// only the right part.
pub fn line(values: &[f64], scale: ChartScale, aspect: f64, window: usize) -> String {
    let span = (scale.high - scale.low).max(f64::EPSILON);
    path(visible(values, window), aspect, window, |value| {
        VIEW_HEIGHT - ((value - scale.low) / span).clamp(0.0, 1.0) * VIEW_HEIGHT
    })
}

/// Heater power from 0 to 1 along the bottom of the chart.
pub fn power_line(powers: &[f64], aspect: f64, window: usize) -> String {
    path(visible(powers, window), aspect, window, |power| {
        VIEW_HEIGHT - power.clamp(0.0, 1.0) * POWER_HEIGHT * VIEW_HEIGHT
    })
}

fn path(values: &[f64], aspect: f64, window: usize, to_y: impl Fn(f64) -> f64) -> String {
    if values.len() < 2 {
        return String::new();
    }
    let stride = values.len().div_ceil(MAX_POINTS);
    let last = values.len() - 1;
    let width = VIEW_HEIGHT * aspect;
    let x_of = |index: usize| width - (last - index) as f64 / (window - 1) as f64 * width;
    let mut indices: Vec<usize> = (0..values.len()).step_by(stride).collect();
    if indices.last() != Some(&last) {
        indices.push(last);
    }
    indices
        .iter()
        .enumerate()
        .map(|(position, &index)| {
            let command = if position == 0 { 'M' } else { 'L' };
            format!("{command} {:.1} {:.1}", x_of(index).max(0.0), to_y(values[index]))
        })
        .collect::<Vec<_>>()
        .join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_scale_covers_both_lines_in_steps_of_ten() {
        assert_eq!(scale(&[21.5, 180.2], &[0.0, 200.0]), ChartScale { low: 20.0, high: 200.0 });
        assert_eq!(scale(&[36.0, 37.0], &[]), ChartScale { low: 20.0, high: 60.0 }, "noise stays small in the middle");
        assert_eq!(scale(&[], &[]), ChartScale { low: 0.0, high: 100.0 });
    }

    #[test]
    fn the_grid_has_round_temperatures_and_five_minute_marks() {
        let grid = grid(ChartScale { low: 20.0, high: 220.0 }, 2.0, FULL_WINDOW_SECONDS);
        assert!(grid.path.starts_with("M 0 850.0 L 2000.0 850.0"), "lines span the whole width");
        let texts: Vec<&str> = grid.temperatures.iter().map(|(text, _)| text.as_str()).collect();
        assert_eq!(texts, ["50 °C", "100 °C", "150 °C", "200 °C"]);
        assert_eq!(grid.temperatures[3].1, 0.1, "200 °C is a tenth below the top");
        let times: Vec<&str> = grid.times.iter().map(|(text, _)| text.as_str()).collect();
        assert_eq!(times, ["−5 min", "−10 min", "−15 min"]);
    }

    #[test]
    fn a_line_ends_at_the_right_edge_and_scales_its_values() {
        let path = line(&[20.0, 120.0], ChartScale { low: 20.0, high: 220.0 }, 2.0, FULL_WINDOW_SECONDS);
        let x_before = 2000.0 - 2000.0 / (HISTORY_LENGTH - 1) as f64;

        assert_eq!(path, format!("M {x_before:.1} 1000.0 L 2000.0 500.0"), "the newest point is at the right edge");
        assert_eq!(
            line(&[20.0], ChartScale { low: 0.0, high: 100.0 }, 2.0, FULL_WINDOW_SECONDS),
            "",
            "one point is no line"
        );
    }

    #[test]
    fn a_long_history_is_thinned_out_but_keeps_its_newest_point() {
        let values: Vec<f64> = (0..HISTORY_LENGTH).map(|second| second as f64).collect();
        let path = line(&values, ChartScale { low: 0.0, high: HISTORY_LENGTH as f64 }, 1.0, FULL_WINDOW_SECONDS);

        assert!(path.split(" ").filter(|word| *word == "L").count() <= MAX_POINTS);
        assert!(path.starts_with("M 0.0 1000.0"));
        assert!(path.ends_with("L 1000.0 0.8"));
    }

    #[test]
    fn a_short_window_shows_the_newest_seconds_with_minute_marks() {
        let values: Vec<f64> = (0..HISTORY_LENGTH).map(|second| second as f64).collect();
        assert_eq!(visible(&values, SHORT_WINDOW_SECONDS).len(), SHORT_WINDOW_SECONDS);
        assert_eq!(visible(&values, SHORT_WINDOW_SECONDS)[0], (HISTORY_LENGTH - SHORT_WINDOW_SECONDS) as f64);
        assert_eq!(visible(&values[..10], SHORT_WINDOW_SECONDS).len(), 10, "less history than the window");

        let scale = ChartScale { low: 0.0, high: HISTORY_LENGTH as f64 };
        let path = line(&values, scale, 1.0, SHORT_WINDOW_SECONDS);
        assert!(path.starts_with("M 0.0 "), "the oldest shown second is at the left edge");
        assert!(path.contains("L 1000.0 "), "the newest at the right edge");

        let times: Vec<String> =
            grid(scale, 1.0, SHORT_WINDOW_SECONDS).times.into_iter().map(|(text, _)| text).collect();
        assert_eq!(times, ["−1 min", "−2 min", "−3 min", "−4 min"]);
    }

    #[test]
    fn power_stays_in_the_bottom_quarter() {
        assert_eq!(power_line(&[0.0, 1.0], 2.0, FULL_WINDOW_SECONDS).rsplit(' ').next(), Some("750.0"));
    }
}
