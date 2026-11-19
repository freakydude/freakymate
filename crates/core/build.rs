//! Embeds the git commit and the release channel of this build (see `sailhand_core::BUILD_COMMIT`).

use std::path::PathBuf;
use std::process::Command;

fn main() {
    println!("cargo:rerun-if-env-changed=SAILHAND_COMMIT");
    println!("cargo:rerun-if-env-changed=SAILHAND_VERSION");
    println!("cargo:rerun-if-env-changed=SAILHAND_CHANNEL");

    println!("cargo:rustc-env=SAILHAND_BUILD_COMMIT={}", commit());
    println!("cargo:rustc-env=SAILHAND_BUILD_CHANNEL={}", channel());
}

/// Short hash from CI (`SAILHAND_COMMIT`), else from git, else "unknown".
fn commit() -> String {
    let from_ci =
        std::env::var("SAILHAND_COMMIT").ok().map(|hash| hash.trim().to_owned()).filter(|hash| !hash.is_empty());
    if let Some(hash) = from_ci {
        return hash.chars().take(7).collect();
    }
    watch_git_head();
    git(&["rev-parse", "--short=7", "HEAD"]).unwrap_or_else(|| "unknown".to_owned())
}

/// `SAILHAND_CHANNEL` if set; else from the version CI passes (see `.github/scripts/version.sh`): `X.Y.0` is a release
/// ("stable"), `X.Y.N` from main a pre-release ("beta"), anything with a suffix or without a version a local, branch
/// or pull request build ("dev").
fn channel() -> String {
    if let Ok(channel) = std::env::var("SAILHAND_CHANNEL")
        && !channel.trim().is_empty()
    {
        return channel.trim().to_owned();
    }
    match std::env::var("SAILHAND_VERSION") {
        Ok(version) if !version.contains('-') && version.ends_with(".0") => "stable",
        Ok(version) if !version.contains('-') && !version.is_empty() => "beta",
        _ => "dev",
    }
    .to_owned()
}

/// A new commit changes the file the checked-out branch points to; switching branches changes HEAD itself.
fn watch_git_head() {
    let Some(head) = git(&["rev-parse", "--git-path", "HEAD"]) else { return };
    println!("cargo:rerun-if-changed={head}");
    let reference =
        std::fs::read_to_string(&head).ok().and_then(|text| text.strip_prefix("ref: ").map(|r| r.trim().to_owned()));
    if let Some(reference) = reference
        && let Some(path) = git(&["rev-parse", "--git-path", &reference])
    {
        println!("cargo:rerun-if-changed={path}");
    }
    if let Some(packed) = git(&["rev-parse", "--git-path", "packed-refs"]) {
        println!("cargo:rerun-if-changed={packed}");
    }
}

fn git(args: &[&str]) -> Option<String> {
    let manifest_dir = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").ok()?);
    let output = Command::new("git").args(args).current_dir(manifest_dir).output().ok()?;
    let text = String::from_utf8(output.stdout).ok()?.trim().to_owned();
    (output.status.success() && !text.is_empty()).then_some(text)
}
