#!/bin/bash
# Packs one downloaded build artifact (a folder named after its target) into a release zip; a Linux target also gets
# a .deb and an Arch package and release_info.json for the Moonraker update manager (type: zip).
# Run in the folder containing the artifact folder: pack-artifacts.sh <release-tag> <artifact-folder>
set -euo pipefail

version="$1"
name="${2%/}"
scripts="$(dirname "${BASH_SOURCE[0]}")"

if [[ "$name" == *linux* ]]; then
  cat > "$name/release_info.json" << EOF
{
  "project_name": "freakymate",
  "project_owner": "freakydude",
  "version": "$version",
  "asset_name": "$name.zip"
}
EOF
  # upload-artifact loses the execute bits; Moonraker restores them from the zip when extracting.
  chmod 0755 "$name/freakymate" "$name/install.sh" "$name/uninstall.sh"
  bash "$scripts/build-deb.sh" "$name" "$version"
  bash "$scripts/build-arch.sh" "$name" "$version"
fi
(cd "$name" && zip -qr "../$name.zip" .)
ls -l ./*.zip ./*.deb ./*.pkg.tar.zst 2> /dev/null || true
