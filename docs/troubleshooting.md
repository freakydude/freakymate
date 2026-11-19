# Troubleshooting

Common problems and where to look first. If none of this helps, ask on [Discord](https://discord.gg/Ja2cxCcmJQ) or
open an issue with the diagnostic report and the verbose log (see [Bug reports](configuration.md#bug-reports)).

## Where is the log?

`printer_data/logs/sailhand.log`, next to `klippy.log`; Mainsail and Fluidd list it under Machine → logs. The
service also writes to the journal:

```bash
systemctl status sailhand                 # installed with install.sh
journalctl -u sailhand -b                 # this boot; with the .deb: sailhand@<user>
```

For more detail switch on `verbose_log` (see [Log](configuration.md#log)).

## The display stays black or shows the text console

- Check that the service runs: `systemctl status sailhand`, and read the end of the log.
- Changed the display after installing? Run `~/sailhand/install.sh` again; it chooses between the normal kiosk
  (`cage`, needs `/dev/dri/card*`) and the framebuffer fallback for displays without DRM/KMS (see
  [Install](install.md#framebuffer-fallback)).
- Something else on `tty1`? The installer disables the login console there; a second kiosk (for example a Debian
  package and the installer at the same time) competes for it. Use one of the two.
- Graphics driver trouble: the environment variable `SLINT_BACKEND=winit-software` forces software rendering (see
  [Command line](configuration.md#command-line)).

## "Cannot connect" or "Moonraker wants a login"

- Check the address of the printer on System → Printers (or `-m` on the command line). On the printer host it is
  usually `http://127.0.0.1:7125`.
- Check that Moonraker runs: `systemctl status moonraker`.
- If Moonraker asks for a login, log in on the screen or use an API key, see
  [Moonraker login](configuration.md#moonraker-login). On the printer host Moonraker usually trusts `localhost`.

## Everything is too small or too large

The layout is made for 800x480 and scaled to other sizes in fullscreen. `SLINT_SCALE_FACTOR` sets the scale by hand
(see [Displays](configuration.md#displays)).

## Touch is rotated or off

Sailhand has no setting for touch rotation yet; the [framebuffer fallback](install.md#framebuffer-fallback) says what
applies there. Please tell me your display on Discord or in a
[hardware support issue](https://github.com/freakydude/sailhand/issues/new/choose).

## A function is missing

Functions only appear when the printer has them. Some common cases:

- **Wi-Fi settings** need NetworkManager and Moonraker on the same host, and the user in the group `netdev` (see
  [Network settings](configuration.md#network-settings)).
- **Brightness and standby** need a display with a controllable backlight and the user in the group `video` (see
  [Brightness and standby](configuration.md#brightness-and-standby)).
- **Thumbnails, layers and excluding objects** depend on the slicer settings (see [Slicer and macros](printer-setup.md)).
- **Happy Hare** needs version 4.1 or newer; with an older one the screen shows a notice instead of the gates.
- **Load and unload** buttons need the macros `LOAD_FILAMENT` and `UNLOAD_FILAMENT` (see [Macros](printer-setup.md#macros)).

## Sailhand is missing from the update list

The installer adds `[update_manager sailhand]` to `moonraker.conf`. If your `moonraker.conf` lives elsewhere, add
the section by hand (see [Updates and channels](install.md#updates-and-channels)). When an update needs more than the
program folder, the Updates tab of the System page says so: run `~/sailhand/install.sh` once.
