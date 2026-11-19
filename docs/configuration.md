# Configuration

Most settings are changed on the screen (System → Settings). This page covers the command line, the settings file,
the Moonraker login, displays and the network settings.

## Command line

```text
sailhand [--moonraker <url>] [--fullscreen] [--framebuffer] [--settings <file>] [--logfile <file>] [--report [<file>]] [--simulate-network] [--simulate-backlight]
```

| Option               | Meaning                                                                                          |
| -------------------- | ------------------------------------------------------------------------------------------------ |
| `-m, --moonraker`    | Moonraker address used when no printer is chosen yet, e.g. `http://printer.local:7125` (otherwise `http://127.0.0.1:7125`); once a printer is chosen, that one wins |
| `-f, --fullscreen`   | Kiosk mode for a touch screen, as the services start it: fullscreen, no window frame, no mouse pointer |
| `--framebuffer`      | Fallback for displays without DRM/KMS (Linux): draws straight to `/dev/fbN` without cage, implies `--fullscreen`. See [Framebuffer fallback](install.md#framebuffer-fallback) |
| `--settings`         | Settings file (INI, like `moonraker.conf`), default `~/.config/sailhand/settings.conf` (Windows: `%APPDATA%`). The installer passes `printer_data/config/sailhand.conf` |
| `-l, --logfile`      | Log file, rotated daily like those of Klipper and Moonraker (default: stderr only). The installer passes `printer_data/logs/sailhand.log` |
| `--report [FILE]`    | Writes a diagnostic report `sailhand-report.json` and exits, see [Bug reports](#bug-reports). Without FILE it goes to `printer_data/logs` (when the settings are in `printer_data/config`), otherwise to the current folder; FILE may also be a folder |
| `--simulate-network` | Simulated Wi-Fi/Ethernet instead of NetworkManager (development builds only, see [Development](development.md#development-builds-and-releases)) |
| `--simulate-backlight` | Simulated backlight with brightness and standby (development builds only) |

Useful environment variables: `RUST_LOG=debug` (logging), `SLINT_BACKEND=winit-software` (force software rendering if the GPU
driver causes trouble), `SLINT_SCALE_FACTOR=1.5` (sets the scale by hand and turns the fitting to the display off, also in fullscreen).

## Settings file

Sailhand keeps its settings in a file with the ending `.conf`, written like `moonraker.conf` and `printer.cfg`:
`printer_data/config/sailhand.conf` when installed with the installer or the `.deb` (so it can be edited in Mainsail or
Fluidd next to the Klipper configs), otherwise `~/.config/sailhand/settings.conf` (Windows:
`%APPDATA%\sailhand\settings.conf`). Test builds (built locally or for a pull request; their version has a suffix such
as `-dev` or `-pr.…`) use the folder `sailhand-dev` instead, so trying one leaves the installed Sailhand alone; the
same holds for the credentials below. The file is created at the first start with every `[sailhand]` option and the four
default presets (PLA 210/60, PETG 240/80, ABS 250/105, TPU 225/50).

The format is the INI dialect of Klipper and Moonraker, so Mainsail highlights it like the other configs:

- Sections are `[kind]` or `[kind name]`. Options are `key: value` (Sailhand writes `:`; `=` is read too). Keys are not
  case-sensitive.
- Full-line comments start with `#` or `;`. A comment after a value needs whitespace before the `#` or `;`.
- Lists are comma-separated or one entry per indented line.
- Booleans are `True` and `False` (`yes`/`no`, `on`/`off` and `1`/`0` are read too).

The file has two parts. Above the `SAVE_CONFIG` line is what you set, on the screen or by hand. Below it is what
Sailhand keeps for itself, as in Klipper's `printer.cfg`:

```ini
[sailhand]
fullscreen: True
keep_screen_on: False
hide_temperature_reports: True
verbose_log: False

[printer voron]
moonraker: http://192.168.40.6:7125
hidden_macros:
    LOAD_FILAMENT
    UNLOAD_FILAMENT

[printer ratrig]
moonraker: http://ratrig.local:7125

[printer werkstatt]
moonraker: https://192.168.40.20:7130

[preset PLA]
extruder: 210
bed: 60

[preset PETG]
extruder: 240
bed: 80

[step_choices]
move_mm: 0.1, 1, 10, 50, 100

[backlight]
brightness: 80
off_when_idle_minutes: 5

#*# <---------------------- SAVE_CONFIG ---------------------->
#*# DO NOT EDIT THIS BLOCK OR BELOW. The contents are auto-generated.
#*#
#*# [sailhand]
#*# printer = voron
#*#
#*# [selected_steps]
#*# move_mm = 1
#*# z_offset_mm = 0.025
#*# temperature = 5
#*# extrude_mm = 10
#*# extrude_speed = 5
#*#
#*# [printer voron]
#*# uuid = 29f7565a940247049493e8ff2a0d0c87
#*# known_macros = PARK, LOAD_FILAMENT, UNLOAD_FILAMENT
#*#
#*# [printer ratrig]
#*# known_macros = PARK
```

**Above the SAVE_CONFIG line** (sections and options):

| Section | Options |
| ------- | ------- |
| `[sailhand]` | `fullscreen`, `keep_screen_on`, `hide_temperature_reports`, `verbose_log`, `rotation` |
| `[printer <name>]` | `moonraker` (required): the address; `hidden_macros` (optional): list of macros hidden from the console of this printer |
| `[preset <name>]` | `extruder` and `bed` (both required): the temperature presets |
| `[step_choices]` | `move_mm`, `z_offset_mm`, `temperature`, `extrude_mm`, `extrude_speed`, `manual_probe_mm`; each optional |
| `[backlight]` | `device`, `brightness`, `off_when_idle_minutes`, `off_when_printing_minutes`; each optional |

Most of this is changed on the screen; the **temperature presets** only here. The Thermal page shows the first four.
Restart Sailhand after editing (`sudo systemctl restart sailhand`).

`rotation` (`0`, `90`, `180` or `270`, default `0`) turns the picture clockwise by that angle and the touch with it. Only
the [framebuffer fallback](install.md#framebuffer-fallback) uses it; under `cage` rotate the display in the system.
Example for a display mounted upside down: `rotation: 180` in `[sailhand]`.

**Below the SAVE_CONFIG line** (every line starts with `#*#`, exactly as in Klipper) is what nobody edits: the printer
in use (`printer = <name>`, or `moonraker = <address>` for one that is not saved), the step sizes chosen last
(`[selected_steps]`), and per printer its `uuid` and `known_macros`. Sailhand writes this block anew on every save. A
line below the SAVE_CONFIG line that does not start with `#*#` is an error; move it above the line. Renaming a
`[printer …]` section by hand is harmless: `known_macros` fill again at the next connection, and the uuid when the
printer is chosen again on System → Printers.

Commenting out works as usual. A commented-out option means its default, and Sailhand does not write it back. A
commented-out `[printer …]` or `[preset …]` section is not used and stays as a comment; if all presets are commented
out, there are none. Comments and blank lines stay when Sailhand saves a change from the screen. Removing a printer on
the screen removes its header and values; its comment lines stay.

If the file cannot be read, Sailhand starts with the defaults and does not overwrite the file; the log names the line.
This happens with a value that cannot be read (for example `fullscreen: tru`), a duplicate section or option, an option
before the first section, and a `[printer]` or `[preset]` without a name or without its required values. Unknown
sections and options are ignored with a warning in the log.

A settings file in the old TOML format is renamed to `<name>.old` (for example `sailhand.conf.old`) at the next start
and replaced by the defaults; nothing is taken over from it.

The **step sizes** the screen offers (Motion, Z offset, Thermal, Filament, the paper test) can be changed in the section
`[step_choices]`. Every option is optional; without it the default applies:

```ini
[step_choices]
move_mm: 0.1, 1, 10, 50, 100
z_offset_mm: 0.005, 0.01, 0.025, 0.05, 0.1
temperature: 1, 5, 10, 25
extrude_mm: 5, 10, 25, 50, 100
extrude_speed: 1, 2, 5, 10
manual_probe_mm: 0.01, 0.05, 0.1, 1
```

The order does not matter: Sailhand sorts each list and removes duplicates. Values that are not above zero are left out,
and a list with nothing usable, or with more than six values (`manual_probe_mm`: four, the paper test offers each of them
as minus and plus), is replaced by the default; the log says which option. If the step chosen last is not in a list any
more, the nearest one is selected. Lengths in `extrude_mm` above Klipper's `max_extrude_only_distance` stay in the row but
are disabled; if none fits, the limit itself is offered as well. Restart Sailhand after editing.

The printers chosen on System → Printers are kept as `[printer <name>]` sections, each with the macros hidden from its
console (see [Several printers](#several-printers)). The name can be changed here.

## Log

With the installer or the `.deb` the log is `printer_data/logs/sailhand.log`, next to `klippy.log` and
`moonraker.log`, so Mainsail and Fluidd list it under Machine → logs. It rotates at midnight, keeps three old files
(`sailhand.log.YYYY-MM-DD`) and also goes to the journal (`journalctl -u sailhand` with `install.sh`,
`journalctl -u sailhand@<user>` with the `.deb`). An installation from before 0.3.7 writes no log file until
`install.sh` runs again (see [Updates and channels](install.md#updates-and-channels)). The Sailhand version is shown
on the Settings page and at the top of each log file.

The normal log holds the basics: a header (Sailhand version, command line, system, the versions of Moonraker, Klipper
and Happy Hare, the detected hardware including a "Changers" line), the connection, Klipper state changes with Klipper's
reason, print state changes (without file names), failed commands with Moonraker's error, and warnings. It leaves out
user names, Wi-Fi network names and file names.

The verbose log adds more detail. Set `verbose_log: True` in the `[sailhand]` section of `sailhand.conf` (Mainsail: Machine → config) and restart
Sailhand: `sudo systemctl restart sailhand`, with the `.deb` `sudo systemctl restart sailhand@<user>`. Mainsail
can restart it too, because the installer adds it to `moonraker.asvc`. It then also logs:

- every command from the screen: the intent, the Moonraker call with its parameters, the answer and how long it took,
  and refusals such as "print running"
- changes of the objects that matter: print state and file, homing, pause, Happy Hare `mmu`, AFC objects, tool changer
  and tools, filament sensors, calibration helpers and LED effects (no temperatures or positions)
- Klipper's console lines and selected Moonraker notifications (Spoolman, power devices, files, history)
- file names, network names and the Moonraker user name

It costs nothing while it is off. Turn it off again after the bug is captured, because it writes more to the SD card.
`RUST_LOG` takes precedence over the setting; `RUST_LOG=sailhand_core=trace` also logs every raw Moonraker
notification.

## Bug reports

A bug report that can be fixed without follow-up questions has three parts:

1. The diagnostic report. On the printer host run `~/sailhand/sailhand --report` (installed with `install.sh`) or
   `sailhand --report` (`.deb`). It finds the settings itself; `--settings` and `--moonraker` work as usual if
   needed. The file `sailhand-report.json` lands in `printer_data/logs`, so Mainsail and Fluidd offer it under
   Machine → logs for download. Without that folder it lands in the current folder; `--report <file or folder>` picks
   another place.
2. The verbose log: set `verbose_log: True` in `[sailhand]` (see [Log](#log)), restart Sailhand, reproduce the problem once and
   download `sailhand.log`. Add `klippy.log` and `moonraker.log` when Klipper or Moonraker show errors. Set
   `verbose_log` back to `False` afterwards.
3. The steps to reproduce, numbered, and what you expected.

The report contains the Sailhand version, system, setup and settings; Moonraker `server.info` and `printer.info`
(without host name); board and OS (without network, serial numbers and SD card); the installed versions from the update
manager (Klipper, Moonraker, Happy Hare, AFC, Mainsail and so on, without commit history); the Spoolman status; the last
100 console lines; `printer.objects.list`; the status of all Klipper objects including the Klipper configuration
(`configfile.settings`); Klipper's command list; and what Sailhand detects (summary, changer systems Happy Hare, AFC
and tool changer, machine tree). It does not contain Moonraker's configuration (API keys, passwords), network
addresses, the host name, serial numbers, user names or the stored access of the display. Have a look at the file
before you share it.

Open a bug on the [issue page](https://github.com/freakydude/sailhand/issues/new/choose) and drag the files into
the form.

## Several printers

One Sailhand can operate several printers, one at a time. System → Printers lists them, like the Wi-Fi networks:
first the printer in use, then saved printers that answer, printers found on the network that are not saved yet, and
last saved printers that do not answer. A tap switches to a printer and saves it; the bin removes a saved printer
together with its login and trusted certificate. **Add** takes an address by hand, for example
`http://voron.local` behind nginx, `https://…`, or a printer that does not announce itself. A running print goes on when
you switch; it is the printer's job, not the screen's.

Printers announce themselves when Moonraker's zeroconf component is switched on (it is not by default):

```ini
[zeroconf]
```

in `moonraker.conf`, then restart Moonraker. Sailhand searches and checks the saved printers only while the Printers
page is open. Sailhand connects to the printer chosen last; if none was chosen yet, to the address given with `--moonraker`;
otherwise to `http://127.0.0.1:7125`. The printer used at the first start is kept as a saved printer (named after its
host), so macros hidden from its console stay hidden. If the printer does not answer, the screen says so and offers the
list, it does not switch to another printer by itself. Per printer Sailhand keeps the macros hidden from the console;
presets, step sizes and the display settings apply to all printers.

## Moonraker login

Usually Moonraker trusts the machine Sailhand runs on (`trusted_clients` in `moonraker.conf`, on the printer host
`localhost`), and nothing needs to be set up. If it does not, for example with Sailhand on another machine or with
`force_logins: True`, the screen says that Moonraker wants a login and offers one:

- **Log in on the screen** with a Moonraker user. Sailhand keeps only Moonraker's refresh token, not the password,
  and logs in again with it at every start until it expires (`login_timeout` in `moonraker.conf`, 90 days by default)
  or the user logs out; then the screen asks again. Logging out (Settings tab, or the same user logging out in
  Mainsail or Fluidd) ends every login of that user, so a user of its own for the display avoids surprises.
- **API key**, entered by hand in the credentials file below. Moonraker hands it out to an authorized client, for
  example on the printer host, which it usually trusts: `curl http://localhost:7125/access/api_key`.

The credentials are kept per Moonraker address in `~/.config/sailhand/credentials.json` (Windows:
`%APPDATA%\sailhand\credentials.json`), readable by the owner only. They stay there also when the settings lie in
`printer_data/config`, because Moonraker serves that folder and it is often backed up to a git repository.

```json
{
  "moonraker": {
    "http://printer.local:7125": { "api_key": "<Moonraker's API key>" }
  }
}
```

## Encrypted connection (HTTPS)

With an `http://` address Sailhand talks plain HTTP and WebSocket to Moonraker, like most installations in a home
network; the password of a login then goes over the network unencrypted. With an `https://` address (e.g.
`https://printer.local:7130`) the connection uses TLS. Moonraker serves TLS itself on `ssl_port` (7130 by default)
once `ssl_certificate_path` and `ssl_key_path` are set in its `[server]` section, or a reverse proxy such as nginx does
it.

A certificate that the system trusts (signed by a known authority, e.g. Let's Encrypt) is accepted without a
question. Moonraker's certificate is usually self-signed, though; then the screen shows its SHA-256 fingerprint and
asks whether to trust it. Compare it with the fingerprint on the printer host before you do:

```sh
openssl x509 -noout -fingerprint -sha256 -in <ssl_certificate_path from moonraker.conf>
```

Sailhand remembers the trusted certificate per address in
the credentials file, in the section `certificates`, and shows it shortened in the settings. If the printer presents
a different certificate later, the screen says that it has changed and shows the new fingerprint beside the one
trusted before; replacing it needs a confirmation. To forget a trusted certificate, delete its line in the file.

```json
{
  "moonraker": {},
  "certificates": {
    "https://printer.local:7130": "3F:A1:9C:07:…:5E:22:B0:4D"
  }
}
```

`sailhand --report` uses the trusted certificate as well; an address whose certificate is not trusted yet has to be
trusted on the screen first.

## Displays

Sailhand works best on a display of 5" or more at 800x480: the layout is made for that size. It has been tested on
the BTT HDMI5 (5") and the Raspberry Pi Touch Display v1 (7", DSI), both 800x480, and on a TFT35 SPI display through
the [framebuffer fallback](install.md#framebuffer-fallback). In fullscreen, other resolutions show the same layout scaled up or down to fit, so
a 7" 1024x600 display fills its screen and a smaller one, say a 4.3" 480x272, shows everything, only smaller. A window on
a desktop is never smaller than 800x480; a larger one shows more room instead of scaling (where a compositor forces a
smaller window, the layout is fitted as in fullscreen). Apart from the TFT35, other sizes are not tested yet. Whether a small display is
comfortable to read and touch depends on the display and on you, so it is worth a try. Layouts of their own for the 7"
and 4.3" displays are planned.

## Network settings

Moonraker can only report the host's network (`machine.system_info`), it cannot change it. Sailhand therefore talks
to NetworkManager over D-Bus. Managing Wi-Fi (open, WPA/WPA2, WPA3, hidden, remove saved networks,
on/off) and Ethernet (status, connect/disconnect) is offered when

- Sailhand runs on Linux with NetworkManager (default on Raspberry Pi OS Bookworm and newer), and
- Moonraker runs on the same host (`localhost`/`127.0.0.1`), so the changed network is the printer's network.

The user needs the polkit rule `50-sailhand.rules` (allows the NetworkManager actions for the group `netdev`).
`install.sh` installs it to `/etc/polkit-1/rules.d` and adds the user to `netdev`; the Debian package ships it in
`/usr/share/polkit-1/rules.d`, add the user yourself: `sudo usermod -aG netdev $USER` (takes effect after re-login or a
service restart). Changing the network while printing asks for confirmation, because the connection may drop.

On Windows, on Linux without NetworkManager or with a remote Moonraker the section only shows the printer host's
interfaces and addresses read from Moonraker.

## Brightness and standby

On displays whose backlight Linux can control (`/sys/class/backlight`: DSI displays such as the Raspberry Pi Touch
Display, SPI displays with a backlight pin, laptops) the light panel of the status bar has a row **Display**: the
slider sets the brightness (10 to 100 %), **Off** switches the screen off at once. System → Settings → Display sets when
the screen goes off by itself, without a touch, once while idle and once while printing (pause included). Both are
**Never** by default.

A dark screen wakes up with the next touch; that touch does nothing else, nor do further touches in the next half
second. The printer wakes it too: a print starts or ends, pauses or fails, Klipper shuts down, the connection is lost or
Klipper answers with an error (`!!`).

**Monitors on HDMI or DisplayPort** are controlled through DDC/CI with the program `ddcutil` (`apt install ddcutil` or
`pacman -S ddcutil`), if it is installed and lists a monitor that answers on brightness. The user of Sailhand needs
access to `/dev/i2c-*` (group `i2c`, the installer adds it) and the kernel module `i2c-dev`, which the `ddcutil`
package normally loads. With several monitors the first that answers is used; `device: ddc:i2c-8` names one, the
log file lists them with their bus. Off switches the monitor through its power mode; a monitor without one is only
dimmed to 0. Some monitors no longer answer on DDC once switched off, so a touch cannot light them again: the log says
so, and the standby is then not for that monitor. DDC is slow (a tenth of a second per command), and the buttons of the
monitor are not followed.

Displays on HDMI that have no DDC/CI, such as the BTT HDMI5, keep their backlight on as long as they get a signal and
have no `/sys/class/backlight`; there Sailhand shows neither the slider nor the standby settings.

Changing the backlight needs the udev rule `70-sailhand-backlight.rules` (makes `brightness` and `bl_power` writable
for the group `video`) and the user in `video`. `install.sh` sets up both; the Debian package ships the rule in
`/lib/udev/rules.d`, add the user yourself: `sudo usermod -aG video $USER` (takes effect after a service restart). An
installation from before this version gets the rule by running `install.sh` again (see
[Updates and channels](install.md#updates-and-channels)).

In the settings file, every entry optional:

```ini
[backlight]
device: intel_backlight
brightness: 80
off_when_idle_minutes: 5
off_when_printing_minutes: 30
```

`device` picks a device by its name under `/sys/class/backlight` (for example `10-0045` or `intel_backlight`) or a
DDC/CI monitor as `ddc:i2c-N`; without it Sailhand takes the first by type (firmware, platform, raw) and, where
there is none, the first monitor on DDC/CI. `brightness` is the last value set on the screen in
percent, set again at the start; without it the brightness stays as the system set it. Without the minutes the screen
never goes off by itself.
The log names every backlight found and the one used, and says why the screen went off or on.
