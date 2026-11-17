use std::path::Path;

fn main() {
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        embed_windows_icon();
    }
}

/// Embeds the app icon into `sailhand.exe` (Explorer, taskbar, shortcuts), with rc.exe (MSVC) or windres (GNU).
fn embed_windows_icon() {
    let manifest_dir = std::env::var("CARGO_MANIFEST_DIR").expect("cargo sets CARGO_MANIFEST_DIR");
    let icon = Path::new(&manifest_dir).join("../../assets/logo/sailhand.ico");
    let icon = icon.canonicalize().expect("assets/logo/sailhand.ico must exist");
    println!("cargo:rerun-if-changed={}", icon.display());

    // Written with an absolute path, so neither compiler depends on its working directory; rc accepts "/".
    let out_dir = std::env::var("OUT_DIR").expect("cargo sets OUT_DIR");
    let resource = Path::new(&out_dir).join("sailhand.rc");
    let icon_path = icon.to_string_lossy().replace('\\', "/");
    let icon_path = icon_path.strip_prefix("//?/").unwrap_or(&icon_path);
    std::fs::write(&resource, format!("1 ICON \"{icon_path}\"\n")).expect("OUT_DIR must be writable");

    embed_resource::compile(&resource, embed_resource::NONE)
        .manifest_optional()
        .expect("the Windows resource with the icon must compile");
}
