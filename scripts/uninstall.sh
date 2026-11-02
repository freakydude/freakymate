#!/bin/bash
# Removes the service, the program folder and the Moonraker update manager entry.
# System packages, settings (printer_data/config/freakymate.conf) and a stored Moonraker login
# (~/.config/freakymate/credentials.json) are kept.
# Usage: ./uninstall.sh [-d <printer_data>]
set -euo pipefail

NAME="freakymate"
INSTALL_DIR="$HOME/$NAME"
DATA_DIR="$HOME/printer_data"

while getopts "d:" option; do
  case $option in
    d) DATA_DIR=$OPTARG ;;
    *) echo "usage: $0 [-d printer_data]"; exit 2 ;;
  esac
done

sudo systemctl disable --now "$NAME.service" || true
sudo rm -f "/etc/systemd/system/$NAME.service" "/etc/polkit-1/rules.d/50-$NAME.rules" \
  "/etc/udev/rules.d/70-$NAME-backlight.rules"
sudo rm -rf "/etc/$NAME"
sudo systemctl daemon-reload
sudo udevadm control --reload-rules || true
sudo systemctl enable getty@tty1.service || true

MOONRAKER_CONF="$DATA_DIR/config/moonraker.conf"
ALLOWED_SERVICES="$DATA_DIR/moonraker.asvc"
if [ -f "$MOONRAKER_CONF" ] && grep -q "^\[update_manager $NAME\]" "$MOONRAKER_CONF"; then
  # Remove the section up to the next section header.
  awk -v header="[update_manager $NAME]" '$0 == header {skip = 1; next} /^\[/ {skip = 0} !skip' \
    "$MOONRAKER_CONF" > "$MOONRAKER_CONF.tmp" && mv "$MOONRAKER_CONF.tmp" "$MOONRAKER_CONF"
  sudo systemctl restart moonraker || true
fi
if [ -f "$ALLOWED_SERVICES" ]; then
  sed -i "/^$NAME\$/d" "$ALLOWED_SERVICES"
fi

rm -rf "$INSTALL_DIR"
echo "Removed. Settings remain in $DATA_DIR/config/$NAME.json."
if [ -f "$HOME/.config/$NAME/credentials.json" ]; then
  echo "A Moonraker login remains in $HOME/.config/$NAME/credentials.json; delete it if it is no longer needed."
fi
