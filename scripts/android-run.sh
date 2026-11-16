#!/bin/bash
# Builds FreakyMate for Android, installs it on the emulator (or a connected device) and shows its log until stopped.
#   scripts/android-run.sh            keeps the app's saved settings
#   scripts/android-run.sh --fresh    clears them first: the app starts with the default address 127.0.0.1:7125,
#                                     which `adb reverse` leads to the fake Moonraker on this machine
# Needs ANDROID_HOME (SDK with the NDK of android/app/build.gradle.kts), cargo-ndk and the Rust target of the device
# (x86_64-linux-android for the emulator). Without a device it starts the AVD $FREAKYMATE_AVD (default freakymate-test).
set -euo pipefail

readonly package=de.freakydu.freakymate
readonly moonraker_port=7125
fresh=false
case "${1:-}" in
  --fresh) fresh=true ;;
  "") ;;
  *) echo "usage: $0 [--fresh]" >&2; exit 2 ;;
esac

repo=$(cd "$(dirname "$0")/.." && pwd)
sdk=${ANDROID_HOME:-${ANDROID_SDK_ROOT:-}}
[ -n "$sdk" ] || { echo "set ANDROID_HOME to the Android SDK" >&2; exit 1; }
export ANDROID_HOME=$sdk
ndk_version=$(sed -n 's/^ *ndkVersion = "\(.*\)"/\1/p' "$repo/android/app/build.gradle.kts")
export ANDROID_NDK_HOME=$sdk/ndk/$ndk_version
adb=$sdk/platform-tools/adb
if [ ! -x "$adb" ]; then
  echo "no adb in $sdk/platform-tools: ANDROID_HOME=$sdk is not the SDK (an editor started before it moved keeps" >&2
  echo "the old value; restart it)" >&2
  exit 1
fi

start_emulator() {
  local avd=${FREAKYMATE_AVD:-freakymate-test}
  local log=${TMPDIR:-/tmp}/freakymate-emulator.log
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
export FREAKYMATE_VERSION=$version
echo "building $version for $abi"
(cd "$repo/android" && ./gradlew -q assembleRelease "-PappVersion=$version" "-Pabis=$abi")
apk=$repo/android/app/build/outputs/apk/release/freakymate-$version-$abi-release.apk

if ! "$adb" install -r "$apk"; then
  echo "install failed; an APK signed with another key (release, CI) must be uninstalled first:" >&2
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
echo "FreakyMate runs (pid $pid); stop with Ctrl+C or the stop button of the debugger"
"$adb" logcat -v time --pid="$pid"
