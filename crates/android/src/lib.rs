//! Android entry point: the native activity calls [`android_main`], which starts the same program as `main.rs`.
//! Everything below is empty on other platforms, so the workspace still builds on the desktop.

#[cfg(target_os = "android")]
#[unsafe(no_mangle)]
fn android_main(app: android_activity::AndroidApp) {
    let data_dir = app.internal_data_path();
    // Slint takes the activity for the window and the input; without it nothing can be shown.
    if let Err(error) = slint::android::init(app) {
        eprintln!("cannot start the Slint backend: {error}");
        return;
    }
    let Some(data_dir) = data_dir else {
        eprintln!("the app has no data folder");
        return;
    };
    // The exit code means nothing to an activity; failures are in the log.
    let _ = freakymate::startup::run(freakymate::startup::Cli::android(&data_dir));
}
