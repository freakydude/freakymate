#!/bin/bash
# Installs Sailhand from an unpacked release zip as a kiosk service (cage on tty1; on systems without a DRM/KMS
# device the framebuffer fallback without cage) and
# registers it with the Moonraker update manager (type: zip), so Mainsail, Fluidd and Sailhand
# itself can install later releases.
#
# Run inside the unpacked folder: ./install.sh [-m <moonraker-url>] [-d <printer_data>] [-c stable|beta]
# Running it again (also as ~/sailhand/install.sh) brings the setup outside the program folder up to date, which
# Moonraker's updates do not touch; without -m and -d it keeps the values of the existing installation.
# Young software: use it with care and at your own risk (see the README).

set -euo pipefail

NAME="sailhand"
SOURCE_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
INSTALL_DIR="$HOME/$NAME"
SERVICE_FILE="/etc/systemd/system/$NAME.service"
POLKIT_RULE="50-sailhand.rules"
UDEV_RULE="70-sailhand-backlight.rules"
SETUP_MARKER="/etc/$NAME/setup-version"
MOONRAKER=""
DATA_DIR=""
CHANNEL="stable"
# glibc of the build container (.github/workflows/build.yml); systems that also have cage and seatd.
MIN_GLIBC="2.35"
SUPPORTED_SYSTEMS="Debian 12 or 13, Raspberry Pi OS Bookworm or newer, Ubuntu or Armbian 22.04 or newer"

while getopts "m:d:c:" option; do
  case $option in
    m) MOONRAKER=$OPTARG ;;
    d) DATA_DIR=$OPTARG ;;
    c) CHANNEL=$OPTARG ;;
    *) echo "usage: $0 [-m moonraker-url] [-d printer_data] [-c stable|beta]"; exit 2 ;;
  esac
done
case $CHANNEL in
  stable | beta) ;;
  *) echo "channel must be stable or beta"; exit 2 ;;
esac

# The value of an option in the existing service, e.g. `previous_option moonraker`; empty without one.
previous_option() {
  if [ -f "$SERVICE_FILE" ]; then
    grep -o -- "--$1 [^ ]*" "$SERVICE_FILE" | head -n 1 | cut -d ' ' -f 2 || true
  fi
}
if [ -z "$MOONRAKER" ]; then
  MOONRAKER=$(previous_option moonraker)
fi
if [ -z "$DATA_DIR" ]; then
  previous_settings=$(previous_option settings)
  if [ -n "$previous_settings" ]; then
    DATA_DIR=$(dirname "$(dirname "$previous_settings")")
  fi
fi
MOONRAKER=${MOONRAKER:-http://127.0.0.1:7125}
DATA_DIR=${DATA_DIR:-$HOME/printer_data}

report() { echo -e "\n###### $1"; }

if [ "$EUID" -eq 0 ]; then
  echo "Run as normal user; sudo is used where needed."
  exit 1
fi
# Sailhand was called FreakyMate and does not take over from it (both want tty1, both have an update entry), so stop
# before changing anything. Pressing Update on FreakyMate after the rename empties ~/freakymate, uninstall.sh included
# (Moonraker removes the folder and then fails on the wrong asset), so the manual steps in the docs are the fallback.
OLD_NAME="freakymate"
MIGRATION_DOCS="https://github.com/freakydude/sailhand/blob/main/docs/install.md#coming-from-freakymate"
# Prints what is left of FreakyMate, one item per line; nothing when it is gone. An empty ~/freakymate is harmless.
freakymate_leftovers() {
  local item
  if freakymate_package > /dev/null; then
    echo "package $OLD_NAME"
  fi
  for item in "/etc/systemd/system/$OLD_NAME.service" /etc/systemd/system/multi-user.target.wants/"$OLD_NAME"@*.service \
    "/etc/polkit-1/rules.d/50-$OLD_NAME.rules" "/etc/udev/rules.d/70-$OLD_NAME-backlight.rules" "/etc/$OLD_NAME"; do
    [ -e "$item" ] && echo "$item"
  done
  if [ -n "$(find "$HOME/$OLD_NAME" -mindepth 1 -print -quit 2> /dev/null)" ]; then
    echo "$HOME/$OLD_NAME"
  fi
  if grep -qs "^\[update_manager $OLD_NAME\]" "$DATA_DIR/config/moonraker.conf"; then
    echo "[update_manager $OLD_NAME] in $DATA_DIR/config/moonraker.conf"
  fi
  if grep -qsx "$OLD_NAME" "$DATA_DIR/moonraker.asvc"; then
    echo "'$OLD_NAME' in $DATA_DIR/moonraker.asvc"
  fi
  return 0
}
# Prints the command that removes the FreakyMate package; fails when no package is installed.
freakymate_package() {
  if dpkg-query -W -f '${Status}' "$OLD_NAME" 2> /dev/null | grep -q "ok installed"; then
    echo "sudo apt remove $OLD_NAME"
  elif pacman -Q "$OLD_NAME" > /dev/null 2>&1; then
    echo "sudo pacman -R $OLD_NAME"
  else
    return 1
  fi
}
# Prints how to remove FreakyMate here: its package, its own uninstaller, or the manual steps.
freakymate_removal() {
  local package
  if package=$(freakymate_package); then
    echo "  $package"
  elif [ -x "$HOME/$OLD_NAME/uninstall.sh" ]; then
    echo "  ~/$OLD_NAME/uninstall.sh"
  else
    echo "  by hand (its uninstall.sh is gone), with the steps from the page below"
  fi
}
leftovers=$(freakymate_leftovers)
if [ -n "$leftovers" ]; then
  echo "FreakyMate (the former name of Sailhand) is still installed here:"
  sed 's/^/  /' <<< "$leftovers"
  echo "Remove it first, then run this script again:"
  freakymate_removal
  echo "Its settings stay in $DATA_DIR/config/. Steps and how to keep the settings: $MIGRATION_DOCS"
  exit 1
fi
for file in "$NAME" release_info.json system-dependencies.json; do
  if [ ! -f "$SOURCE_DIR/$file" ]; then
    echo "'$file' missing; run this script inside the unpacked release zip for your architecture."
    exit 1
  fi
done

report "Checking the system"
echo "Moonraker: $MOONRAKER, printer_data: $DATA_DIR"
glibc=$(getconf GNU_LIBC_VERSION 2>/dev/null | awk '{print $2}')
if [ -n "$glibc" ] && ! printf '%s\n%s\n' "$MIN_GLIBC" "$glibc" | sort -V -C; then
  echo "This system has glibc $glibc; Sailhand needs $MIN_GLIBC or newer: $SUPPORTED_SYSTEMS."
  exit 1
fi
if ! "$SOURCE_DIR/$NAME" --version > /dev/null 2>&1; then
  echo "The program in this folder does not run on this system ($(uname -m)); download the zip for its architecture."
  exit 1
fi
# A package (.deb, Arch) brings its own kiosk service sailhand@<user>; it and this one both want tty1.
for packaged in /etc/systemd/system/multi-user.target.wants/sailhand@*.service; do
  [ -e "$packaged" ] || continue
  echo "Note: the kiosk of the Sailhand package is enabled too ($(basename "$packaged")); both want tty1."
  echo "Keep one of them, e.g.: sudo systemctl disable --now $(basename "$packaged")"
done

report "Installing system packages"
mapfile -t packages < <(python3 -c 'import json, sys; print("\n".join(json.load(open(sys.argv[1]))["debian"]))' \
  "$SOURCE_DIR/system-dependencies.json")
sudo apt-get update
missing=()
for package in "${packages[@]}"; do
  apt-cache show "$package" > /dev/null 2>&1 || missing+=("$package")
done
if [ ${#missing[@]} -gt 0 ]; then
  echo "Not available from the package sources: ${missing[*]}."
  echo "On Ubuntu and Ubuntu-based Armbian enable 'universe' (sudo add-apt-repository universe); otherwise Sailhand"
  echo "needs $SUPPORTED_SYSTEMS."
  exit 1
fi
sudo apt-get install -y "${packages[@]}"
sudo usermod -aG video,input,render "$USER" || true
# Monitors on DDC/CI (brightness through ddcutil, if installed) need access to /dev/i2c-*.
if getent group i2c > /dev/null; then
  sudo usermod -aG i2c "$USER" || true
fi

report "Allowing network management (NetworkManager via polkit, group netdev)"
if [ -f "$SOURCE_DIR/$POLKIT_RULE" ] && [ -d /etc/polkit-1/rules.d ]; then
  sudo install -m 0644 "$SOURCE_DIR/$POLKIT_RULE" "/etc/polkit-1/rules.d/$POLKIT_RULE"
  sudo groupadd -f netdev
  sudo usermod -aG netdev "$USER" || true
else
  echo "polkit rules directory not found; Wi-Fi settings in Sailhand will be read-only."
fi

report "Allowing brightness and screen standby (backlight via udev, group video)"
if [ -f "$SOURCE_DIR/$UDEV_RULE" ] && [ -d /etc/udev/rules.d ]; then
  sudo install -m 0644 "$SOURCE_DIR/$UDEV_RULE" "/etc/udev/rules.d/$UDEV_RULE"
  sudo udevadm control --reload-rules || true
  # Applies the rule to the backlight that is already there, without a reboot.
  sudo udevadm trigger --subsystem-match=backlight --action=add || true
else
  echo "udev rules directory not found; Sailhand will not offer brightness and standby."
fi

report "Installing to $INSTALL_DIR"
if [ "$SOURCE_DIR" != "$INSTALL_DIR" ]; then
  mkdir -p "$INSTALL_DIR"
  # A running Sailhand cannot be overwritten ("Text file busy"); replacing the file leaves the old one to the running
  # process until the restart below.
  cp -a --remove-destination "$SOURCE_DIR"/. "$INSTALL_DIR"/
fi
chmod 0755 "$INSTALL_DIR/$NAME" "$INSTALL_DIR/install.sh" "$INSTALL_DIR/uninstall.sh"

report "Creating $SERVICE_FILE"
# Settings live next to the Klipper configs, so they can be edited in Mainsail or Fluidd and backups of printer_data
# include them. The program creates the file with the defaults at its first start; an old sailhand.json stays as it
# is and is no longer read.
SETTINGS_FILE="$DATA_DIR/config/sailhand.conf"
USER_ID=$(id -u)
ARGUMENTS="--moonraker $MOONRAKER --settings $SETTINGS_FILE --logfile $DATA_DIR/logs/sailhand.log"
# -d: no client-side frame, otherwise winit keeps room for its title bar after leaving fullscreen (black bar).
START="/usr/bin/cage -d -s -- $INSTALL_DIR/$NAME --fullscreen $ARGUMENTS"
CONSOLE_LINES=""
# cage needs a DRM/KMS device. Without one (only a framebuffer driver such as fbtft) the fallback draws straight to
# the framebuffer. The text console would draw into it as well, so it is detached from the framebuffer while the
# service runs (as root: `+`); systemd turns `$$` into `$`.
if ! compgen -G "/dev/dri/card*" > /dev/null && compgen -G "/dev/fb[0-9]*" > /dev/null; then
  echo "No DRM/KMS device, only a framebuffer: using the framebuffer fallback without cage."
  START="$INSTALL_DIR/$NAME --framebuffer $ARGUMENTS"
  console_loop='for c in /sys/class/vtconsole/vtcon*; do grep -q "frame buffer" "$$c/name" && echo BIND > "$$c/bind"; done; true'
  CONSOLE_LINES="ExecStartPre=+/bin/sh -c '${console_loop/BIND/0}'
ExecStopPost=+/bin/sh -c '${console_loop/BIND/1}'"
fi
sudo tee "$SERVICE_FILE" > /dev/null << EOF
[Unit]
Description=Sailhand touch UI for Klipper
After=systemd-user-sessions.service network-online.target moonraker.service
Wants=network-online.target
Conflicts=getty@tty1.service

[Service]
Type=simple
User=$USER
PAMName=login
TTYPath=/dev/tty1
StandardInput=tty
Environment=XDG_RUNTIME_DIR=/run/user/$USER_ID
$CONSOLE_LINES
ExecStart=$START
Restart=always
RestartSec=5

[Install]
# Klipper hosts run multi-user.target without a desktop; graphical.target is never reached there.
WantedBy=multi-user.target
EOF
# The program compares it with the setup it needs and asks for this script again when it is older.
sudo mkdir -p "$(dirname "$SETUP_MARKER")"
"$INSTALL_DIR/$NAME" --setup-version | sudo tee "$SETUP_MARKER" > /dev/null
sudo systemctl daemon-reload
sudo systemctl disable getty@tty1.service || true
sudo systemctl enable "$NAME.service"
# Restart, not only start: when run again, the running instance still has the old arguments and the old marker.
sudo systemctl restart "$NAME.service"

report "Registering with the Moonraker update manager"
MOONRAKER_CONF="$DATA_DIR/config/moonraker.conf"
ALLOWED_SERVICES="$DATA_DIR/moonraker.asvc"
if [ ! -f "$MOONRAKER_CONF" ]; then
  echo "$MOONRAKER_CONF not found; add the [update_manager $NAME] section manually (see docs/install.md)."
else
  if [ -f "$ALLOWED_SERVICES" ]; then
    if ! grep -qx "$NAME" "$ALLOWED_SERVICES"; then
      # Without a trailing newline the name would be glued onto the last service.
      if [ -s "$ALLOWED_SERVICES" ] && [ -n "$(tail -c 1 "$ALLOWED_SERVICES")" ]; then
        echo >> "$ALLOWED_SERVICES"
      fi
      echo "$NAME" >> "$ALLOWED_SERVICES"
    fi
  else
    echo "WARNING: $ALLOWED_SERVICES not found. Moonraker may refuse to restart $NAME after an update;"
    echo "         add the line '$NAME' to that file yourself and restart Moonraker."
  fi
  if ! grep -q "^\[update_manager $NAME\]" "$MOONRAKER_CONF"; then
    cat >> "$MOONRAKER_CONF" << EOF

[update_manager $NAME]
type: zip
channel: $CHANNEL
repo: freakydude/sailhand
path: $INSTALL_DIR
managed_services: $NAME
system_dependencies: system-dependencies.json
EOF
  fi
  sudo systemctl restart moonraker || echo "Restart Moonraker yourself to activate the update entry."
fi

echo "Done. Log: $DATA_DIR/logs/sailhand.log (Mainsail: Machine, logs) or journalctl -u $NAME -f"
