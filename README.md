<img src="assets/logo/freakymate.svg" alt="FreakyMate logo" width="96" align="right">

# FreakyMate

**A touch screen for Klipper, with Happy Hare and Spoolman built in.**

A touch screen interface for 3D printers running [Klipper](https://www.klipper3d.org/): a small native app that runs
on the printer's own host (Raspberry Pi, BTT CB1/CB2 and similar), fills the display attached to it and talks to
[Moonraker](https://moonraker.readthedocs.io/). It has built-in support for **[Happy Hare](https://github.com/moggieuk/Happy-Hare)
(MMU)** and **[Spoolman](https://github.com/Donkie/Spoolman)**.

Written in Rust with a [Slint](https://slint.dev/) UI: no browser, no WebView, no X11 desktop needed. The first target
is the BTT HDMI5 (5", 800x480, landscape).

> [!CAUTION]
> **Young software: use it with care and at your own risk.**
> FreakyMate has been tried on three different printers and works quite well there, so you are welcome to use it every
> day. It is still very young, though, and may well contain bugs; feedback and bug reports are very welcome (see
> [Contact and support](#contact-and-support)). It sends commands that move motors, heat up nozzles and beds and
> restart services, and a bug can damage your printer or your print or start a fire. Do not leave the printer
> unattended, and keep an emergency stop within reach.
> The software is provided "as is", **without any warranty and without any liability** of the author
> for damage of any kind, to the extent permitted by law (see sections 15 and 16 of the [GPL](LICENSE)).

![Print screen with Happy Hare and Spoolman](docs/screenshots/print.png)

|                                      |                                        |                                                            |
| ------------------------------------ | -------------------------------------- | ---------------------------------------------------------- |
| ![Files](docs/screenshots/files.png) | ![Move](docs/screenshots/move.png)     | ![Temperature](docs/screenshots/temperature.png)           |
| ![Tune](docs/screenshots/tune.png)   | ![System](docs/screenshots/system.png) | ![Klipper shut down](docs/screenshots/klippy-shutdown.png) |
| ![Console](docs/screenshots/console.png) | ![Filament](docs/screenshots/filament.png) | ![MMU (Happy Hare)](docs/screenshots/mmu.png)       |

## Features

- **Print**: progress, layers, ETA and filament; pause, resume, cancel and exclude objects on a map of the bed; a file
  list with thumbnails.
- **Move**: jog, home, Quad Gantry Level or Z-Tilt; a **Calibrate** tab for bed screws, Z offset and the bed mesh with
  a coloured map and profiles.
- **Thermal**: heaters with presets and temperature history, fans with a warning when one stops, sensors.
- **Light**: main light, LEDs and the effects of led_effect.
- **Tune**: Z offset and speed during a print; an Expert tab for flow, pressure advance, limits and retraction.
- **Filament**: extrude, retract, load and unload macros, filament sensors.
- **System**: restarts and power devices, updates, Wi-Fi and Ethernet, a full console with macros and forms for their
  parameters, display settings.
- **Happy Hare (MMU)**, 4.1 or newer: gates, filament path, buffer, FlowGuard and flow rate; select, load and eject
  gates, tool-to-gate map, EndlessSpool.
- **Spoolman**: the spool in use on the print page and at each gate, a spool picker with weighing, a preset from the
  spool and hints before a print.

Functions only appear when the printer has them. Every page in detail: [docs/features.md](docs/features.md).

## What to expect

|                    | FreakyMate                                                                                                        |
| ------------------ | ----------------------------------------------------------------------------------------------------------------- |
| Maturity           | Young, one developer, tried on three printers, English only                                                       |
| Technology         | Rust with Slint, one native binary; fullscreen under `cage` (Wayland) or as a normal window                       |
| Scope              | What a printer needs every day: print, move, heat, tune, filament, console and macros, system, settings, network |
| Printers and hosts | One Moonraker at a time, on the printer host or elsewhere; Wi-Fi settings only on the printer host               |
| Displays           | Works best from 5" at 800x480 (tested with the BTT HDMI5); other sizes are scaled, not tested yet                |
| Multi-material     | Happy Hare 4.1 or newer; gates, filament path, buffer, FlowGuard and flow rate                                   |
| Spoolman           | Woven into the print page, the temperature page and the MMU gates, with hints before a print                     |
| Updates            | Moonraker's update manager, with a `stable` and a `beta` channel                                                 |
| License            | GPL-3.0-or-later                                                                                                  |

Functions appear only where the printer has them, and the screen points out what looks unusual without forbidding it.
Feedback, bug reports and corrections are welcome, please open an issue.

## Quick start

On the printer host (Debian 12 or newer, Raspberry Pi OS Bookworm or newer, Ubuntu or Armbian 22.04 or newer, with
the display attached), run as the user running Klipper and Moonraker:

```bash
curl -fsSLO https://github.com/freakydude/freakymate/releases/latest/download/get-freakymate.sh
bash get-freakymate.sh -m http://127.0.0.1:7125
```

The script downloads the zip for your architecture from the [releases](https://github.com/freakydude/freakymate/releases)
and runs its installer; installing from the zip by hand works as well.

FreakyMate then starts fullscreen on the display at boot and updates through Moonraker's update manager, like Mainsail
or Fluidd. Release or pre-release, update channels, the Debian package and the desktop window:
[docs/install.md](docs/install.md). For thumbnails, layers and the layer pause see
[Slicer and macros](docs/printer-setup.md). To try it without a printer, see
[Try it without a printer](docs/development.md#try-it-without-a-printer).

## Documentation

| Page                                   | What it covers                                                                       |
| -------------------------------------- | ------------------------------------------------------------------------------------ |
| [Features](docs/features.md)           | Every page in detail, Happy Hare (MMU) and Spoolman                                  |
| [Install and update](docs/install.md)  | Release and pre-release, installer, update channels, Debian package, desktop use     |
| [Slicer and macros](docs/printer-setup.md) | Thumbnails, layers, excluding objects; macros for loading filament and pausing at a layer |
| [Configuration](docs/configuration.md) | Command line, settings file and temperature presets, Moonraker login, displays, network |
| [Development](docs/development.md)     | Build, simulated printer, checks and screenshots, CI, versions and releases          |
| [Architecture](docs/architecture.md)   | Structure, data flow and conventions of the code                                     |
| [Contributing](CONTRIBUTING.md)        | Pull requests and the Contributor License Agreement                                  |

## Contact and support

Want to get in touch, report a bug, discuss a feature or show what you built? Meet me and the others who try
FreakyMate on Discord. Bugs and feature requests are also welcome as GitHub issues, and pull requests too (see
[CONTRIBUTING.md](CONTRIBUTING.md)).

<a href="https://discord.gg/Ja2cxCcmJQ"><img src="https://img.shields.io/badge/Discord-join%20the%20chat-5865F2?style=for-the-badge&logo=discord&logoColor=white" alt="Join the Discord" height="40"></a>

FreakyMate is a private project that I work on in my spare time, for my own printer. It comes without support
promises, so answers may take a while. If you like it and want to say thanks, you can support me here:

<a href="https://ko-fi.com/freakydude"><img src="https://ko-fi.com/img/githubbutton_sm.svg" alt="Support me on Ko-fi" height="40"></a>
<a href="https://www.patreon.com/freakydude"><img src="https://c5.patreon.com/external/logo/become_a_patron_button.png" alt="Become a patron" height="40"></a>
<a href="https://www.buymeacoffee.com/freakydude"><img src="https://cdn.buymeacoffee.com/buttons/v2/default-yellow.png" alt="Buy me a coffee" height="40"></a>

## Thanks

FreakyMate builds on the work of many people. Thank you to

- [Klipper](https://github.com/Klipper3d/klipper) and [Moonraker](https://github.com/Arksine/moonraker), above all:
  the firmware and the API that FreakyMate talks to; without them it would have nothing to show.
- [Happy Hare](https://github.com/moggieuk/Happy-Hare) for the multi-material support that the MMU pages show and
  control.
- [Spoolman](https://github.com/Donkie/Spoolman) for keeping track of the spools.
- [Mainsail](https://github.com/mainsail-crew/mainsail), [KlipperScreen](https://github.com/KlipperScreen/KlipperScreen)
  and the [KlipperScreen Happy Hare Edition](https://github.com/moggieuk/KlipperScreen-Happy-Hare-Edition) for their
  work on running Klipper and Happy Hare from a browser and a touch screen.
- [Slint](https://slint.dev/) for the UI toolkit, and the Rust crates FreakyMate is built with, among them
  [Tokio](https://tokio.rs/), [reqwest](https://github.com/seanmonstar/reqwest),
  [tokio-tungstenite](https://github.com/snapview/tokio-tungstenite), [serde](https://serde.rs/),
  [image](https://github.com/image-rs/image), [zbus](https://github.com/dbus2/zbus) and
  [clap](https://github.com/clap-rs/clap); the [license](#license) section says where to find all of them.
- [#3DBenchy](https://www.3dbenchy.com/) (CC0) for the boat in the screenshots.

## License

GPL-3.0-or-later, see [LICENSE](LICENSE). Slint is used under its GPLv3 option. The packages list the licenses of
all dependencies in `THIRD-PARTY-LICENSES.html`.

- **Use is free for everyone**, companies included, for example on your own printers or in a print farm.
- **Building FreakyMate into a product** (preinstalled on printers, displays or kits) is possible under the GPL's
  conditions. If they do not fit your plans, I am open to a commercial license: see [COMMERCIAL.md](COMMERCIAL.md).
- **Name and logo** are not covered by the GPL; forks and products need their own name, see
  [TRADEMARKS.md](TRADEMARKS.md).
