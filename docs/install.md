# Install and update

FreakyMate is young software: the caution in the [README](../README.md) applies to every build and every way of
installing it.

## Release and pre-release

The [releases page](https://github.com/freakydude/freakymate/releases) offers two kinds of builds:

| Build           | What it is                                                                                                                                  |
| --------------- | ------------------------------------------------------------------------------------------------------------------------------------------- |
| **Pre-release** | Always the current state of `main`. Every merged change replaces it, so it has the newest functions first.                                 |
| **Release**     | From time to time I tag a state of `main` as release `vX.Y.0`, when a set of changes feels finished. Fewer updates, more time in use.       |

Neither is an untested nightly: a change only reaches `main` through a reviewed pull request whose automated checks
(tests, builds and packages for all targets) are green, and a release is a state that was the pre-release before.
Beyond that, "tested" means tried on three different printers, where it works quite well; other setups may still
show bugs, so the caution applies to both builds. Pick the release if you want few updates, the
pre-release if you want to follow the development; the [update channel](#updates-and-channels) decides which one
Moonraker installs.

## Install on the printer host

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
   the zip for your system's architecture, unpacks it to `~/freakymate-setup` and runs `install.sh` with your options:

   ```bash
   curl -fsSLO https://github.com/freakydude/freakymate/releases/latest/download/get-freakymate.sh
   bash get-freakymate.sh -m http://127.0.0.1:7125     # add -c beta for the pre-release channel
   ```

2. Or by hand: download the zip for your architecture from the
   [releases](https://github.com/freakydude/freakymate/releases) (release or pre-release, see above), unpack it and
   run the installer:

   ```bash
   unzip freakymate-aarch64-unknown-linux-gnu.zip -d freakymate-setup
   cd freakymate-setup && ./install.sh -m http://127.0.0.1:7125
   ```

   The installer
   - installs the packages from `system-dependencies.json` (among them `cage`, a minimal Wayland kiosk compositor),
   - copies the program to `~/freakymate` and creates the systemd service `freakymate` (fullscreen on `tty1`),
   - registers it with Moonraker's update manager (`[update_manager freakymate]` in `moonraker.conf`, service in
     `moonraker.asvc`) and restarts Moonraker.

   Options: `-m <moonraker url>`, `-d <printer_data folder>` (default `~/printer_data`), `-c stable|beta` (update channel, see below).
   Settings go to `printer_data/config/freakymate.json`, next to the Klipper configs, so they can be edited in
   Mainsail or Fluidd (see [Configuration](configuration.md)). An existing `~/.config/freakymate/settings.json` is
   copied there once.
   Remove everything with `~/freakymate/uninstall.sh`; the settings file stays.

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

## Packages and desktop use

Besides the Moonraker route there are plain packages, for example to try it on a desktop or any other device:

- **Debian package**: each release contains `freakymate_<version>_<arch>.deb` (`amd64`, `arm64`, `armhf`). apt installs
  the dependencies and removes the package cleanly, but Moonraker does **not** update it, install newer `.deb` files
  yourself. Use either the zip installer or the package, not both (they would compete for `tty1`).

  ```bash
  sudo apt install ./freakymate_0.3.0_arm64.deb
  sudo systemctl disable getty@tty1                # free tty1 for the kiosk
  sudo systemctl enable --now freakymate@$USER  # fullscreen on tty1 as this user
  # remove again (stops and disables all freakymate@ instances)
  sudo apt remove freakymate && sudo systemctl enable getty@tty1
  ```

- **Windowed mode on a desktop** (Linux PC, Raspberry Pi OS with desktop, Windows): the program opens as a normal window
  and talks to any Moonraker over the network. On Debian and Ubuntu install the `.deb` and start FreakyMate from the
  application menu or with `freakymate` in a terminal (skip the `systemctl` lines above, they set up the kiosk); on Windows unpack
  `freakymate-x86_64-pc-windows-msvc.zip` and run `freakymate.exe`. There is no Windows installer yet. Set the
  Moonraker address on System → Display or with `-m` (default `http://127.0.0.1:7125`). For fullscreen with a mouse,
  switch on Fullscreen on System → Display; `-f` is the kiosk mode for a touch screen and hides the mouse pointer.
