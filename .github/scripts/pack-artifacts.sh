#!/bin/bash
# Packs the downloaded build artifacts (one folder per target) into release zips and builds a .deb per Linux target.
# Linux packages get release_info.json for the Moonraker update manager (type: zip); get-freakymate.sh goes next to them.
# Run in the folder containing the artifact subfolders: pack-artifacts.sh <release-tag>
set -euo pipefail

version="$1"

for directory in */; do
  name="${directory%/}"
  if [[ "$name" != *windows* ]]; then
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
    bash "$(dirname "${BASH_SOURCE[0]}")/build-deb.sh" "$name" "$version"
  fi
  (cd "$name" && zip -qr "../$name.zip" .)
done
install -m 0755 "$(dirname "${BASH_SOURCE[0]}")/../../scripts/get-freakymate.sh" .
ls -l ./*.zip ./*.deb get-freakymate.sh
