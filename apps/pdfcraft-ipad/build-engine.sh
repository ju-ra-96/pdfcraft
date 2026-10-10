#!/usr/bin/env bash
# Xcode build phase: compile PdfCraft's Rust engine (this folder's crate, pdfcraft-ipad) for the
# platform Xcode is building, and put libpdfcraft_ipad.a where the app links it from
# ($DERIVED_FILE_DIR/rust, see LIBRARY_SEARCH_PATHS).
set -euo pipefail
export PATH="$HOME/.cargo/bin:/opt/homebrew/bin:/usr/local/bin:$PATH"
command -v cargo >/dev/null || { echo "error: cargo not found; install Rust from https://rustup.rs"; exit 1; }

case "${PLATFORM_NAME:-}" in
  iphonesimulator)
    case "${NATIVE_ARCH_ACTUAL:-arm64}" in
      x86_64) TARGET=x86_64-apple-ios ;;
      *) TARGET=aarch64-apple-ios-sim ;;
    esac
    ;;
  iphoneos) TARGET=aarch64-apple-ios ;;
  *) echo "error: PdfCraft for iPad builds for iphoneos or iphonesimulator, not ${PLATFORM_NAME:-?}"; exit 1 ;;
esac
if [ "${CONFIGURATION:-Debug}" = Release ]; then
  PROFILE=release
  FLAGS=(--release)
else
  PROFILE=debug
  FLAGS=()
fi

rustup target list --installed 2>/dev/null | grep -qx "$TARGET" || rustup target add "$TARGET"
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$ROOT"
cargo build --locked -p pdfcraft-ipad --lib --target "$TARGET" ${FLAGS[@]+"${FLAGS[@]}"}
mkdir -p "$DERIVED_FILE_DIR/rust"
cp "${CARGO_TARGET_DIR:-$ROOT/target}/$TARGET/$PROFILE/libpdfcraft_ipad.a" "$DERIVED_FILE_DIR/rust/libpdfcraft_ipad.a"
