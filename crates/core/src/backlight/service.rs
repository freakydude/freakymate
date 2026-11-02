//! Background task that owns the backlight, runs the standby and publishes the state.

#[cfg(feature = "dev-mode")]
use super::simulated::Simulated;
use super::{
    usable_brightness, Availability, Backlight, BacklightError, BacklightSettings, BacklightState, Screen, Standby,
    StandbyTimeouts, Sysfs,
};
use std::path::PathBuf;
use std::time::Instant;
use tokio::sync::{mpsc, watch};

/// Which implementation controls the backlight.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum BacklightBackend {
    /// The kernel's backlight class under this directory, normally [`Sysfs::ROOT`].
    Sysfs(PathBuf),
    /// No control (e.g. Windows): the UI offers neither brightness nor standby.
    #[default]
    None,
    /// In-memory backlight for development and screenshots.
    #[cfg(feature = "dev-mode")]
    Simulated,
}

impl BacklightBackend {
    /// What this platform offers.
    pub fn system() -> Self {
        if cfg!(target_os = "linux") {
            Self::Sysfs(PathBuf::from(Sysfs::ROOT))
        } else {
            Self::None
        }
    }
}

/// Handle for the UI; cheap to clone.
#[derive(Clone)]
pub struct BacklightService {
    requests: mpsc::UnboundedSender<Request>,
}

#[derive(Debug)]
enum Request {
    SetBrightness(u8),
    Sleep,
    Touch,
    /// With the reason for the log.
    Wake(String),
    Printing(bool),
    Timeouts(StandbyTimeouts),
}

impl BacklightService {
    /// Opens the backlight and sets the saved brightness, if any.
    pub fn start(
        runtime: &tokio::runtime::Handle,
        backend: BacklightBackend,
        settings: &BacklightSettings,
    ) -> (Self, watch::Receiver<BacklightState>) {
        let (requests, requests_rx) = mpsc::unbounded_channel();
        let (state_tx, states) = watch::channel(BacklightState::default());
        runtime.spawn(run(backend, settings.clone(), requests_rx, state_tx));
        (Self { requests }, states)
    }

    /// Brightness in percent, kept between [`MIN_BRIGHTNESS`](super::MIN_BRIGHTNESS) and 100.
    pub fn set_brightness(&self, percent: u8) {
        self.send(Request::SetBrightness(percent));
    }

    /// Screen off now; the next touch wakes it.
    pub fn sleep(&self) {
        self.send(Request::Sleep);
    }

    /// Every touch: wakes a dark screen or restarts the idle time.
    pub fn touch(&self) {
        self.send(Request::Touch);
    }

    /// The printer needs attention; `reason` goes to the log when this lights the screen.
    pub fn wake(&self, reason: String) {
        self.send(Request::Wake(reason));
    }

    /// Whether a print runs or is paused; a change wakes the screen.
    pub fn set_printing(&self, printing: bool) {
        self.send(Request::Printing(printing));
    }

    pub fn set_timeouts(&self, timeouts: StandbyTimeouts) {
        self.send(Request::Timeouts(timeouts));
    }

    fn send(&self, request: Request) {
        let _ = self.requests.send(request);
    }
}

fn open(backend: BacklightBackend, device: Option<&str>) -> Result<Box<dyn Backlight>, BacklightError> {
    match backend {
        BacklightBackend::Sysfs(root) => Ok(Box::new(Sysfs::open(&root, device)?)),
        BacklightBackend::None => Err(BacklightError::Unsupported),
        #[cfg(feature = "dev-mode")]
        BacklightBackend::Simulated => Ok(Box::new(Simulated::default())),
    }
}

struct Controller {
    backlight: Box<dyn Backlight>,
    standby: Standby,
    state: watch::Sender<BacklightState>,
    /// What the last request was, for the log line when it switches the screen.
    cause: Option<String>,
    /// The last error logged; the same one again (say on every step of the slider) is not repeated.
    last_error: Option<BacklightError>,
}

impl Controller {
    fn apply(&mut self, request: Request, now: Instant) {
        self.cause = match &request {
            Request::Sleep => Some("Off button".to_owned()),
            Request::Touch => Some("touch".to_owned()),
            Request::Wake(reason) => Some(reason.clone()),
            Request::Printing(true) => Some("print started".to_owned()),
            Request::Printing(false) => Some("print ended".to_owned()),
            Request::SetBrightness(_) | Request::Timeouts(_) => None,
        };
        match request {
            Request::SetBrightness(percent) => {
                let percent = usable_brightness(percent);
                match self.backlight.set_brightness(percent) {
                    Ok(()) => {
                        self.state.send_if_modified(|state| replace(&mut state.brightness, percent));
                    }
                    Err(error) => self.warn(error),
                }
            }
            Request::Sleep => self.standby.sleep(),
            Request::Touch => self.standby.touch(now),
            Request::Wake(_) => self.standby.wake(now),
            Request::Printing(printing) => self.standby.set_printing(printing, now),
            Request::Timeouts(timeouts) => {
                log::info!(
                    "screen off after {} idle, {} printing",
                    describe(timeouts.idle),
                    describe(timeouts.printing)
                );
                self.standby.set_timeouts(timeouts, now);
            }
        }
    }

    /// Switches the light to what the standby says and publishes the screen.
    fn follow_standby(&mut self, now: Instant) {
        self.standby.update(now);
        let screen = self.standby.screen();
        let was_off = self.state.borrow().screen == Screen::Off;
        let is_off = screen == Screen::Off;
        let cause = self.cause.take();
        if was_off != is_off {
            let cause = cause.unwrap_or_else(|| format!("no touch for {}", describe(self.standby.timeout())));
            log::info!("screen {} ({cause})", if is_off { "off" } else { "on" });
            if let Err(error) = self.backlight.set_power(!is_off) {
                self.warn(error);
            }
        }
        self.state.send_if_modified(|state| replace(&mut state.screen, screen));
    }

    fn warn(&mut self, error: BacklightError) {
        if self.last_error.as_ref() != Some(&error) {
            log::warn!("{error}");
            self.last_error = Some(error);
        }
    }
}

fn describe(timeout: Option<std::time::Duration>) -> String {
    match timeout {
        Some(timeout) if timeout.as_secs() % 60 == 0 => format!("{} min", timeout.as_secs() / 60),
        Some(timeout) => format!("{:.1} s", timeout.as_secs_f64()),
        None => "never".to_owned(),
    }
}

/// Sets `target` to `value`; true when that changed it.
fn replace<T: PartialEq>(target: &mut T, value: T) -> bool {
    let changed = *target != value;
    *target = value;
    changed
}

async fn run(
    backend: BacklightBackend,
    settings: BacklightSettings,
    mut requests: mpsc::UnboundedReceiver<Request>,
    state: watch::Sender<BacklightState>,
) {
    let mut backlight = match open(backend, settings.device.as_deref()) {
        Ok(backlight) => backlight,
        Err(error) => {
            // Said once, in the header of every log file; the UI simply offers nothing.
            log::info!(target: crate::HEADER_TARGET, "Backlight: none ({error})");
            state.send_replace(BacklightState {
                availability: Availability::Unavailable(error.to_string()),
                ..Default::default()
            });
            while requests.recv().await.is_some() {}
            return;
        }
    };
    if let Some(percent) = settings.brightness {
        if let Err(error) = backlight.set_brightness(usable_brightness(percent)) {
            log::warn!("{error}");
        }
    }
    let brightness = backlight.brightness().unwrap_or(100);
    let timeouts = settings.timeouts();
    log::info!(
        target: crate::HEADER_TARGET,
        "Backlight: {} at {brightness} %, off after {} idle, {} printing",
        backlight.name(),
        describe(timeouts.idle),
        describe(timeouts.printing)
    );
    state.send_replace(BacklightState {
        availability: Availability::Available { device: backlight.name().to_owned() },
        brightness,
        screen: Screen::On,
    });
    let mut controller =
        Controller { backlight, standby: Standby::new(timeouts, Instant::now()), state, cause: None, last_error: None };
    loop {
        let deadline = controller.standby.deadline().map(tokio::time::Instant::from_std);
        tokio::select! {
            request = requests.recv() => match request {
                Some(request) => controller.apply(request, Instant::now()),
                None => return,
            },
            () = sleep_until(deadline) => {}
        }
        controller.follow_standby(Instant::now());
    }
}

/// Waits for `deadline`, or forever without one.
async fn sleep_until(deadline: Option<tokio::time::Instant>) {
    match deadline {
        Some(deadline) => tokio::time::sleep_until(deadline).await,
        None => std::future::pending().await,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn available(states: &mut watch::Receiver<BacklightState>) -> BacklightState {
        states.wait_for(|state| state.availability != Availability::Checking).await.unwrap().clone()
    }

    #[tokio::test]
    async fn without_control_nothing_is_offered() {
        let (service, mut states) =
            BacklightService::start(&tokio::runtime::Handle::current(), BacklightBackend::None, &Default::default());
        assert!(matches!(available(&mut states).await.availability, Availability::Unavailable(_)));
        service.sleep();
    }

    #[tokio::test]
    async fn sysfs_device_gets_the_saved_brightness_and_goes_off_and_on() {
        let root = tempfile::tempdir().unwrap();
        let dir = root.path().join("10-0045");
        std::fs::create_dir(&dir).unwrap();
        for (file, value) in [("max_brightness", "255"), ("brightness", "255"), ("bl_power", "0")] {
            std::fs::write(dir.join(file), value).unwrap();
        }
        let settings = BacklightSettings { brightness: Some(2), ..Default::default() };
        let backend = BacklightBackend::Sysfs(root.path().to_owned());
        let (service, mut states) = BacklightService::start(&tokio::runtime::Handle::current(), backend, &settings);
        let state = available(&mut states).await;
        assert_eq!(state.availability, Availability::Available { device: "10-0045".into() });
        assert_eq!(state.brightness, super::super::MIN_BRIGHTNESS, "a saved value below the minimum is raised");

        let read = |file: &str| std::fs::read_to_string(dir.join(file)).unwrap().trim().to_owned();
        service.sleep();
        states.wait_for(|state| state.screen == Screen::Off).await.unwrap();
        assert_eq!(read("bl_power"), "4");
        service.touch();
        states.wait_for(|state| state.screen == Screen::Waking).await.unwrap();
        assert_eq!(read("bl_power"), "0");
        states.wait_for(|state| state.screen == Screen::On).await.unwrap();
    }

    #[cfg(feature = "dev-mode")]
    #[tokio::test]
    async fn simulated_idle_timeout_switches_off() {
        use std::time::Duration;
        let (service, mut states) = BacklightService::start(
            &tokio::runtime::Handle::current(),
            BacklightBackend::Simulated,
            &Default::default(),
        );
        assert_eq!(available(&mut states).await.brightness, 80);
        service.set_brightness(55);
        states.wait_for(|state| state.brightness == 55).await.unwrap();
        service.set_timeouts(StandbyTimeouts { idle: Some(Duration::from_millis(20)), printing: None });
        states.wait_for(|state| state.screen == Screen::Off).await.unwrap();
        service.wake("test".to_owned());
        states.wait_for(|state| state.screen == Screen::Waking).await.unwrap();
    }
}
