#!/bin/bash
# Builds an Arch Linux package freakymate-<version>-1-<arch>.pkg.tar.zst from a Linux build artifact with makepkg.
# Usage: build-arch.sh <artifact-folder> <tag>  (tag like v0.2.5 or v0.2.5-pr.147.3; in the artifacts folder, repo one level up)
# The package lands in the current folder. Runs makepkg in an archlinux container (makepkg refuses root);
# FM_NO_DOCKER=1 runs it directly on an Arch system as a normal user.
set -euo pipefail

repo="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"

# Inside the container (as root): create a build user and run this script again as that user.
if [ "${1:-}" = "--container" ]; then
  pacman -Syu --noconfirm --needed fakeroot binutils
  useradd -m builder
  mkdir -p /home/builder/out
  chown builder /home/builder/out
  runuser -u builder -- env FM_NO_DOCKER=1 FM_ARCH="$FM_ARCH" FM_DEPENDS="$FM_DEPENDS" FM_OUT=/home/builder/out bash "$0" "$2" "$3"
  cp /home/builder/out/*.pkg.tar.zst /out/
  exit 0
fi

artifact="$(cd "$1" && pwd)"
release="$2"

# FM_ARCH is set in the container, where the artifact is mounted under a neutral name.
case "${FM_ARCH:-$artifact}" in
  *x86_64*) arch=x86_64 ;;
  *aarch64*) arch=aarch64 ;;
  *armv7*) arch=armv7h ;;
  *) echo "no Arch package for '$artifact'"; exit 1 ;;
esac

# pacman forbids '-' in the version; the pre-release suffix keeps its meaning after '_'.
pkgver=$(echo "${release#v}" | tr '-' '_')

# Read here, not in the container: the archlinux image has no Python.
depends="${FM_DEPENDS:-$(python3 -c 'import json, sys; print(" ".join("\x27" + name + "\x27" for name in json.load(open(sys.argv[1]))["arch"]))' \
  "$repo/scripts/system-dependencies.json")}"

if [ -z "${FM_NO_DOCKER:-}" ]; then
  docker run --rm \
    -v "$repo:/repo:ro" -v "$artifact:/artifact:ro" -v "$PWD:/out" -e FM_ARCH="$arch" -e FM_DEPENDS="$depends" \
    archlinux:latest bash /repo/.github/scripts/build-arch.sh --container /artifact "$release"
  ls "freakymate-${pkgver}-1-${arch}.pkg.tar.zst"
  exit 0
fi

work=$(mktemp -d)
sed -e "s|@PKGVER@|$pkgver|" -e "s|@ARCH@|$arch|" -e "s|@DEPENDS@|$depends|" \
  "$repo/scripts/arch/PKGBUILD.in" > "$work/PKGBUILD"
cp "$repo/scripts/arch/freakymate.install" "$work/"

# The runner is x86_64: makepkg only builds for its CARCH, so use a config copy with the target architecture.
conf="$work/makepkg.conf"
sed "s|^CARCH=.*|CARCH=\"$arch\"|" /etc/makepkg.conf > "$conf"

out="${FM_OUT:-$PWD}"
(cd "$work" && FM_ARTIFACT="$artifact" FM_REPO="$repo" PKGDEST="$out" PACKAGER="Frank Roth <developer@freakydu.de>" makepkg --config "$conf" --nodeps --force --clean)
rm -rf "$work"
