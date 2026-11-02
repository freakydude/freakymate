# Configuration

Most settings are changed on the screen (System → Display). This page covers the command line, the settings file,
the Moonraker login, displays and the network settings.

## Command line

```text
freakymate [--moonraker <url>] [--fullscreen] [--settings <file>] [--logfile <file>] [--simulate-network]
```

| Option               | Meaning                                                                                          |
| -------------------- | ------------------------------------------------------------------------------------------------ |
| `-m, --moonraker`    | Moonraker address for this start, e.g. `http://printer.local:7125` (otherwise the saved setting) |
| `-f, --fullscreen`   | Kiosk mode for a touch screen, as the services start it: fullscreen, no window frame, no mouse pointer |
| `--settings`         | Settings file, default `~/.config/freakymate/settings.json` (Windows: `%APPDATA%`). The installer passes `printer_data/config/freakymate.json` |
| `-l, --logfile`      | Log file, rotated daily like those of Klipper and Moonraker (default: stderr only). The installer passes `printer_data/logs/freakymate.log` |
| `--simulate-network` | Simulated Wi-Fi/Ethernet instead of NetworkManager (development builds only, see [Development](development.md#development-builds-and-releases)) |

Useful environment variables: `RUST_LOG=debug` (logging), `SLINT_BACKEND=winit-software` (force software rendering if the GPU
driver causes trouble), `SLINT_SCALE_FACTOR=1.5` (sets the scale by hand and turns the fitting to the display off, also in fullscreen).
Without a desktop or compositor the optional cargo feature `linuxkms` renders directly via DRM/KMS (experimental in Slint).

## Settings file

FreakyMate keeps its settings in a JSON file: `printer_data/config/freakymate.json` when installed with the installer
(so it can be edited in Mainsail or Fluidd next to the Klipper configs), otherwise `~/.config/freakymate/settings.json`
(Windows: `%APPDATA%\freakymate\settings.json`). The file is created with the defaults at the first start. Most entries are
changed on the screen; the **temperature presets** only here:

```json
"temperature_presets": [
  { "name": "PLA", "extruder": 210.0, "bed": 60.0 },
  { "name": "PETG", "extruder": 240.0, "bed": 80.0 },
  { "name": "ABS", "extruder": 250.0, "bed": 105.0 },
  { "name": "TPU", "extruder": 225.0, "bed": 50.0 }
]
```

The Thermal page shows the first four. Restart FreakyMate after editing (`sudo systemctl restart freakymate`); it
writes the file itself when a setting changes on the screen, so edit it while FreakyMate is stopped or restart right
after saving.

## Log

With the installer or the `.deb` the log is `printer_data/logs/freakymate.log`, next to `klippy.log` and
`moonraker.log`, so Mainsail and Fluidd list it under Machine → logs. It rotates at midnight, keeps three old files
(`freakymate.log.YYYY-MM-DD`) and also goes to the journal (`journalctl -u freakymate@<user>`). Each file starts with a
header: FreakyMate version, command line, system, then the versions of Moonraker, Klipper and Happy Hare and what was
detected on the printer (power devices, Spoolman, LEDs, macros and so on). Attach `freakymate.log`, `klippy.log` and
`moonraker.log` to a bug report.

Normally only the important lines are logged. For a bug report set `"verbose_log": true` in the settings file and
restart; FreakyMate then logs its debug messages too. `RUST_LOG` (for example `RUST_LOG=freakymate_core=trace`) takes
precedence over the setting.

## Moonraker login

Usually Moonraker trusts the machine FreakyMate runs on (`trusted_clients` in `moonraker.conf`, on the printer host
`localhost`), and nothing needs to be set up. If it does not, for example with FreakyMate on another machine or with
`force_logins: True`, the screen says that Moonraker wants a login and offers one:

- **Log in on the screen** with a Moonraker user. FreakyMate keeps only Moonraker's refresh token, not the password,
  and logs in again with it at every start until it expires (`login_timeout` in `moonraker.conf`, 90 days by default)
  or the user logs out; then the screen asks again. Logging out (Settings tab, or the same user logging out in
  Mainsail or Fluidd) ends every login of that user, so a user of its own for the display avoids surprises.
- **API key**, entered by hand in the credentials file below. Moonraker hands it out to an authorized client, for
  example on the printer host, which it usually trusts: `curl http://localhost:7125/access/api_key`.

The credentials are kept per Moonraker address in `~/.config/freakymate/credentials.json` (Windows:
`%APPDATA%\freakymate\credentials.json`), readable by the owner only. They stay there also when the settings lie in
`printer_data/config`, because Moonraker serves that folder and it is often backed up to a git repository.

```json
{
  "moonraker": {
    "http://printer.local:7125": { "api_key": "<Moonraker's API key>" }
  }
}
```

FreakyMate talks plain HTTP and WebSocket to Moonraker so far (no HTTPS), like most installations in a home network;
the password of a login goes over the network unencrypted.

## Displays

FreakyMate works best on a display of 5" or more at 800x480, such as the BTT HDMI5: the layout is made for that size and
it is the one it has been tested with. In fullscreen, other resolutions show the same layout scaled up or down to fit, so
a 7" 1024x600 display fills its screen and a smaller one, say a 4.3" 480x272, shows everything, only smaller. A window on
a desktop is never smaller than 800x480; a larger one shows more room instead of scaling (where a compositor forces a
smaller window, the layout is fitted as in fullscreen). Other sizes are not tested yet. Whether a small display is
comfortable to read and touch depends on the display and on you, so it is worth a try. Layouts of their own for the 7"
and 4.3" displays are planned.

## Network settings

Moonraker can only report the host's network (`machine.system_info`), it cannot change it. FreakyMate therefore talks
to NetworkManager over D-Bus. Managing Wi-Fi (open, WPA/WPA2, WPA3, hidden, remove saved networks,
on/off) and Ethernet (status, connect/disconnect) is offered when

- FreakyMate runs on Linux with NetworkManager (default on Raspberry Pi OS Bookworm and newer), and
- Moonraker runs on the same host (`localhost`/`127.0.0.1`), so the changed network is the printer's network.

The user needs the polkit rule `50-freakymate.rules` (allows the NetworkManager actions for the group `netdev`).
`install.sh` installs it to `/etc/polkit-1/rules.d` and adds the user to `netdev`; the Debian package ships it in
`/usr/share/polkit-1/rules.d`, add the user yourself: `sudo usermod -aG netdev $USER` (takes effect after re-login or a
service restart). Changing the network while printing asks for confirmation, because the connection may drop.

On Windows, on Linux without NetworkManager or with a remote Moonraker the section only shows the printer host's
interfaces and addresses read from Moonraker.

## Brightness and standby

On displays whose backlight Linux can control (`/sys/class/backlight`: DSI displays such as the Raspberry Pi Touch
Display, SPI displays with a backlight pin, laptops) the light panel of the status bar has a row **Display**: the
slider sets the brightness (10 to 100 %), **Off** switches the screen off at once. Settings → Display and console sets
when the screen goes off by itself, without a touch, once while idle and once while printing (pause included). Both are
**Never** by default.

A dark screen wakes up with the next touch; that touch does nothing else, nor do further touches in the next half
second. The printer wakes it too: a print starts or ends, pauses or fails, Klipper shuts down, the connection is lost or
Klipper answers with an error (`!!`).

Displays on HDMI, such as the BTT HDMI5, keep their backlight on as long as they get a signal and have no
`/sys/class/backlight`; there FreakyMate shows neither the slider nor the standby settings.

Changing the backlight needs the udev rule `70-freakymate-backlight.rules` (makes `brightness` and `bl_power` writable
for the group `video`) and the user in `video`. `install.sh` sets up both; the Debian package ships the rule in
`/lib/udev/rules.d`, add the user yourself: `sudo usermod -aG video $USER` (takes effect after a service restart). An
installation from before this version gets the rule by running `install.sh` again; updates through Moonraker only
replace the program folder.

In the settings file:

```json
"backlight": {
  "device": null,
  "brightness": null,
  "off_when_idle_minutes": null,
  "off_when_printing_minutes": null
}
```

`device` picks a device by its name under `/sys/class/backlight` (for example `10-0045` or `intel_backlight`); without
one FreakyMate takes the first by type (firmware, platform, raw). `brightness` is the last value set on the screen in
percent, set again at the start; `null` leaves the brightness as the system set it. The minutes are `null` for never.
The log names every backlight found and the one used, and says why the screen went off or on.
