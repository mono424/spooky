#!/usr/bin/env bash
# Build the ssp-ffi native library and copy it into the spooky_core package
# under native/<platform>/ so dart:ffi can locate it.
set -euo pipefail

cd "$(dirname "$0")/../.."

cargo build --release -p ssp-ffi

DEST_ROOT="packages/spooky_core/native"

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
