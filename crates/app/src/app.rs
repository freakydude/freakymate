//! Assembling the application: settings, service, window and bridges.

use crate::actions;
use crate::backlight_controller::BacklightController;
use crate::display;
use crate::network_controller::NetworkController;
use crate::presenter::Presenter;
use crate::settings_controller::{Bridges, SettingsController};
use crate::{AppWindow, UiSettings};
use freakymate_core::backlight::{BacklightBackend, BacklightService};
use freakymate_core::network::{NetworkBackend, NetworkService};
use freakymate_core::settings::{SettingsError, SettingsStore};
use freakymate_core::{CredentialStore, Endpoint, PrinterService};
use slint::winit_030::winit::event::{ElementState, KeyEvent, WindowEvent};
use slint::winit_030::{EventResult, WinitWindowAccessor};
use slint::{ComponentHandle, Timer, TimerMode};
use std::path::PathBuf;
use std::time::Duration;
use thiserror::Error;

/// Start options, typically from the command line.
#[derive(Debug, Clone, Default)]
pub struct Options {
    /// Overrides the saved Moonraker address for this start.
    pub moonraker: Option<String>,
    pub fullscreen: bool,
    /// Fallback for displays without DRM/KMS: draws straight to the framebuffer (see [`crate::framebuffer`], Linux).
    pub framebuffer: bool,
    pub settings_file: Option<PathBuf>,
    /// NetworkManager, or simulated networks in development builds.
    pub network: NetworkBackend,
    /// The kernel's backlight, or a simulated one in development builds. Default: none, the UI offers no control.
    pub backlight: BacklightBackend,
    /// Set when the setup outside the program folder is older than the program: the command that updates it, shown
    /// as a hint on the Updates tab.
    pub setup_command: Option<String>,
}

#[derive(Debug, Error)]
pub enum AppError {
    #[error(transparent)]
    Settings(#[from] SettingsError),
    #[error(transparent)]
    Endpoint(#[from] freakymate_core::moonraker::InvalidEndpoint),
    #[error("cannot start background runtime: {0}")]
    Runtime(#[from] std::io::Error),
    #[error(transparent)]
    Platform(#[from] slint::PlatformError),
    #[error(transparent)]
    EventLoop(#[from] slint::EventLoopError),
}

/// Running application; keeps runtime and bridges alive as long as the window exists.
pub struct Application {
    pub window: AppWindow,
    pub presenter: std::rc::Rc<Presenter>,
    pub service: PrinterService,
    _settings: std::rc::Rc<SettingsController>,
    _network: std::rc::Rc<NetworkController>,
    _backlight: std::rc::Rc<BacklightController>,
    _clock: Timer,
    _runtime: tokio::runtime::Runtime,
}

impl Application {
    pub fn build(options: Options) -> Result<Self, AppError> {
        let store = match options.settings_file {
            Some(path) => SettingsStore::new(path),
            None => SettingsStore::default_location()?,
        };
        let settings = store.load_or_create().unwrap_or_else(|error| {
            log::warn!("{error}; using defaults");
            freakymate_core::Settings::default()
        });
        crate::logging::set_verbose(settings.verbose_log);
        let renderer = if options.framebuffer {
            "linuxkms-software (framebuffer fallback)".to_owned()
        } else {
            std::env::var("SLINT_BACKEND").unwrap_or_else(|_| "default".to_owned())
        };
        crate::logging::header("Renderer", &renderer);
        let endpoint = Endpoint::parse(options.moonraker.as_deref().unwrap_or(&settings.moonraker))?;
        log::info!("Moonraker at {}", endpoint.display());

        let runtime = tokio::runtime::Builder::new_multi_thread().worker_threads(2).enable_all().build()?;
        #[cfg(target_os = "linux")]
        let input_target = crate::framebuffer::InputTarget::default();
        #[cfg(target_os = "linux")]
        if options.framebuffer {
            crate::framebuffer::select_backend(input_target.clone())?;
        }
        let window = AppWindow::new()?;
        // Wayland and X11 desktops find the icon through the app id and `freakymate.desktop`, not the window icon.
        if !options.framebuffer {
            if let Err(error) = slint::set_xdg_app_id("freakymate") {
                log::warn!("cannot set the app id: {error}");
            }
        }
        // Always in the user's configuration directory, also when the settings lie in printer_data/config.
        let (service, channels) =
            PrinterService::start(runtime.handle(), endpoint.clone(), CredentialStore::default_location());
        let presenter = Presenter::new(&window, service.clone());
        let (network_service, network_channels) = NetworkService::start(runtime.handle(), options.network);
        let network = NetworkController::bind(
            &window,
            network_service,
            service.clone(),
            presenter.clone(),
            &endpoint,
            options.network.is_simulated(),
        );
        let (backlight_service, backlight_states) =
            BacklightService::start(runtime.handle(), options.backlight, &settings.backlight);
        let backlight = BacklightController::bind(&window, backlight_service.clone(), &presenter);
        let settings = SettingsController::bind(
            &window,
            store,
            settings,
            &endpoint,
            Bridges {
                service: service.clone(),
                presenter: presenter.clone(),
                network: network.clone(),
                backlight: backlight_service,
            },
        );
        actions::bind(&window, &service, &presenter, &settings);
        presenter.follow(channels)?;
        network.follow(network_channels)?;
        backlight.follow(backlight_states)?;

        let clock = Timer::default();
        let clock_presenter = presenter.clone();
        clock.start(TimerMode::Repeated, Duration::from_secs(1), move || clock_presenter.tick());
        presenter.refresh_job_texts();

        // Started as a kiosk, the window never gets a frame: without `cage -d` (the service files) cage asks for
        // client-side decorations, and winit keeps the height of its own title bar free when leaving fullscreen
        // (800x445 instead of 800x480). This covers compositors without xdg-decoration as well.
        window.set_kiosk(options.fullscreen);
        window.global::<UiSettings>().set_setup_command(options.setup_command.unwrap_or_default().into());
        if options.fullscreen || settings.fullscreen() {
            window.window().set_fullscreen(true);
        }
        #[cfg(target_os = "linux")]
        if options.framebuffer {
            // Set only once, so this cannot fail.
            let _ = input_target.set(backlight.clone());
            crate::framebuffer::fit_when_shown(&window);
        }
        if !options.framebuffer {
            watch_window_events(&window, Some(backlight.clone()));
        }
        Ok(Self {
            window,
            presenter,
            service,
            _settings: settings,
            _network: network,
            _backlight: backlight,
            _clock: clock,
            _runtime: runtime,
        })
    }

    pub fn run(self) -> Result<(), AppError> {
        self.window.run()?;
        Ok(())
    }
}

/// The one filter for winit's window events; Slint keeps only one per window, so a second registration would
/// silently replace the first. In fullscreen the layout is fitted to the real display whenever the size changes (see
/// [`display`]); the scale of the system is not used there, a compositor reports 1 for a touch screen. A window keeps
/// the scale of the system, so a larger window shows more room. Whoever sets `SLINT_SCALE_FACTOR` chooses the scale
/// by hand, and nothing is fitted then. (The pointer of the kiosk is hidden by its cursor theme, see
/// [`crate::kiosk_pointer`].) While the screen is dark or just lit, touches, clicks and keys are swallowed here, so the
/// UI needs no wake-up surface of its own; they restart the idle time (see [`BacklightController::input`]).
pub fn watch_window_events(window: &AppWindow, backlight: Option<std::rc::Rc<BacklightController>>) {
    let fit = std::env::var_os("SLINT_SCALE_FACTOR").is_none();
    let weak = window.as_weak();
    window.window().on_winit_window_event(move |_, event| {
        if let (Some(backlight), Some(activity)) = (&backlight, input_activity(event)) {
            if backlight.input(activity) {
                return EventResult::PreventDefault;
            }
        }
        match event {
            WindowEvent::Resized(_) if fit => {
                // Applied after the event, not inside it: Slint is still handling it.
                let weak = weak.clone();
                Timer::single_shot(Duration::ZERO, move || {
                    if let Some(window) = weak.upgrade() {
                        scale_to_mode(window.window());
                    }
                });
                EventResult::Propagate
            }
            WindowEvent::ScaleFactorChanged { .. } if fit => EventResult::PreventDefault,
            _ => EventResult::Propagate,
        }
    });
}

/// Whether an input event is a press, a touch or a key, which counts as activity for the standby; `None` for events
/// that are no input. A release or a mouse wheel is input, but no activity of its own.
fn input_activity(event: &WindowEvent) -> Option<bool> {
    match event {
        WindowEvent::MouseInput { state, .. } | WindowEvent::KeyboardInput { event: KeyEvent { state, .. }, .. } => {
            Some(*state == ElementState::Pressed)
        }
        WindowEvent::Touch(_) => Some(true),
        WindowEvent::MouseWheel { .. } => Some(false),
        _ => None,
    }
}

/// Fullscreen fits the layout to the display; a window shows it at the scale of the system (see
/// [`display::fits_to_display`]).
pub(crate) fn scale_to_mode(window: &slint::Window) {
    let system_scale = window.with_winit_window(|winit_window| winit_window.scale_factor() as f32).unwrap_or(1.0);
    let size = window.size();
    if display::fits_to_display(window.is_fullscreen(), size.width, size.height, system_scale) {
        display::apply_fit(window);
    } else {
        display::apply_system_scale(window, system_scale);
    }
    let mode = if window.is_fullscreen() { "fullscreen" } else { "window" };
    crate::logging::header(
        "Display",
        &format!("{mode} {}x{}, scale factor {:.3}", size.width, size.height, window.scale_factor()),
    );
}
