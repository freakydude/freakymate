#!/bin/bash
# Installs the Android SDK parts android/app/build.gradle.kts needs (platform, build-tools, NDK) with the sdkmanager of the
# GitHub runner and exports ANDROID_HOME, ANDROID_NDK_HOME/ANDROID_NDK_ROOT and ANDROID_JAR to the following steps.
# Versions are the ones android/ and crates/android are built and tested with.
set -euo pipefail

platform=36
build_tools=36.0.0
ndk=30.0.16248370

sdk=${ANDROID_HOME:-${ANDROID_SDK_ROOT:-/usr/local/lib/android/sdk}}
sdkmanager=$(command -v sdkmanager || echo "$sdk/cmdline-tools/latest/bin/sdkmanager")

yes | "$sdkmanager" --sdk_root="$sdk" --licenses > /dev/null || true
"$sdkmanager" --sdk_root="$sdk" "platforms;android-$platform" "build-tools;$build_tools" "ndk;$ndk" > /dev/null

{
  echo "ANDROID_HOME=$sdk"
  echo "ANDROID_NDK_HOME=$sdk/ndk/$ndk"
  # The runner sets ANDROID_NDK_ROOT to an older NDK; cargo-ndk warns when the two differ.
  echo "ANDROID_NDK_ROOT=$sdk/ndk/$ndk"
  echo "ANDROID_JAR=$sdk/platforms/android-$platform/android.jar"
} >> "${GITHUB_ENV:?runs inside GitHub Actions}"
