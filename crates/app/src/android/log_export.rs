//! Hands the log to the user: into the public Download folder, where a file manager or a mail app finds it.
//!
//! Android 10 and later: `MediaStore.Downloads` (`Download/FreakyMate`), which needs no permission. Android 7 to 9:
//! the app's own folder on the shared storage (`Context.getExternalFilesDir`), also without permission (the public
//! Download folder would need `WRITE_EXTERNAL_STORAGE`). The Java calls run on a thread of their own.

use super::ACTIVITY;
use crate::log_export::{self, LogExport};
use jni::errors::Error as JniError;
use jni::objects::{JObject, JString};
use jni::{Env, JValue, JavaVM, jni_sig, jni_str};
use std::fs;
use std::path::Path;
use thiserror::Error;

/// First Android version with `MediaStore.Downloads`.
const MEDIA_STORE_DOWNLOADS_API: i32 = 29;
/// Folder in the public Download folder (Android 10 and later).
const PUBLIC_FOLDER: &str = "Download/FreakyMate";

#[derive(Debug, Error)]
enum ExportError {
    #[error("{0}")]
    Jni(#[from] JniError),
    #[error("{0}")]
    Other(String),
}

/// Writes the export on a thread of its own and calls `done` there with where the file is or why it is not.
pub(crate) fn start(done: impl FnOnce(Result<String, String>) + Send + 'static) {
    let spawned = std::thread::Builder::new().name("log-export".to_owned()).spawn(move || {
        let result = export().inspect_err(|error| log::warn!("log export failed: {error}"));
        done(result);
    });
    if let Err(error) = spawned {
        log::warn!("log export did not start: {error}");
    }
}

fn export() -> Result<String, String> {
    let app = ACTIVITY.get().ok_or("no activity")?;
    let export = log_export::read_current(chrono::Local::now().naive_local()).map_err(|error| error.to_string())?;
    // SAFETY: both pointers belong to the activity held by `ACTIVITY` for the life of the process.
    let vm = unsafe { JavaVM::from_raw(app.vm_as_ptr().cast()) };
    let activity_ptr = app.activity_as_ptr();
    vm.attach_current_thread(|env| {
        // SAFETY: the activity is a global reference kept alive by `AndroidApp`; this is only a borrowed handle.
        let activity = unsafe { JObject::from_raw(env, activity_ptr.cast()) };
        save(env, &activity, &export)
    })
    .map_err(|error: ExportError| error.to_string())
}

fn save(env: &mut Env, activity: &JObject, export: &LogExport) -> Result<String, ExportError> {
    let sdk = env.get_static_field(jni_str!("android/os/Build$VERSION"), jni_str!("SDK_INT"), jni_sig!(int))?.i()?;
    if sdk >= MEDIA_STORE_DOWNLOADS_API {
        save_to_media_store(env, activity, export)?;
        Ok(format!("{PUBLIC_FOLDER}/{}", export.file_name))
    } else {
        save_to_app_folder(env, activity, export)
    }
}

fn save_to_media_store(env: &mut Env, activity: &JObject, export: &LogExport) -> Result<(), ExportError> {
    let resolver = env
        .call_method(activity, jni_str!("getContentResolver"), jni_sig!(() -> android.content.ContentResolver), &[])?
        .l()?;
    let values = env.new_object(jni_str!("android/content/ContentValues"), jni_sig!(() -> void), &[])?;
    for (key, value) in
        [("_display_name", export.file_name.as_str()), ("mime_type", "text/plain"), ("relative_path", PUBLIC_FOLDER)]
    {
        let key = env.new_string(key)?;
        let value = env.new_string(value)?;
        env.call_method(
            &values,
            jni_str!("put"),
            jni_sig!((key: java.lang.String, value: java.lang.String) -> void),
            &[JValue::Object(&key), JValue::Object(&value)],
        )?;
    }
    let collection = env
        .get_static_field(
            jni_str!("android/provider/MediaStore$Downloads"),
            jni_str!("EXTERNAL_CONTENT_URI"),
            jni_sig!(android.net.Uri),
        )?
        .l()?;
    let uri = env
        .call_method(
            &resolver,
            jni_str!("insert"),
            jni_sig!((url: android.net.Uri, values: android.content.ContentValues) -> android.net.Uri),
            &[JValue::Object(&collection), JValue::Object(&values)],
        )?
        .l()?;
    if uri.is_null() {
        return Err(ExportError::Other("the system gave no place for the file".to_owned()));
    }
    let stream = env
        .call_method(
            &resolver,
            jni_str!("openOutputStream"),
            jni_sig!((uri: android.net.Uri) -> java.io.OutputStream),
            &[JValue::Object(&uri)],
        )?
        .l()?;
    let bytes = env.byte_array_from_slice(&export.content)?;
    let written =
        env.call_method(&stream, jni_str!("write"), jni_sig!((bytes: jbyte[]) -> void), &[JValue::Object(&bytes)]);
    // Closed in any case; a write error is the one to report.
    let closed = env.call_method(&stream, jni_str!("close"), jni_sig!(() -> void), &[]);
    written?;
    closed?;
    Ok(())
}

fn save_to_app_folder(env: &mut Env, activity: &JObject, export: &LogExport) -> Result<String, ExportError> {
    let kind = env.new_string("Download")?;
    let folder = env
        .call_method(
            activity,
            jni_str!("getExternalFilesDir"),
            jni_sig!((kind: java.lang.String) -> java.io.File),
            &[JValue::Object(&kind)],
        )?
        .l()?;
    if folder.is_null() {
        return Err(ExportError::Other("no app folder on the shared storage".to_owned()));
    }
    let path = env.call_method(&folder, jni_str!("getAbsolutePath"), jni_sig!(() -> java.lang.String), &[])?.l()?;
    let path = env.cast_local::<JString>(path)?.try_to_string(env)?;
    let target = Path::new(&path).join(&export.file_name);
    fs::write(&target, &export.content).map_err(|error| ExportError::Other(error.to_string()))?;
    Ok(target.display().to_string())
}
