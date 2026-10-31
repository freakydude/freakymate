#!/bin/bash
# Downloads the FreakyMate release zip for this system's architecture and runs its install.sh.
#
#   curl -fsSLO https://github.com/freakydude/freakymate/releases/latest/download/get-freakymate.sh
#   bash get-freakymate.sh [-c stable|beta] [-m <moonraker-url>] [-d <printer_data>]
#
# All options go to install.sh; -c beta also takes the newest pre-release. The zip is unpacked to ~/freakymate-setup.
# Young software: use it with care and at your own risk (see the README).

set -euo pipefail

REPO="freakydude/freakymate"
SETUP_DIR="$HOME/freakymate-setup"

channel="stable"
arguments=("$@")
while getopts "m:d:c:" option; do
  case $option in
    c) channel=$OPTARG ;;
    m | d) ;;
    *) echo "usage: $0 [-c stable|beta] [-m moonraker-url] [-d printer_data]"; exit 2 ;;
  esac
done

for tool in curl unzip python3; do
  if ! command -v "$tool" > /dev/null; then
    echo "'$tool' is missing; install it first: sudo apt-get install $tool"
    exit 1
  fi
done

# The architecture of the installed packages: a 32-bit Raspberry Pi OS often runs on a 64-bit kernel.
architecture=$(dpkg --print-architecture 2>/dev/null || uname -m)
case $architecture in
  amd64 | x86_64) target="x86_64-unknown-linux-gnu" ;;
  arm64 | aarch64) target="aarch64-unknown-linux-gnu" ;;
  armhf | armv7l) target="armv7-unknown-linux-gnueabihf" ;;
  *) echo "No FreakyMate build for the architecture '$architecture'."; exit 1 ;;
esac

case $channel in
  stable) release_url="https://api.github.com/repos/$REPO/releases/latest" ;;
  beta) release_url="https://api.github.com/repos/$REPO/releases?per_page=1" ;;
  *) echo "channel must be stable or beta"; exit 2 ;;
esac

asset="freakymate-$target.zip"
download_url=$(curl -fsSL "$release_url" | python3 -c '
import json, sys
release = json.load(sys.stdin)
release = release[0] if isinstance(release, list) else release
urls = [a["browser_download_url"] for a in release.get("assets", []) if a["name"] == sys.argv[1]]
print(urls[0] if urls else "")' "$asset")
if [ -z "$download_url" ]; then
  echo "The newest $channel release has no $asset."
  exit 1
fi

echo "Downloading $download_url"
rm -rf "$SETUP_DIR"
mkdir -p "$SETUP_DIR"
curl -fSL "$download_url" -o "$SETUP_DIR/$asset"
(cd "$SETUP_DIR" && unzip -q "$asset" && rm "$asset")
"$SETUP_DIR/install.sh" "${arguments[@]}"
