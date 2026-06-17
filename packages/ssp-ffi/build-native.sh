#!/usr/bin/env bash
# Build the ssp-ffi native library and copy it into the spooky_core package
# under native/<platform>/ so dart:ffi can locate it.
#
# Usage:
#   build-native.sh            Build for the host platform (macOS/Linux/Windows).
#   build-native.sh android    Cross-build the Android cdylib (arm64-v8a + x86_64)
#                              via cargo-ndk into native/android/<abi>/libssp_ffi.so.
#   build-native.sh ios        Cross-build the iOS static libs and package them as
#                              native/ios/ssp_ffi.xcframework (device + simulator).
set -euo pipefail

cd "$(dirname "$0")/../.."

DEST_ROOT="packages/spooky_core/native"

build_host() {
  cargo build --release -p ssp-ffi
  case "$(uname -s)" in
    Darwin)
      mkdir -p "$DEST_ROOT/macos"
      cp target/release/libssp_ffi.dylib "$DEST_ROOT/macos/libssp_ffi.dylib"
      echo "Copied libssp_ffi.dylib -> $DEST_ROOT/macos/"
      ;;
    Linux)
      mkdir -p "$DEST_ROOT/linux"
      cp target/release/libssp_ffi.so "$DEST_ROOT/linux/libssp_ffi.so"
      echo "Copied libssp_ffi.so -> $DEST_ROOT/linux/"
      ;;
    MINGW* | MSYS* | CYGWIN*)
      mkdir -p "$DEST_ROOT/windows"
      cp target/release/ssp_ffi.dll "$DEST_ROOT/windows/ssp_ffi.dll"
      echo "Copied ssp_ffi.dll -> $DEST_ROOT/windows/"
      ;;
    *)
      echo "Unsupported platform: $(uname -s)" >&2
      exit 1
      ;;
  esac
}

build_android() {
  # ABIs mirror the app's other native libs (untangle ships arm64-v8a + x86_64).
  # cargo-ndk lays the output out as <out>/<abi>/libssp_ffi.so using Gradle's ABI
  # dir names, which is exactly the jniLibs layout Flutter merges into the APK.
  # --platform 21 matches the app's minSdk; NDK r26+ links 16KB-aligned .so by
  # default (the app sets useLegacyPackaging=false, which expects that).
  if ! command -v cargo-ndk >/dev/null 2>&1; then
    echo "cargo-ndk not found. Install it with: cargo install cargo-ndk" >&2
    exit 1
  fi
  rustup target add aarch64-linux-android x86_64-linux-android

  mkdir -p "$DEST_ROOT/android"
  cargo ndk -t arm64-v8a -t x86_64 --platform 21 \
    -o "$DEST_ROOT/android" \
    build --release -p ssp-ffi
  echo "Built libssp_ffi.so (arm64-v8a + x86_64) -> $DEST_ROOT/android/"
}

build_ios() {
  # iOS bans loose dylibs, so produce a static .xcframework that the consumer
  # links into its host binary (Dart then resolves symbols via
  # DynamicLibrary.process()). Device slice + a fat simulator slice (arm64 + x64).
  if [ "$(uname -s)" != "Darwin" ]; then
    echo "iOS build requires macOS (xcodebuild/lipo)." >&2
    exit 1
  fi
  rustup target add aarch64-apple-ios aarch64-apple-ios-sim x86_64-apple-ios

  local LIB="libssp_ffi.a"
  cargo build --release -p ssp-ffi --target aarch64-apple-ios
  cargo build --release -p ssp-ffi --target aarch64-apple-ios-sim
  cargo build --release -p ssp-ffi --target x86_64-apple-ios

  local SIM_DIR="target/ios-sim"
  mkdir -p "$SIM_DIR"
  lipo -create \
    "target/aarch64-apple-ios-sim/release/$LIB" \
    "target/x86_64-apple-ios/release/$LIB" \
    -output "$SIM_DIR/$LIB"

  local OUT="$DEST_ROOT/ios/ssp_ffi.xcframework"
  rm -rf "$OUT"
  mkdir -p "$DEST_ROOT/ios"
  xcodebuild -create-xcframework \
    -library "target/aarch64-apple-ios/release/$LIB" \
    -library "$SIM_DIR/$LIB" \
    -output "$OUT"
  echo "Built ssp_ffi.xcframework -> $OUT"
}

case "${1:-host}" in
  host) build_host ;;
  android) build_android ;;
  ios) build_ios ;;
  all)
    build_host
    build_android
    build_ios
    ;;
  *)
    echo "Unknown mode: $1 (expected: host | android | ios | all)" >&2
    exit 1
    ;;
esac
