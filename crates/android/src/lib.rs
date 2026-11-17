//! Android entry point: the native activity calls [`android_main`], which starts the same program as `main.rs`.
//! Everything below is empty on other platforms, so the workspace still builds on the desktop.
//!
//! It runs once per activity, on a thread of its own: when Android destroyed the activity in the background, the
//! next start calls it again in the same process. Slint keeps its platform per thread, so it can start again; the
//! program's own process-wide state must allow that too (`logging::init`, `android::init`). Text on stderr appears in
//! logcat under the tag `RustStdoutStderr`.

#[cfg(target_os = "android")]
#[unsafe(no_mangle)]
fn android_main(app: android_activity::AndroidApp) {
    let data_dir = app.internal_data_path();
    sailhand::android::init(app.clone());
    // Slint takes the activity for the window and the input; without it nothing can be shown.
    if let Err(error) = slint::android::init_with_event_listener(app, sailhand::android::event_listener()) {
        eprintln!("cannot start the Slint backend: {error}");
        return;
    }
    let Some(data_dir) = data_dir else {
        eprintln!("the app has no data folder");
        return;
    };
    // The exit code means nothing to an activity; failures are in the log.
    let _ = sailhand::startup::run(sailhand::startup::Cli::android(&data_dir));
}
