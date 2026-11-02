//! Brightness and standby of a monitor on HDMI or DisplayPort through DDC/CI, with the program `ddcutil` (VCP code
//! `0x10` brightness, `0xD6` power mode). Offered only when `ddcutil` is installed and lists a monitor that answers.
//!
//! DDC is slow (50 to 100 ms per command and more), so the commands run on a thread of their own and the calls of
//! [`Backlight`] return at once. The brightness shown is the one last set here; the buttons of the monitor are not
//! followed. Some monitors no longer answer on DDC once switched off; then waking over DDC fails and the log says so.

use super::{percent_from_raw, raw_from_percent, Backlight, BacklightError};
use std::io;
use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::sync::Arc;

/// Prefix of the `device` setting that names a DDC/CI monitor, e.g. `ddc:i2c-7`.
pub const DEVICE_PREFIX: &str = "ddc:";

const BRIGHTNESS: &str = "10";
const POWER_MODE: &str = "d6";
/// Values of the power mode (MCCS): on, and off (DPM).
const POWER_ON: &str = "1";
const POWER_OFF: &str = "4";

/// Runs `ddcutil` with arguments and returns what it printed; replaceable, so tests need no monitor.
pub trait Runner: Send + Sync + 'static {
    fn run(&self, arguments: &[String]) -> Result<String, RunError>;
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RunError {
    NotInstalled,
    /// Started, but failed; the first line it printed.
    Failed(String),
}

/// The real `ddcutil` of this system.
pub struct Program(pub String);

impl Runner for Program {
    fn run(&self, arguments: &[String]) -> Result<String, RunError> {
        let output = Command::new(&self.0).args(arguments).env("LC_ALL", "C").stdin(Stdio::null()).output().map_err(
            |error| match error.kind() {
                io::ErrorKind::NotFound => RunError::NotInstalled,
                _ => RunError::Failed(error.to_string()),
            },
        )?;
        let text = String::from_utf8_lossy(&output.stdout).into_owned();
        if output.status.success() {
            return Ok(text);
        }
        let stderr = String::from_utf8_lossy(&output.stderr);
        let message = stderr.lines().chain(text.lines()).map(str::trim).find(|line| !line.is_empty());
        Err(RunError::Failed(message.unwrap_or("failed without a message").to_owned()))
    }
}

/// A monitor from `ddcutil detect`.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Monitor {
    bus: u32,
    /// `MANUFACTURER:MODEL:SERIAL` as ddcutil prints it.
    description: String,
}

impl Monitor {
    fn name(&self) -> String {
        format!("{DEVICE_PREFIX}i2c-{}", self.bus)
    }
}

/// What the worker thread sets.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Job {
    Brightness(u32),
    Power(bool),
}

pub struct Ddcutil {
    name: String,
    max: u32,
    /// Percent as last set, or as read at the start.
    brightness: u8,
    has_power: bool,
    /// Without a power mode, off is brightness 0: the percent to bring back.
    restore: Option<u8>,
    jobs: mpsc::Sender<Job>,
}

impl Ddcutil {
    /// Finds the monitor to control: `wanted` (`ddc:i2c-N`) or the first that answers on brightness.
    pub fn open(runner: Arc<dyn Runner>, wanted: Option<&str>) -> Result<Self, BacklightError> {
        let listing = runner.run(&arguments(["detect", "--brief"])).map_err(|error| match error {
            RunError::NotInstalled => BacklightError::NotInstalled("ddcutil".to_owned()),
            RunError::Failed(message) => BacklightError::Device { name: "ddcutil".to_owned(), message },
        })?;
        let monitors = parse_monitors(&listing);
        log::info!(
            "DDC/CI monitors: {}",
            if monitors.is_empty() {
                "none".to_owned()
            } else {
                monitors.iter().map(|m| format!("i2c-{} {}", m.bus, m.description)).collect::<Vec<_>>().join(", ")
            }
        );
        let candidates: Vec<&Monitor> = match wanted {
            Some(name) => monitors.iter().filter(|monitor| monitor.name() == name).collect(),
            None => monitors.iter().collect(),
        };
        if candidates.is_empty() {
            return Err(match wanted {
                Some(name) => BacklightError::NotFound(name.to_owned()),
                None => BacklightError::NoDdcMonitor,
            });
        }
        for monitor in candidates {
            match read_brightness(runner.as_ref(), monitor.bus) {
                Ok((value, max)) => return Ok(Self::start(runner, monitor, value, max)),
                Err(message) => log::info!("DDC/CI monitor on i2c-{} cannot be controlled: {message}", monitor.bus),
            }
        }
        Err(BacklightError::NoDdcMonitor)
    }

    fn start(runner: Arc<dyn Runner>, monitor: &Monitor, value: u32, max: u32) -> Self {
        let bus = monitor.bus;
        let has_power = runner.run(&getvcp(bus, POWER_MODE)).is_ok();
        let (jobs, queue) = mpsc::channel();
        let worker = Worker { runner, bus };
        if let Err(error) = std::thread::Builder::new().name("ddcutil".to_owned()).spawn(move || worker.run(&queue)) {
            log::warn!("cannot start the DDC/CI thread: {error}");
        }
        let how = if has_power { "off through the power mode" } else { "off through brightness 0 (no power mode)" };
        log::info!("using DDC/CI monitor {} on i2c-{bus}, {how}", monitor.description);
        Self { name: monitor.name(), max, brightness: percent_from_raw(value, max), has_power, restore: None, jobs }
    }

    fn send(&self, job: Job) -> Result<(), BacklightError> {
        self.jobs.send(job).map_err(|_| BacklightError::Device {
            name: self.name.clone(),
            message: "the DDC/CI thread has ended".to_owned(),
        })
    }
}

impl Backlight for Ddcutil {
    fn name(&self) -> &str {
        &self.name
    }

    fn brightness(&self) -> Result<u8, BacklightError> {
        Ok(self.restore.unwrap_or(self.brightness))
    }

    fn set_brightness(&mut self, percent: u8) -> Result<(), BacklightError> {
        self.brightness = percent;
        // Switched off by brightness 0: remember it for later instead of lighting up.
        if self.restore.is_some() {
            self.restore = Some(percent);
            return Ok(());
        }
        self.send(Job::Brightness(raw_from_percent(percent, self.max)))
    }

    fn set_power(&mut self, on: bool) -> Result<(), BacklightError> {
        if self.has_power {
            return self.send(Job::Power(on));
        }
        match (on, self.restore) {
            (false, None) => {
                self.restore = Some(self.brightness);
                self.send(Job::Brightness(0))
            }
            (true, Some(percent)) => {
                self.restore = None;
                self.send(Job::Brightness(raw_from_percent(percent, self.max)))
            }
            _ => Ok(()),
        }
    }
}

/// Runs the commands one after the other; a brightness that a newer one overtakes is skipped.
struct Worker {
    runner: Arc<dyn Runner>,
    bus: u32,
}

impl Worker {
    fn run(self, queue: &mpsc::Receiver<Job>) {
        let mut last_error = None;
        let mut pending = None;
        loop {
            let job = match pending.take() {
                Some(job) => job,
                None => match queue.recv() {
                    Ok(job) => job,
                    Err(_) => return,
                },
            };
            let job = skip_overtaken(job, queue, &mut pending);
            match self.execute(job) {
                Ok(()) => last_error = None,
                Err(message) => {
                    // The same error again (say on every step of the slider) is not repeated.
                    if last_error.as_ref() != Some(&message) {
                        log::warn!("DDC/CI monitor on i2c-{}: {message}", self.bus);
                        last_error = Some(message);
                    }
                }
            }
        }
    }

    fn execute(&self, job: Job) -> Result<(), String> {
        let (code, value) = match job {
            Job::Brightness(raw) => (BRIGHTNESS, raw.to_string()),
            Job::Power(on) => (POWER_MODE, (if on { POWER_ON } else { POWER_OFF }).to_owned()),
        };
        log::debug!("ddcutil setvcp {code} {value} on i2c-{}", self.bus);
        let mut command = arguments(["setvcp", code, value.as_str()]);
        command.extend(bus_arguments(self.bus));
        // No read back: it doubles the time, and the monitor may not answer once it is off.
        command.push("--noverify".to_owned());
        self.runner.run(&command).map(|_| ()).map_err(|error| match error {
            RunError::NotInstalled => "ddcutil is not installed any more".to_owned(),
            RunError::Failed(message) => message,
        })
    }
}

/// Of consecutive brightness values only the last counts; the first job of another kind is kept in `pending`.
fn skip_overtaken(mut job: Job, queue: &mpsc::Receiver<Job>, pending: &mut Option<Job>) -> Job {
    while matches!(job, Job::Brightness(_)) {
        match queue.try_recv() {
            Ok(next @ Job::Brightness(_)) => job = next,
            Ok(next) => {
                *pending = Some(next);
                break;
            }
            Err(_) => break,
        }
    }
    job
}

fn arguments<const N: usize>(parts: [&str; N]) -> Vec<String> {
    parts.iter().map(|part| (*part).to_owned()).collect()
}

fn bus_arguments(bus: u32) -> [String; 2] {
    ["--bus".to_owned(), bus.to_string()]
}

fn getvcp(bus: u32, code: &str) -> Vec<String> {
    let mut command = arguments(["getvcp", code, "--brief"]);
    command.extend(bus_arguments(bus));
    command
}

fn read_brightness(runner: &dyn Runner, bus: u32) -> Result<(u32, u32), String> {
    let text = runner.run(&getvcp(bus, BRIGHTNESS)).map_err(|error| match error {
        RunError::NotInstalled => "ddcutil is not installed any more".to_owned(),
        RunError::Failed(message) => message,
    })?;
    parse_continuous(&text).filter(|(_, max)| *max > 0).ok_or_else(|| format!("unexpected answer '{}'", text.trim()))
}

/// The monitors of `ddcutil detect --brief`: the blocks "Display N" with their bus and description. Invalid or
/// phantom displays have other headings and are not listed.
fn parse_monitors(text: &str) -> Vec<Monitor> {
    let mut monitors = Vec::new();
    let mut current: Option<Monitor> = None;
    for line in text.lines() {
        let is_heading = line.strip_prefix("Display ").is_some_and(|rest| rest.trim().parse::<u32>().is_ok());
        if is_heading {
            monitors.extend(current.take().filter(|monitor| monitor.bus != u32::MAX));
            current = Some(Monitor { bus: u32::MAX, description: String::new() });
        } else if !line.is_empty() && !line.starts_with(char::is_whitespace) {
            // Another kind of display (invalid, phantom): its lines are not ours.
            monitors.extend(current.take().filter(|monitor| monitor.bus != u32::MAX));
        } else if let Some(monitor) = current.as_mut() {
            let line = line.trim();
            if let Some(path) = line.strip_prefix("I2C bus:") {
                if let Some(number) = path.trim().strip_prefix("/dev/i2c-").and_then(|n| n.parse().ok()) {
                    monitor.bus = number;
                }
            } else if let Some(description) = line.strip_prefix("Monitor:") {
                monitor.description = description.trim().to_owned();
            }
        }
    }
    monitors.extend(current.filter(|monitor| monitor.bus != u32::MAX));
    monitors
}

/// `VCP 10 C 55 100` (code, continuous, current, maximum) as current and maximum.
fn parse_continuous(text: &str) -> Option<(u32, u32)> {
    let mut parts = text.split_whitespace();
    let (vcp, _code, kind) = (parts.next()?, parts.next()?, parts.next()?);
    if vcp != "VCP" || kind != "C" {
        return None;
    }
    Some((parts.next()?.parse().ok()?, parts.next()?.parse().ok()?))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;
    use std::sync::Mutex;
    use std::time::{Duration, Instant};

    const TWO_MONITORS: &str = "Display 1\n   I2C bus:          /dev/i2c-7\n   DRM connector:    card1-DP-2\n   \
        Monitor:          IVM:PL3467WQ:1214234300077\n\nDisplay 2\n   I2C bus:          /dev/i2c-8\n   \
        Monitor:          IVM:PL2792Q:1152170101445\n\n";

    /// Answers by the arguments joined with spaces; everything else fails. Remembers the commands.
    #[derive(Default)]
    struct Fake {
        answers: HashMap<String, Result<String, RunError>>,
        calls: Mutex<Vec<String>>,
    }

    impl Fake {
        fn answer(mut self, command: &str, text: &str) -> Self {
            self.answers.insert(command.to_owned(), Ok(text.to_owned()));
            self
        }

        fn calls(&self) -> Vec<String> {
            self.calls.lock().unwrap().clone()
        }

        /// Waits for the thread of the monitor to run `count` commands that set something.
        fn sets(&self, count: usize) -> Vec<String> {
            let deadline = Instant::now() + Duration::from_secs(5);
            loop {
                let sets: Vec<String> = self.calls().into_iter().filter(|call| call.starts_with("setvcp")).collect();
                if sets.len() >= count || Instant::now() > deadline {
                    return sets;
                }
                std::thread::sleep(Duration::from_millis(5));
            }
        }
    }

    impl Runner for Fake {
        fn run(&self, arguments: &[String]) -> Result<String, RunError> {
            let command = arguments.join(" ");
            self.calls.lock().unwrap().push(command.clone());
            if command.starts_with("setvcp") {
                return Ok(String::new());
            }
            self.answers.get(&command).cloned().unwrap_or_else(|| Err(RunError::Failed("no answer".to_owned())))
        }
    }

    fn with_power(fake: Fake) -> Fake {
        fake.answer("getvcp d6 --brief --bus 7", "VCP D6 SNC x01")
    }

    fn one_monitor() -> Fake {
        Fake::default().answer("detect --brief", TWO_MONITORS).answer("getvcp 10 --brief --bus 7", "VCP 10 C 40 100\n")
    }

    #[test]
    fn detect_lists_only_displays_with_a_bus() {
        let text = "Invalid display\n   I2C bus:  /dev/i2c-3\n\nDisplay 1\n   I2C bus: /dev/i2c-7\n   \
            Monitor: A:B:C\n\nPhantom display\n   I2C bus: /dev/i2c-4\n";
        assert_eq!(parse_monitors(text), vec![Monitor { bus: 7, description: "A:B:C".to_owned() }]);
        assert_eq!(parse_monitors(TWO_MONITORS).len(), 2);
        assert!(parse_monitors("No displays found.\n").is_empty());
    }

    #[test]
    fn brightness_answer_is_current_and_maximum() {
        assert_eq!(parse_continuous("VCP 10 C 55 100\n"), Some((55, 100)));
        assert_eq!(parse_continuous("VCP D6 SNC x01"), None);
        assert_eq!(parse_continuous("Bus /dev/i2c-99 does not exist."), None);
    }

    #[test]
    fn missing_program_is_said_plainly() {
        let fake = Fake {
            answers: HashMap::from([("detect --brief".to_owned(), Err(RunError::NotInstalled))]),
            ..Fake::default()
        };
        let error = Ddcutil::open(Arc::new(fake), None).err().unwrap();
        assert_eq!(error, BacklightError::NotInstalled("ddcutil".to_owned()));
    }

    #[test]
    fn no_listed_monitor_means_nothing_to_control() {
        let fake = Fake::default().answer("detect --brief", "No displays found.\n");
        assert_eq!(Ddcutil::open(Arc::new(fake), None).err().unwrap(), BacklightError::NoDdcMonitor);
    }

    #[test]
    fn the_first_monitor_that_answers_is_taken() {
        // Bus 7 is listed but gives no brightness (like a panel whose HDMI chip has no DDC/CI); bus 8 works.
        let fake = Fake::default()
            .answer("detect --brief", TWO_MONITORS)
            .answer("getvcp 10 --brief --bus 8", "VCP 10 C 70 100\n");
        let backlight = Ddcutil::open(Arc::new(fake), None).unwrap();
        assert_eq!(backlight.name(), "ddc:i2c-8");
        assert_eq!(backlight.brightness().unwrap(), 70);
    }

    #[test]
    fn a_named_monitor_is_chosen_or_not_found() {
        let fake = Arc::new(one_monitor().answer("getvcp 10 --brief --bus 8", "VCP 10 C 70 100"));
        assert_eq!(Ddcutil::open(fake.clone(), Some("ddc:i2c-8")).unwrap().name(), "ddc:i2c-8");
        assert_eq!(
            Ddcutil::open(fake, Some("ddc:i2c-9")).err().unwrap(),
            BacklightError::NotFound("ddc:i2c-9".to_owned())
        );
    }

    #[test]
    fn brightness_goes_to_the_monitor_in_its_own_steps() {
        let fake = Arc::new(one_monitor().answer("getvcp 10 --brief --bus 7", "VCP 10 C 40 255"));
        let mut backlight = Ddcutil::open(fake.clone(), None).unwrap();
        assert_eq!(backlight.brightness().unwrap(), 16, "40 of 255");
        backlight.set_brightness(50).unwrap();
        assert_eq!(fake.sets(1), vec!["setvcp 10 128 --bus 7 --noverify"]);
        assert_eq!(backlight.brightness().unwrap(), 50);
    }

    #[test]
    fn off_uses_the_power_mode_when_the_monitor_has_one() {
        let fake = Arc::new(with_power(one_monitor()));
        let mut backlight = Ddcutil::open(fake.clone(), None).unwrap();
        backlight.set_power(false).unwrap();
        backlight.set_power(true).unwrap();
        assert_eq!(fake.sets(2), vec!["setvcp d6 4 --bus 7 --noverify", "setvcp d6 1 --bus 7 --noverify"]);
    }

    #[test]
    fn without_a_power_mode_off_is_brightness_zero_and_comes_back() {
        let fake = Arc::new(one_monitor());
        let mut backlight = Ddcutil::open(fake.clone(), None).unwrap();
        backlight.set_power(false).unwrap();
        assert_eq!(fake.sets(1), vec!["setvcp 10 0 --bus 7 --noverify"]);
        backlight.set_brightness(60).unwrap();
        assert_eq!(backlight.brightness().unwrap(), 60, "the choice is kept while off");
        backlight.set_power(true).unwrap();
        assert_eq!(fake.sets(2), vec!["setvcp 10 0 --bus 7 --noverify", "setvcp 10 60 --bus 7 --noverify"]);
    }

    #[test]
    fn a_slider_that_runs_ahead_of_the_slow_monitor_sends_only_the_last_value() {
        let (jobs, queue) = mpsc::channel();
        for percent in [20, 30, 40] {
            jobs.send(Job::Brightness(percent)).unwrap();
        }
        jobs.send(Job::Power(false)).unwrap();
        let mut pending = None;
        let first = queue.recv().unwrap();
        assert_eq!(skip_overtaken(first, &queue, &mut pending), Job::Brightness(40));
        assert_eq!(pending, Some(Job::Power(false)), "a power change is never skipped");
    }
}
