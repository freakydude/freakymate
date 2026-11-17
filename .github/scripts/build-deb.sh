#!/bin/bash
# Builds a Debian package sailhand_<version>_<arch>.deb from a Linux build artifact.
# Usage: build-deb.sh <artifact-folder> <tag>  (tag like v0.2.5 or v0.2.5-pr.147.3; in the artifacts folder, repo one level up)
set -euo pipefail

artifact="$1"
release="$2"
repo="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"

case "$artifact" in
  *x86_64*) arch=amd64 ;;
  *aarch64*) arch=arm64 ;;
  *armv7*) arch=armhf ;;
  *) echo "unknown architecture in '$artifact'"; exit 1 ;;
esac

# Pre-release suffixes sort before the release in Debian via ~.
version=$(echo "${release#v}" | tr '-' '~')

depends=$(python3 -c 'import json, sys; print(", ".join(["libc6 (>= 2.35)", "libfontconfig1"] + json.load(open(sys.argv[1]))["debian"]))' \
  "$repo/scripts/system-dependencies.json")

root=$(mktemp -d)
install -D -m 0755 "$artifact/sailhand" "$root/usr/bin/sailhand"
install -D -m 0644 "$repo/scripts/debian/sailhand@.service" "$root/lib/systemd/system/sailhand@.service"
install -D -m 0644 "$repo/scripts/polkit/50-sailhand.rules" "$root/usr/share/polkit-1/rules.d/50-sailhand.rules"
install -D -m 0644 "$repo/scripts/udev/70-sailhand-backlight.rules" "$root/lib/udev/rules.d/70-sailhand-backlight.rules"
install -D -m 0644 "$repo/LICENSE" "$root/usr/share/doc/sailhand/copyright"
install -D -m 0644 "$artifact/THIRD-PARTY-LICENSES.html" "$root/usr/share/doc/sailhand/THIRD-PARTY-LICENSES.html"
install -D -m 0644 "$repo/scripts/debian/sailhand.desktop" "$root/usr/share/applications/sailhand.desktop"
install -D -m 0644 "$repo/assets/logo/sailhand.svg" "$root/usr/share/icons/hicolor/scalable/apps/sailhand.svg"
for size in 16 24 32; do
  install -D -m 0644 "$repo/assets/logo/sailhand-small.svg" "$root/usr/share/icons/hicolor/${size}x${size}/apps/sailhand.svg"
done
install -d "$root/DEBIAN"
install -m 0755 "$repo/scripts/debian/postinst" "$repo/scripts/debian/prerm" "$repo/scripts/debian/postrm" "$root/DEBIAN/"

cat > "$root/DEBIAN/control" << EOF
Package: sailhand
Version: $version
Architecture: $arch
Maintainer: Frank Roth <developer@freakydu.de>
Depends: $depends
Suggests: network-manager
Section: misc
Priority: optional
Homepage: https://github.com/freakydude/sailhand
Description: Touch screen UI for Klipper printers via Moonraker (experimental)
 Native Rust + Slint UI for small displays on the printer host.
 UNTESTED EXPERIMENT: use at your own risk, no warranty, no liability.
 Start in kiosk mode on tty1 with: systemctl enable --now sailhand@<user>
EOF

dpkg-deb --root-owner-group --build "$root" "sailhand_${version}_${arch}.deb"
rm -rf "$root"
