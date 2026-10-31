#!/bin/bash
# Installs FreakyMate from an unpacked release zip as a kiosk service (cage on tty1) and
# registers it with the Moonraker update manager (type: zip), so Mainsail, Fluidd and FreakyMate
# itself can install later releases.
#
# Run inside the unpacked folder: ./install.sh [-m <moonraker-url>] [-d <printer_data>] [-c stable|beta]
# Young software: use it with care and at your own risk (see the README).

set -euo pipefail

NAME="freakymate"
SOURCE_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
INSTALL_DIR="$HOME/$NAME"
SERVICE_FILE="/etc/systemd/system/$NAME.service"
POLKIT_RULE="50-freakymate.rules"
MOONRAKER="http://127.0.0.1:7125"
DATA_DIR="$HOME/printer_data"
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

report() { echo -e "\n###### $1"; }

if [ "$EUID" -eq 0 ]; then
  echo "Run as normal user; sudo is used where needed."
  exit 1
fi
for file in "$NAME" release_info.json system-dependencies.json; do
  if [ ! -f "$SOURCE_DIR/$file" ]; then
    echo "'$file' missing; run this script inside the unpacked release zip for your architecture."
    exit 1
  fi
done

report "Checking the system"
glibc=$(getconf GNU_LIBC_VERSION 2>/dev/null | awk '{print $2}')
if [ -n "$glibc" ] && ! printf '%s\n%s\n' "$MIN_GLIBC" "$glibc" | sort -V -C; then
  echo "This system has glibc $glibc; FreakyMate needs $MIN_GLIBC or newer: $SUPPORTED_SYSTEMS."
  exit 1
fi
if ! "$SOURCE_DIR/$NAME" --version > /dev/null 2>&1; then
  echo "The program in this folder does not run on this system ($(uname -m)); download the zip for its architecture."
  exit 1
fi

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
  echo "On Ubuntu and Ubuntu-based Armbian enable 'universe' (sudo add-apt-repository universe); otherwise FreakyMate"
  echo "needs $SUPPORTED_SYSTEMS."
  exit 1
fi
sudo apt-get install -y "${packages[@]}"
sudo usermod -aG video,input,render "$USER" || true

report "Allowing network management (NetworkManager via polkit, group netdev)"
if [ -f "$SOURCE_DIR/$POLKIT_RULE" ] && [ -d /etc/polkit-1/rules.d ]; then
  sudo install -m 0644 "$SOURCE_DIR/$POLKIT_RULE" "/etc/polkit-1/rules.d/$POLKIT_RULE"
  sudo groupadd -f netdev
  sudo usermod -aG netdev "$USER" || true
else
  echo "polkit rules directory not found; Wi-Fi settings in FreakyMate will be read-only."
fi

report "Installing to $INSTALL_DIR"
if [ "$SOURCE_DIR" != "$INSTALL_DIR" ]; then
  mkdir -p "$INSTALL_DIR"
  cp -a "$SOURCE_DIR"/. "$INSTALL_DIR"/
fi
chmod 0755 "$INSTALL_DIR/$NAME" "$INSTALL_DIR/install.sh" "$INSTALL_DIR/uninstall.sh"

report "Creating $SERVICE_FILE"
# Settings live next to the Klipper configs, so they can be edited in Mainsail or Fluidd and backups of printer_data
# include them.
SETTINGS_FILE="$DATA_DIR/config/freakymate.json"
OLD_SETTINGS_FILE="$HOME/.config/$NAME/settings.json"
if [ -f "$OLD_SETTINGS_FILE" ] && [ ! -f "$SETTINGS_FILE" ]; then
  mkdir -p "$(dirname "$SETTINGS_FILE")"
  cp "$OLD_SETTINGS_FILE" "$SETTINGS_FILE"
  echo "Copied existing settings from $OLD_SETTINGS_FILE"
fi
USER_ID=$(id -u)
sudo tee "$SERVICE_FILE" > /dev/null << EOF
[Unit]
Description=FreakyMate touch UI for Klipper
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
# -d: no client-side frame, otherwise winit keeps room for its title bar after leaving fullscreen (black bar).
ExecStart=/usr/bin/cage -d -s -- $INSTALL_DIR/$NAME --fullscreen --moonraker $MOONRAKER --settings $SETTINGS_FILE
Restart=always
RestartSec=5

[Install]
# Klipper hosts run multi-user.target without a desktop; graphical.target is never reached there.
WantedBy=multi-user.target
EOF
sudo systemctl daemon-reload
sudo systemctl disable getty@tty1.service || true
sudo systemctl enable --now "$NAME.service"

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
repo: freakydude/freakymate
path: $INSTALL_DIR
managed_services: $NAME
system_dependencies: system-dependencies.json
EOF
  fi
  sudo systemctl restart moonraker || echo "Restart Moonraker yourself to activate the update entry."
fi

echo "Done. Logs: journalctl -u $NAME -f"
