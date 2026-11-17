#!/bin/bash
# Builds Sailhand for Android, installs it on the emulator (or a connected device) and shows its log until stopped.
#   scripts/android-run.sh            keeps the app's saved settings
#   scripts/android-run.sh --fresh    clears them first: the app starts with the default address 127.0.0.1:7125,
#                                     which `adb reverse` leads to the fake Moonraker on this machine
# Needs ANDROID_HOME (SDK with the NDK of android/app/build.gradle.kts), cargo-ndk and the Rust target of the device
# (x86_64-linux-android for the emulator). Without a device it starts the AVD $SAILHAND_AVD (default sailhand-test).
set -euo pipefail

# A local build is always the app Sailhand Dev (flavor dev), beside an installed Sailhand.
readonly package=de.freakydu.sailhand.dev
readonly moonraker_port=7125
fresh=false
case "${1:-}" in
  --fresh) fresh=true ;;
  "") ;;
  *) echo "usage: $0 [--fresh]" >&2; exit 2 ;;
esac

repo=$(cd "$(dirname "$0")/.." && pwd)
sdk=${ANDROID_HOME:-${ANDROID_SDK_ROOT:-}}
# A desktop session keeps the environment of its login, also an SDK path that has moved since; then the adb on the
# PATH tells where the SDK is (<sdk>/platform-tools/adb).
if [ ! -x "$sdk/platform-tools/adb" ] && path_adb=$(command -v adb); then
  echo "ANDROID_HOME=${sdk:-(unset)} holds no SDK; using the one of $path_adb" >&2
  sdk=$(dirname "$(dirname "$(readlink -f "$path_adb")")")
fi
adb=$sdk/platform-tools/adb
[ -x "$adb" ] || { echo "no Android SDK: set ANDROID_HOME (no adb in ${sdk:-ANDROID_HOME}/platform-tools)" >&2; exit 1; }
export ANDROID_HOME=$sdk ANDROID_SDK_ROOT=$sdk
ndk_version=$(sed -n 's/^ *ndkVersion = "\(.*\)"/\1/p' "$repo/android/app/build.gradle.kts")
export ANDROID_NDK_HOME=$sdk/ndk/$ndk_version

start_emulator() {
  local avd=${SAILHAND_AVD:-sailhand-test}
  local log=${TMPDIR:-/tmp}/sailhand-emulator.log
  echo "starting the emulator $avd (log: $log)"
  nohup "$sdk/emulator/emulator" -avd "$avd" >"$log" 2>&1 &
  "$adb" wait-for-device
  until [ "$("$adb" shell getprop sys.boot_completed 2>/dev/null | tr -d '\r')" = 1 ]; do sleep 1; done
}

if ! "$adb" get-state >/dev/null 2>&1; then
  start_emulator
fi

abi=$("$adb" shell getprop ro.product.cpu.abi | tr -d '\r')
version=$(bash "$repo/.github/scripts/version.sh")
# The program shows it (crates/core VERSION), as in CI.
export SAILHAND_VERSION=$version
echo "building $version for $abi"
(cd "$repo/android" && ./gradlew -q assembleDevRelease "-PappVersion=$version" "-Pabis=$abi")
apk=$repo/android/app/build/outputs/apk/dev/release/sailhand-$version-$abi-dev-release.apk

if ! "$adb" install -r "$apk"; then
  echo "install failed; Sailhand Dev from another machine or from CI (other debug key) must be uninstalled first:" >&2
  echo "  $adb uninstall $package" >&2
  exit 1
fi
if $fresh; then
  "$adb" shell pm clear "$package" >/dev/null
fi
"$adb" reverse "tcp:$moonraker_port" "tcp:$moonraker_port" >/dev/null
"$adb" shell am start -W -n "$package/android.app.NativeActivity" >/dev/null

pid=$("$adb" shell pidof "$package" | tr -d '\r')
[ -n "$pid" ] || { echo "the app did not start; see: $adb logcat" >&2; exit 1; }
trap '"$adb" shell am force-stop "$package"' EXIT
echo "Sailhand runs (pid $pid); stop with Ctrl+C or the stop button of the debugger"
"$adb" logcat -v time --pid="$pid"
