# Install and update

FreakyMate is young software: the note in the [README](../README.md) applies to every build and every way of
installing it. What the host needs: [Requirements and hardware](hardware.md#requirements).

Ways to run it: on the [printer host](#printer-host) (the main use case), on an [Android](#android) phone or tablet, as
[Debian or Arch package](#packages), or in a [desktop window](#desktop-linux-windows) on Linux or Windows.

## Which build

The [releases page](https://github.com/freakydude/freakymate/releases) offers two kinds of builds, and every open pull
request is built as well:

| Build            | What it is                                                                          | Where                                                                                            |
| ---------------- | ----------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------ |
| **Release**      | A tagged state of `main` (`vX.Y.0`), when a set of changes feels finished. Fewer updates, more time in use. | [Releases](https://github.com/freakydude/freakymate/releases/latest), update channel `stable` |
| **Pre-release**  | Always the latest `main`, replaced by every merged change. The newest functions first.                      | [Releases](https://github.com/freakydude/freakymate/releases), update channel `beta`          |
| **Pull request** | The state of an open pull request, to try a change before it is merged.             | Artifacts of the pull request's CI run (needs a GitHub login), see below                      |

Neither release nor pre-release is an untested nightly: a change only reaches `main` through a reviewed pull request
whose automated checks (tests, builds and packages for all targets) are green, and a release is a state that was the
pre-release before. Beyond that, testing on real printers is limited to a few machines (see
[Requirements and hardware](hardware.md)), so the note in the README applies to both builds. Pick the release if you
want few updates, the pre-release if you want to follow the development; the
[update channel](#updates-and-channels) decides which one Moonraker installs.

**Builds of a pull request.** The packages (`packages-<target>`, the same zips, `.deb` and Arch packages as a release)
are listed under Artifacts on the page of its CI run; a pull request from a branch of this repository also gets a
comment with the link. Downloading them needs a GitHub login. Install such a zip by hand as described under
[Printer host](#printer-host). Moonraker does not offer pull request builds as updates: on the `beta` channel the next
pre-release replaces it, on `stable` the next release.

## Printer host

This is the main use case: FreakyMate runs on the host of the printer (Raspberry Pi OS, Armbian or another
Debian- or Ubuntu-based system with the display attached), starts fullscreen on the display at boot and updates itself
through Moonraker's update manager, like Mainsail or Fluidd.

**Supported systems.** One build per architecture runs on every system that has glibc 2.35 or newer and the kiosk
packages `cage` and `seatd`:

| System                                    | Status                                                                 |
| ----------------------------------------- | ---------------------------------------------------------------------- |
| Debian 12 Bookworm, 13 Trixie             | supported                                                              |
| Raspberry Pi OS Bookworm or newer         | supported (32-bit and 64-bit)                                          |
| Ubuntu 22.04, 24.04 and Armbian on them   | supported; `cage` and `seatd` come from `universe`                     |
| Armbian Bookworm or Trixie                | supported (Armbian uses Debian's packages)                             |
| Debian 11 Bullseye and images based on it | not supported: it has neither `cage` nor `seatd`; update the system    |

Builds for `aarch64-unknown-linux-gnu` (64-bit, e.g. CB1/CB2, Raspberry Pi 4/5 with a 64-bit system),
`armv7-unknown-linux-gnueabihf` (32-bit systems) and `x86_64-unknown-linux-gnu`. The installer checks glibc,
architecture and packages first and stops with a message if something does not fit.

1. Download and start the installer as your normal user (the one running Klipper and Moonraker). The script picks
   the zip for your system's architecture, unpacks it to a temporary directory, runs `install.sh` with your options
   and removes the temporary directory again:

   ```bash
   curl -fsSLO https://github.com/freakydude/freakymate/releases/latest/download/get-freakymate.sh
   bash get-freakymate.sh -m http://127.0.0.1:7125     # add -c beta for the pre-release channel
   ```

2. Or by hand: download the zip for your architecture from the
   [releases](https://github.com/freakydude/freakymate/releases) (release or pre-release, see [Which build](#which-build)), unpack it and
   run the installer:

   ```bash
   unzip freakymate-aarch64-unknown-linux-gnu.zip -d freakymate-setup
   cd freakymate-setup && ./install.sh -m http://127.0.0.1:7125
   ```

   The installer copies everything to `~/freakymate`; the unpacked `freakymate-setup` can be deleted afterwards.

   The installer
   - installs the packages from `system-dependencies.json` (among them `cage`, a minimal Wayland kiosk compositor),
   - copies the program to `~/freakymate` and creates the systemd service `freakymate` (fullscreen on `tty1`, through
     the kiosk compositor `cage`). On systems without a DRM/KMS device it uses the
     [framebuffer fallback](#framebuffer-fallback) instead. Run the installer again after changing the display,
   - allows brightness and screen standby for the group `video` (udev rule `70-freakymate-backlight.rules`; only
     displays with a controllable backlight have one, see [Configuration](configuration.md#brightness-and-standby)),
   - registers it with Moonraker's update manager (`[update_manager freakymate]` in `moonraker.conf`, service in
     `moonraker.asvc`) and restarts Moonraker.

   Options: `-m <moonraker url>`, `-d <printer_data folder>` (default `~/printer_data`), `-c stable|beta` (update channel, see [Updates and channels](#updates-and-channels)).
   Run again, it keeps the address and folder of the existing installation unless you give them.
   Settings go to `printer_data/config/freakymate.conf` (TOML), next to the Klipper configs, so they can be edited in
   Mainsail or Fluidd (see [Configuration](configuration.md#settings-file)). The log is `printer_data/logs/freakymate.log` (Mainsail: Machine → logs), see [Configuration](configuration.md#log).
   To remove it again, see [Uninstall](#uninstall).

### Framebuffer fallback

`cage` needs a DRM/KMS device (`/dev/dri/card*`). Small displays with only a framebuffer driver (SPI, fbtft, e.g.
`fb_st7796s`) have none. On such a system the installer sets up the service differently: FreakyMate draws straight to
`/dev/fbN` without `cage` (`--framebuffer`, software rendering, see [Configuration](configuration.md#command-line)), and
the text console is detached from the framebuffer while it runs. Rotate such a display in its driver (e.g.
`armbianEnv.txt`); touch rotation is not handled yet. This is a fallback: HDMI and DSI displays use the normal kiosk.

## Android

An old Android phone or tablet near the printer can serve as touch screen: the same UI, in landscape. It needs Android
7.0 or newer on a 64-bit ARM device, and Moonraker must be reachable over the home network (plain `http`/`ws`). So far
it has been tried in the Android emulator and on a real phone with a printer, not yet on many devices.

1. Download `freakymate-<version>-arm64-v8a-prod-release.apk` from the latest
   [pre-release](https://github.com/freakydude/freakymate/releases) and open it on the device; allow installing from
   unknown sources when Android asks.
2. Updates: add the repository `freakydude/freakymate` to [Obtainium](https://github.com/ImranR98/Obtainium) and enable
   pre-releases.

For a bug report, Settings has "Export log": it copies the log of the last days into one text file you can attach to the
issue. On Android 10 and newer it lands in `Download/FreakyMate`; on Android 7 to 9 in the app's own folder
(`Android/data/de.freakydu.freakymate/files/Download`), which the screen names after the export.

The APK is signed with a release key; `apksigner verify --print-certs <apk>` shows the SHA-256 fingerprint of its
certificate, which must be <!-- TODO Frank: SHA-256 fingerprint of the release certificate -->.

The APK in the artifacts of a pull request (`…-dev-release.apk`) is a test build: it installs as a separate app,
FreakyMate Dev, with its own settings, and leaves an installed FreakyMate alone.

## Packages

Besides the Moonraker route there are plain packages for Debian-based and Arch-based systems:

- **Debian package**: each release contains `freakymate_<version>_<arch>.deb` (`amd64`, `arm64`, `armhf`). apt installs
  the dependencies and removes the package cleanly, but Moonraker does **not** update it, install newer `.deb` files
  yourself. Use either the zip installer or the package, not both (they would compete for `tty1`).

  ```bash
  sudo apt install ./freakymate_<version>_arm64.deb
  sudo systemctl disable getty@tty1                # free tty1 for the kiosk
  sudo usermod -aG video $USER                  # brightness and standby, if the display has a backlight
  sudo usermod -aG i2c $USER                    # the same for a monitor on DDC/CI, with `ddcutil` installed
  sudo systemctl enable --now freakymate@$USER  # fullscreen on tty1 as this user
  # remove again (stops and disables all freakymate@ instances)
  sudo apt remove freakymate && sudo systemctl enable getty@tty1
  ```

- **Arch Linux package**: each release also contains `freakymate-<version>-1-<arch>.pkg.tar.zst` for `x86_64`,
  `aarch64` and `armv7h` (Arch Linux ARM). It is built in CI from the same binary as the `.deb`, with the same files and kiosk
  service, and has had little testing. There is no repository: download the file and update by installing the next one.
  As with the `.deb`, use either the zip installer or the package, not both (they would compete for `tty1`).

  ```bash
  sudo pacman -U freakymate-<version>-1-x86_64.pkg.tar.zst
  sudo systemctl disable getty@tty1                # free tty1 for the kiosk
  sudo systemctl enable --now freakymate@$USER     # fullscreen on tty1 as this user
  # remove again
  sudo pacman -R freakymate && sudo systemctl enable getty@tty1
  ```

## Desktop (Linux, Windows)

On a desktop (Linux PC, Raspberry Pi OS with desktop, Windows) the program opens as a normal window and talks to any
Moonraker over the network. On Debian and Ubuntu install the [`.deb`](#packages) and start FreakyMate from the
application menu or with `freakymate` in a terminal (skip the `systemctl` lines, they set up the kiosk); on Windows
unpack `freakymate-x86_64-pc-windows-msvc.zip` and run `freakymate.exe`. There is no Windows installer yet. Set the
Moonraker address on System → Settings or with `-m` (default `http://127.0.0.1:7125`). For fullscreen with a mouse,
switch on Fullscreen on System → Settings → Display; `-f` is the kiosk mode for a touch screen and hides the mouse
pointer.

## Updates and channels

After the installation FreakyMate shows up in the update list of Mainsail, Fluidd and its own System page.
Moonraker downloads the matching zip from the GitHub releases, replaces `~/freakymate` and restarts the service.
The section written by the installer (add it yourself if `moonraker.conf` lives elsewhere):

```ini
[update_manager freakymate]
type: zip
channel: stable          # beta = also the pre-release (current main)
repo: freakydude/freakymate
path: ~/freakymate
managed_services: freakymate
system_dependencies: system-dependencies.json
```

| Channel  | What you get                                                                                                                        |
| -------- | ----------------------------------------------------------------------------------------------------------------------------------- |
| `stable` | Only releases `vX.Y.0`. Few updates, more time in use.                                                                              |
| `beta`   | In addition the pre-release of **every** change on `main` (`vX.Y.Z`, replaced by the next one). New functions first.               |

To change the channel set `channel:` in that section and restart Moonraker (`sudo systemctl restart moonraker`). The
installer only writes the section once: re-running `install.sh` with `-c beta` does not change an existing `channel`
line, so edit it by hand. Your edits survive later updates, because Moonraker only replaces the program folder.

Because of that, what the installer set up outside the program folder (the service and its options, the udev and
polkit rules, the groups) stays as it was. When a release needs more there, the Updates tab of the System page says
so: run `~/freakymate/install.sh` once. Without `-m` and `-d` it keeps the Moonraker address and the `printer_data`
folder of the existing installation; your settings stay. The Debian package brings everything along with each
version.

## Uninstall

Installed with the installer:

```bash
~/freakymate/uninstall.sh            # add -d <printer_data folder> if it is not ~/printer_data
```

It stops and removes the service, the udev and polkit rules and `~/freakymate`, removes the
`[update_manager freakymate]` section from `moonraker.conf` and the entry in `moonraker.asvc`, restarts Moonraker and
gives `tty1` back to the login console. It keeps the system packages (`cage`, `seatd`, …), the settings
`printer_data/config/freakymate.conf`, the log and a stored Moonraker login (`~/.config/freakymate/credentials.json`);
delete those by hand if you no longer need them.

Installed as Debian package: `sudo apt remove freakymate && sudo systemctl enable getty@tty1` (see [Packages](#packages)).
