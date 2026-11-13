# Requirements and hardware

What FreakyMate needs, which Klipper plugins it understands and what hardware is typically behind them. FreakyMate
reads the state that Klipper and its plugins report through Moonraker; the hardware itself does not matter to it. A
machine that one of these plugins drives is therefore covered, whatever its name.

## Requirements

| What              | Needed                                                                                                                                 |
| ----------------- | -------------------------------------------------------------------------------------------------------------------------------------- |
| Printer           | [Klipper](https://www.klipper3d.org/) with [Moonraker](https://moonraker.readthedocs.io/), as set up by KIAUH, MainsailOS and similar |
| Host system       | Debian 12 Bookworm or newer, Raspberry Pi OS Bookworm or newer (32 or 64 bit), Ubuntu 22.04 or newer, Armbian on these; glibc 2.35 or newer and the packages `cage` and `seatd` (the installer checks all of this). Debian 11 Bullseye is too old |
| Host hardware     | Raspberry Pi, BTT CB1/CB2 and similar boards: `aarch64`, `armv7` or `x86_64`                                                         |
| Display           | Any display the host drives (HDMI, DSI). Best at 5" or more with 800x480; other sizes are scaled. Small displays with only a framebuffer driver (SPI, `fbtft`) run through a fallback. Tested displays: see [below](#tested-setups) |
| Touch             | A touch screen; a mouse and keyboard work as well                                                                                     |
| Optional          | NetworkManager for the Wi-Fi settings, a controllable backlight for brightness and standby, the plugins below                         |

FreakyMate can also run as a window on a Linux or Windows desktop and talk to a Moonraker over the network (see
[Install and update](install.md#packages-and-desktop-use)).

## Plugins and typical hardware

Every plugin is optional and detected by itself; a printer without it looks as if the plugin did not exist.

| Plugin | What FreakyMate does with it | Typical hardware | Tested on |
| ------ | ---------------------------- | ---------------- | --------- |
| [Happy Hare](https://github.com/moggieuk/Happy-Hare) 4.1 or newer | Gates, filament path, buffer, FlowGuard, flow rate; select, load, eject; tool-to-gate map, EndlessSpool, service; several units on one printer | MMUs from its installer: ERCF, Tradrack, Box Turtle, Night Owl, QuattroBox, BTT ViViD, HTLF, EMU, Angry Beaver, 3MS, 3D Chameleon, Pico MMU, MMX, KMS, QIDI Box and custom designs | real printers with one unit (a custom type-B unit with one gate, an ERCF with 8 gates); several units against the simulator |
| [AFC](https://github.com/AFCProject/AFC-Klipper-Add-On) (Armored Turtle) | Lanes and the lane in use, tool map, service ("Back to hub", clear error); AFC's own tool changer | Box Turtle, Night Owl, QuattroBox, HTLF, EMU, ViViD, Claymore, OpenAMS; Snapmaker U1 through `AFC_Toolchanger` | simulator, built from AFC's source code |
| [klipper-toolchanger](https://github.com/viesturz/klipper-toolchanger) and [klipper-toolchanger-easy](https://github.com/jwellman80/klipper-toolchanger-easy) | Heads with their nozzle, spool and state; select, pick up and drop a head; initialize and recover the changer; also together with Happy Hare or AFC | [StealthChanger](https://github.com/DraftShift/StealthChanger), [TapChanger](https://github.com/viesturz/tapchanger), mostly on Voron printers | simulator, built from the plugins' source code |
| [Spoolman](https://github.com/Donkie/Spoolman) (Moonraker's `spoolman` component) | Spool in use, spool at each gate or head, spool picker with weighing, preset for the spool, hints before a print | any spools; Spoolman runs as a service in the network | real printers |
| [klipper-led_effect](https://github.com/julianschill/klipper-led_effect) | Turns the effects on and off | addressable LEDs such as Neopixels, WS2812, SK6812 | real printer |
| Moonraker's `wled` and `power` components | WLED strips and power devices on and off | WLED controllers, smart plugs, relays | real printers |

Happy Hare and AFC were read at the versions named in [multi-tool.md](multi-tool.md#what-each-source-reports); the
fake printer used for testing (see [Development](development.md#try-it-without-a-printer)) follows their source code
field by field.

**Conventions FreakyMate follows without a plugin.** Many configurations share macros by name. FreakyMate calls them
like any other macro and reads back only what they declare openly (details in [Slicer and macros](printer-setup.md)):

- the layer pause of [mainsail-config](https://github.com/mainsail-crew/mainsail-config) (`SET_PAUSE_AT_LAYER` and
  friends),
- `LOAD_FILAMENT` and `UNLOAD_FILAMENT`,
- tool macros `T0` … `Tn` with the variables `active`, `color` and `spool_id`, as Fluidd and Mainsail use them, for
  printers with several extruders, IDEX or a nozzle changer built from macros.

Klipper's own `[extruder1]` and `[dual_carriage]` (IDEX) need no plugin; FreakyMate shows the heater of every
extruder, but no IDEX modes yet.

## Tested setups

FreakyMate runs on these printers; everything else has been tried against the simulated printer only.

| Printer                         | Plugins                                                                          |
| ------------------------------- | -------------------------------------------------------------------------------- |
| Voron 2.4 R2                    | Happy Hare, a custom type-B unit with one gate; Spoolman, led_effect, a power device |
| Artillery Sidewinder (modified) | Spoolman, a power device                                                         |
| Voron 2.4 R2 of a friend        | Happy Hare with an ERCF of 8 gates, WLED                                         |

Displays: BTT HDMI5 (5", 800x480, HDMI), Raspberry Pi Touch Display v1 (7", 800x480, DSI) and a TFT35 SPI display
(3.5", through the framebuffer fallback, on the Sidewinder).

## Not supported yet

- **Languages:** the screen is in English only.
- **IDEX modes** (copy, mirror) and which head sits on which carriage.
- **KTC** by TypQxQ (`ktc` objects): not read; tool macros `T<n>` still work.
- **Kalico** (the Klipper fork) has not been tried.
- **Touch rotation** in the framebuffer fallback; rotate the display in its driver.
- **HTTPS** to Moonraker: FreakyMate talks plain HTTP and WebSocket, like most home installations.
- **Windows installer:** on Windows FreakyMate runs from the unpacked zip.
- Own layouts for 7" and 4.3" displays are planned; until then the 800x480 layout is scaled.

Missing something for your printer? Open a [hardware support issue](https://github.com/freakydude/freakymate/issues/new/choose)
with the diagnostic report (see [Bug reports](configuration.md#bug-reports)).

## Privacy

FreakyMate talks only to the Moonraker you set up, and on the printer host to NetworkManager for the Wi-Fi settings.
Spoolman, power devices and WLED are reached through Moonraker. There is no telemetry and no call to any other
server. Updates are downloaded by Moonraker's update manager from the GitHub releases, as for Mainsail and Fluidd.
The log and the diagnostic report stay on the printer until you share them; what they contain is listed under
[Log](configuration.md#log) and [Bug reports](configuration.md#bug-reports).
