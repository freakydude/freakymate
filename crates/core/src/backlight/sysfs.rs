//! Backlight through the kernel's backlight class (`/sys/class/backlight/<device>`): DSI and SPI displays
//! (`gpio-backlight`, `pwm-backlight`) and laptops alike, the kernel already abstracts them.

use super::{percent_from_raw, raw_from_percent, Backlight, BacklightError};
use std::fs::{self, OpenOptions};
use std::io;
use std::path::{Path, PathBuf};

/// `bl_power` values of the kernel (`FB_BLANK_UNBLANK`, `FB_BLANK_POWERDOWN`).
const POWER_ON: u32 = 0;
const POWER_OFF: u32 = 4;

#[derive(Debug)]
pub struct Sysfs {
    name: String,
    dir: PathBuf,
    max: u32,
    /// Whether `bl_power` is writable. Brightness 0 is not off on every driver, so off goes through `bl_power`
    /// where possible.
    has_power: bool,
    /// Without `bl_power`: the raw brightness to bring back after off.
    restore: Option<u32>,
}

impl Sysfs {
    pub const ROOT: &str = "/sys/class/backlight";

    /// Opens `wanted` or, without a name, the device the type suits best. `root` is [`Self::ROOT`] except in tests.
    pub fn open(root: &Path, wanted: Option<&str>) -> Result<Self, BacklightError> {
        // Everything there is, so a report shows which device would have been the right one.
        log::info!("backlight devices in {}: {}", root.display(), describe_devices(root));
        let dir = choose_device(root, wanted)?;
        let name = dir.file_name().map(|name| name.to_string_lossy().into_owned()).unwrap_or_default();
        let max = read_number(&dir.join("max_brightness")).map_err(|error| device_error(&name, &error))?;
        if max == 0 {
            return Err(BacklightError::Device { name, message: "max_brightness is 0".to_owned() });
        }
        check_writable(&dir.join("brightness")).map_err(|error| match error.kind() {
            io::ErrorKind::PermissionDenied => BacklightError::PermissionDenied(name.clone()),
            _ => device_error(&name, &error),
        })?;
        let has_power = check_writable(&dir.join("bl_power")).is_ok();
        let off = if has_power { "off through bl_power" } else { "off through brightness 0 (bl_power not writable)" };
        log::info!(
            "using backlight '{name}' ({}), {off}",
            if wanted.is_some() { "configured" } else { "chosen by type" }
        );
        Ok(Self { name, dir, max, has_power, restore: None })
    }

    /// What the driver reports after switching, for the verbose log: shows whether it really went dark.
    fn log_power_readback(&self) {
        if log::log_enabled!(log::Level::Debug) {
            let read =
                |file: &str| read_number(&self.dir.join(file)).map_or_else(|_| "-".to_owned(), |v| v.to_string());
            log::debug!(
                "backlight '{}': bl_power {}, brightness {}, actual_brightness {}",
                self.name,
                read("bl_power"),
                read("brightness"),
                read("actual_brightness")
            );
        }
    }

    fn read_raw(&self) -> Result<u32, BacklightError> {
        read_number(&self.dir.join("brightness")).map_err(|error| device_error(&self.name, &error))
    }

    fn write(&self, file: &str, value: u32) -> Result<(), BacklightError> {
        fs::write(self.dir.join(file), format!("{value}\n")).map_err(|error| device_error(&self.name, &error))
    }
}

impl Backlight for Sysfs {
    fn name(&self) -> &str {
        &self.name
    }

    fn brightness(&self) -> Result<u8, BacklightError> {
        let raw = match self.restore {
            Some(raw) => raw,
            None => self.read_raw()?,
        };
        Ok(percent_from_raw(raw, self.max))
    }

    fn set_brightness(&mut self, percent: u8) -> Result<(), BacklightError> {
        let raw = raw_from_percent(percent, self.max);
        // Switched off by brightness 0: remember it for later instead of lighting up.
        if self.restore.is_some() {
            self.restore = Some(raw);
            return Ok(());
        }
        self.write("brightness", raw)
    }

    fn set_power(&mut self, on: bool) -> Result<(), BacklightError> {
        self.switch_power(on)?;
        self.log_power_readback();
        Ok(())
    }
}

impl Sysfs {
    fn switch_power(&mut self, on: bool) -> Result<(), BacklightError> {
        if self.has_power {
            return self.write("bl_power", if on { POWER_ON } else { POWER_OFF });
        }
        match (on, self.restore) {
            (false, None) => {
                let raw = self.read_raw()?;
                self.write("brightness", 0)?;
                self.restore = Some(raw);
            }
            (true, Some(raw)) => {
                self.write("brightness", raw)?;
                self.restore = None;
            }
            _ => {}
        }
        Ok(())
    }
}

fn choose_device(root: &Path, wanted: Option<&str>) -> Result<PathBuf, BacklightError> {
    if let Some(name) = wanted {
        let dir = root.join(name);
        return if is_device(&dir) { Ok(dir) } else { Err(BacklightError::NotFound(name.to_owned())) };
    }
    let no_device = || BacklightError::NoDevice(root.display().to_string());
    let mut devices: Vec<(u8, PathBuf)> = fs::read_dir(root)
        .map_err(|_| no_device())?
        .filter_map(|entry| entry.ok().map(|entry| entry.path()))
        .filter(|dir| is_device(dir))
        .map(|dir| (type_rank(&dir), dir))
        .collect();
    devices.sort();
    devices.into_iter().next().map(|(_, dir)| dir).ok_or_else(no_device)
}

/// One entry per device: `10-0045 (raw, 128/255, bl_power, writable)`.
fn describe_devices(root: &Path) -> String {
    let Ok(entries) = fs::read_dir(root) else {
        return "none (directory missing)".to_owned();
    };
    let mut devices: Vec<String> = entries
        .filter_map(|entry| entry.ok().map(|entry| entry.path()))
        .filter(|dir| is_device(dir))
        .map(|dir| {
            let read = |file: &str| fs::read_to_string(dir.join(file)).unwrap_or_default().trim().to_owned();
            let name = dir.file_name().map(|name| name.to_string_lossy().into_owned()).unwrap_or_default();
            let power = if dir.join("bl_power").is_file() { ", bl_power" } else { "" };
            let access = if check_writable(&dir.join("brightness")).is_ok() { "writable" } else { "read-only" };
            format!("{name} ({}, {}/{}{power}, {access})", read("type"), read("brightness"), read("max_brightness"))
        })
        .collect();
    devices.sort();
    if devices.is_empty() {
        "none".to_owned()
    } else {
        devices.join(", ")
    }
}

fn is_device(dir: &Path) -> bool {
    dir.join("brightness").is_file() && dir.join("max_brightness").is_file()
}

/// Like the usual laptop tools: firmware (ACPI) before platform before raw (direct register access).
fn type_rank(dir: &Path) -> u8 {
    match fs::read_to_string(dir.join("type")).unwrap_or_default().trim() {
        "firmware" => 0,
        "platform" => 1,
        "raw" => 2,
        _ => 3,
    }
}

fn read_number(path: &Path) -> io::Result<u32> {
    fs::read_to_string(path)?
        .trim()
        .parse()
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, format!("{}: {error}", path.display())))
}

/// Opening for writing checks the permission without changing anything.
fn check_writable(path: &Path) -> io::Result<()> {
    OpenOptions::new().write(true).open(path).map(drop)
}

fn device_error(name: &str, error: &io::Error) -> BacklightError {
    BacklightError::Device { name: name.to_owned(), message: error.to_string() }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A device directory like the kernel's.
    fn add_device(root: &Path, name: &str, kind: &str, max: u32, brightness: u32, power: bool) -> PathBuf {
        let dir = root.join(name);
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("type"), format!("{kind}\n")).unwrap();
        fs::write(dir.join("max_brightness"), format!("{max}\n")).unwrap();
        fs::write(dir.join("brightness"), format!("{brightness}\n")).unwrap();
        if power {
            fs::write(dir.join("bl_power"), "0\n").unwrap();
        }
        dir
    }

    fn read(dir: &Path, file: &str) -> u32 {
        read_number(&dir.join(file)).unwrap()
    }

    #[test]
    fn picks_firmware_before_platform_before_raw() {
        let root = tempfile::tempdir().unwrap();
        add_device(root.path(), "a_raw", "raw", 255, 100, true);
        add_device(root.path(), "b_platform", "platform", 255, 100, true);
        assert_eq!(Sysfs::open(root.path(), None).unwrap().name(), "b_platform");
        add_device(root.path(), "c_firmware", "firmware", 255, 100, true);
        assert_eq!(Sysfs::open(root.path(), None).unwrap().name(), "c_firmware");
    }

    #[test]
    fn configured_device_wins_and_a_missing_one_is_reported() {
        let root = tempfile::tempdir().unwrap();
        add_device(root.path(), "intel_backlight", "raw", 96_000, 48_000, true);
        add_device(root.path(), "acpi_video0", "firmware", 100, 50, true);
        assert_eq!(Sysfs::open(root.path(), Some("intel_backlight")).unwrap().name(), "intel_backlight");
        assert_eq!(
            Sysfs::open(root.path(), Some("10-0045")).unwrap_err(),
            BacklightError::NotFound("10-0045".to_owned())
        );
    }

    #[test]
    fn empty_or_missing_class_has_no_device() {
        let root = tempfile::tempdir().unwrap();
        assert!(matches!(Sysfs::open(root.path(), None), Err(BacklightError::NoDevice(_))));
        assert!(matches!(Sysfs::open(&root.path().join("missing"), None), Err(BacklightError::NoDevice(_))));
    }

    #[test]
    fn reads_and_writes_brightness_in_percent() {
        let root = tempfile::tempdir().unwrap();
        let dir = add_device(root.path(), "10-0045", "raw", 255, 255, true);
        let mut backlight = Sysfs::open(root.path(), None).unwrap();
        assert_eq!(backlight.brightness().unwrap(), 100);
        backlight.set_brightness(40).unwrap();
        assert_eq!(read(&dir, "brightness"), 102);
        assert_eq!(backlight.brightness().unwrap(), 40);
    }

    #[test]
    fn off_and_on_through_bl_power_keep_the_brightness() {
        let root = tempfile::tempdir().unwrap();
        let dir = add_device(root.path(), "10-0045", "raw", 255, 128, true);
        let mut backlight = Sysfs::open(root.path(), None).unwrap();
        backlight.set_power(false).unwrap();
        assert_eq!((read(&dir, "bl_power"), read(&dir, "brightness")), (POWER_OFF, 128));
        backlight.set_power(true).unwrap();
        assert_eq!((read(&dir, "bl_power"), read(&dir, "brightness")), (POWER_ON, 128));
    }

    #[test]
    fn without_bl_power_off_is_brightness_zero_and_on_restores_it() {
        let root = tempfile::tempdir().unwrap();
        let dir = add_device(root.path(), "gpio", "raw", 1, 1, false);
        let mut backlight = Sysfs::open(root.path(), None).unwrap();
        backlight.set_power(false).unwrap();
        backlight.set_power(false).unwrap();
        assert_eq!(read(&dir, "brightness"), 0);
        assert_eq!(backlight.brightness().unwrap(), 100, "reports the brightness to come back to");
        backlight.set_power(true).unwrap();
        assert_eq!(read(&dir, "brightness"), 1);
    }

    #[test]
    fn brightness_set_while_off_waits_for_on() {
        let root = tempfile::tempdir().unwrap();
        let dir = add_device(root.path(), "pwm", "raw", 100, 80, false);
        let mut backlight = Sysfs::open(root.path(), None).unwrap();
        backlight.set_power(false).unwrap();
        backlight.set_brightness(30).unwrap();
        assert_eq!(read(&dir, "brightness"), 0);
        backlight.set_power(true).unwrap();
        assert_eq!(read(&dir, "brightness"), 30);
    }

    #[test]
    fn zero_max_brightness_is_an_error() {
        let root = tempfile::tempdir().unwrap();
        add_device(root.path(), "broken", "raw", 0, 0, true);
        assert!(matches!(Sysfs::open(root.path(), None), Err(BacklightError::Device { .. })));
    }

    #[cfg(unix)]
    #[test]
    fn read_only_brightness_is_a_permission_error() {
        use std::os::unix::fs::PermissionsExt;
        let root = tempfile::tempdir().unwrap();
        let dir = add_device(root.path(), "10-0045", "raw", 255, 255, true);
        fs::set_permissions(dir.join("brightness"), fs::Permissions::from_mode(0o444)).unwrap();
        // Root may write anyway; then there is nothing to check.
        if check_writable(&dir.join("brightness")).is_ok() {
            return;
        }
        assert_eq!(Sysfs::open(root.path(), None).unwrap_err(), BacklightError::PermissionDenied("10-0045".into()));
    }
}
