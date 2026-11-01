# Features

What each page of FreakyMate does. Functions only appear when the printer has them (capabilities from
`printer.objects.list` and `server.info`).

- **Print**: thumbnail, progress, elapsed/remaining/ETA, layers with current and object height, filament; start, pause, resume, cancel, pause at a layer (planned pause shown), exclude objects (on a map of the bed if the slicer defines their outlines);
  file browser with thumbnails, filament, nozzle and print time of each file.
- **Move**: jog X/Y/Z with selectable steps, home (all/XY/Z), motors off, Quad Gantry Level or Z-Tilt when configured.
  Move and Filament are locked while a print runs; the screen leaves them when a print starts.
  - **Calibrate** tab: Klipper's own calibration commands, with their results. Bed screws
    (`SCREWS_TILT_CALCULATE` with the turns per screw, or the paper test `BED_SCREWS_ADJUST`), Z offset of the probe or
    the Z endstop, bed mesh (measure, a coloured map of the active mesh, load, save and remove profiles, clear) and
    what `SAVE_CONFIG` would write. While Klipper waits for the paper test or the Z height, a dialog shows its steps;
    closing it aborts. FreakyMate only starts these commands, Klipper does the work.
- **Thermal**: every heater with a target (nozzle, bed, `heater_generic`, `temperature_fan`), presets; − and +
  respect `min_temp`/`max_temp` from the Klipper configuration. Tapping a heater or sensor shows its temperature
  history with a grid: the last 20 minutes of Moonraker's temperature store, asked again every five seconds while it
  is open. Moonraker keeps heaters and `temperature_sensor` objects there; the others have no history. Further tabs when the printer has them:
  - **Fans**: part fan and `fan_generic` fans (filter, exhaust) with a slider, `heater_fan` and `controller_fan` as a
    bar, speed in rpm where there is a tachometer. A fan turns red when its tachometer has reported 0 rpm for a few
    seconds at a speed where it measured before (many tachometers report nothing at a low speed although the fan
    turns); what it learned is forgotten when Klipper restarts.
  - **Sensors**: every temperature without a target (`temperature_sensor` of any sensor type, TMC2240 drivers).
- **Light**: the light symbol in the status bar opens a panel with the main (chamber) light as a switch, every LED
  (`neopixel`, `led`, `dotstar`, …) on, off and dimmed as white light, and the effects of the
  [led_effect](https://github.com/julianschill/klipper-led_effect) plugin when it is installed.
- **Tune**: what one corrects while watching a print: Z offset (babysteps, apply to probe/endstop, save config) and
  the speed factor, with a quiet note that layer times then differ from the slicer's plan. The **Expert** tab holds
  the calibration values until the next restart: flow factor, pressure advance and smooth time, velocity and
  acceleration limits, square corner velocity, minimum cruise ratio and firmware retraction; it shows where a value
  differs from the configuration and resets it.
- **Filament**: extrude/retract with length and speed (lengths up to `max_extrude_only_distance`), `LOAD_FILAMENT`/`UNLOAD_FILAMENT` macros when present, filament sensors (with Happy Hare those it does not show itself).
- **System**, in five tabs:
  - **Machine**: Klipper/firmware restart, host reboot/shutdown, service restart, power devices.
  - **Updates**: Moonraker's update manager; the tab shows how many updates are waiting.
  - **Network**: Wi-Fi and Ethernet (see [Network settings](configuration.md#network-settings)).
  - **Console** (also from the message flyout): G-code history and live responses, a command field (on-screen keyboard
    or physical keyboard), and an expandable list of macros. Like Mainsail it hides the temperature reports Klipper
    sends while waiting for a heater (can be turned off). Macros starting with `_` and macros that rename a built-in
    command (`rename_existing`) are hidden; further macros can be hidden on the Display tab. Macros with parameters
    open a form first: each parameter is read from the macro's template, Klipper defaults show as placeholders in
    brackets.
  - **Display**: Moonraker address and login (see [Moonraker login](configuration.md#moonraker-login)), fullscreen,
    the console filter and which macros the console offers. Temperature presets are set in the
    [settings file](configuration.md#settings-file).

## Happy Hare (MMU)

Needs Happy Hare 4.1 or newer. The Filament page then opens on the **Gates** tab, next to **Extruder**, **Tools** and
**Service**. The Gates tab and a strip on the print page show:

- the gates and the bypass (five at a time): status, material, colour and the tools that take their filament from each
  gate; unload what is loaded, home, recover and unlock after a pause;
- the way of the filament of the selected gate, from the gate through the bowden and the extruder to the nozzle,
  filled in the colours of the spool as far as Happy Hare reports it (`filament_pos`), with the sensors on the way,
  the direction and the distance; on the print page as a slim line;
- the other MMU sensors, the sync-feedback buffer (tension, neutral, compression) and the FlowGuard state: ready, monitoring,
  tangle prevention at work, or triggered with its reason;
- the flow rate, measured by the encoder or estimated by the sync-feedback controller with a proportional sensor.

A tap on a gate opens what can be done with it: select, load, preload, check or eject it, pick its spool, move tools to
it (Happy Hare's tool-to-gate map) and, with EndlessSpool on, choose its group. Without Spoolman the dialog sets
material and colour of the gate in Happy Hare's gate map instead, from a set of common materials and colours. A tap on
a gate never moves the selector by itself. The bypass takes filament fed by hand past the gates; its dialog selects
and loads it. The **Tools** tab lists the tools with their gates, resets the map and turns EndlessSpool on or off.
The **Service** tab holds what is rarely needed for the MMU as a whole: home, recover, unlock, motors off, check all
gates and, on selectors with a servo, servo up and down.

| ![Gates tab](screenshots/mmu.png)     | ![FlowGuard line while tangle prevention raises the gear current](screenshots/print-tangle-boost.png) |
| ------------------------------------- | ----------------------------------------------------------------------------------------------------- |
| The Gates tab: gates, sensors, buffer | The print page while tangle prevention raises the gear current (the FlowGuard line turns yellow)      |

**Quieter console (recommended).** FreakyMate shows Klipper's console messages in its status bar. Happy Hare prints a
lot there by default: the gate table at startup, a picture of the filament path and statistics after every tool
change. FreakyMate draws the gates and the filament path itself from `printer.mmu`, so these lines only crowd the
status bar.
Happy Hare can keep the console to the essentials and still write everything to `mmu.log`; in `mmu_parameters.cfg`:

```ini
log_level: 0            # console: essential messages and errors only
log_file_level: 2       # mmu.log keeps the details (debug)
log_visual: 0           # no filament path picture in the console
log_startup_status: 0   # no gate table at startup
log_statistics: 0       # no statistics after each tool change (still recorded)
```

This changes the console for every client, Mainsail and Fluidd included.

## Spoolman

Appears when Moonraker has the `spoolman` component, with or without an MMU:

- the spool in use and what is left of it on the print page and at each gate, with a colour bar for multi-colour
  filaments: side by side for coaxial ones (silk), taking turns for longitudinal ones (gradient);
- a picker lists the spools (the ones in a gate first, then the most recently used): assign a spool to a gate or, without
  an MMU, choose the active spool, and correct the remaining weight by weighing;
- the temperature page offers a preset for the spool in use and notes quietly when the target is off from it;
- before a print the confirmation names what differs between the file and the spools (material, weight, colour), only as
  a hint.

| ![Spool picker](screenshots/spool-picker.png) | ![Start print with hints](screenshots/spool-hints.png) | ![Spool preset on the temperature page](screenshots/temperature-spool.png) |
| --------------------------------------------- | ------------------------------------------------------ | -------------------------------------------------------------------------- |
| Spools with colour, material and weight left  | Before a print: what differs between file and spools   | The first preset on the temperature page belongs to the spool in use       |
