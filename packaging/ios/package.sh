#!/usr/bin/env bash
# Build PdfCraft for iPad and wrap it in an app bundle:
#
#   simulator: $CARGO_TARGET_DIR/ios-package/<profile>/PdfCraft.app   (ad-hoc signed)
#   device:    $DIST/pdfcraft-<version>-ipados.ipa                      (signed with your identity)
#
# Usage: packaging/ios/package.sh [--device] [--debug] [--skip-build] [--run [SIMULATOR]]
#
#   --device       build for iPad hardware (aarch64-apple-ios) instead of the simulator
#   --debug        debug profile (default: release)
#   --skip-build   reuse the last cargo build
#   --run [NAME]   simulator only: boot NAME (default "iPad Air 11-inch (M4)", or the first iPad
#                  available), install the app and launch it, streaming its log to this terminal
#
# Device signing (env; needed with --device):
#   IOS_SIGN_IDENTITY        codesign identity, e.g. "Apple Development: Jane Doe (ABCDE12345)"
#   IOS_PROVISIONING_PROFILE path to a .mobileprovision that covers IOS_BUNDLE_ID and the iPad
# Optional:
#   IOS_BUNDLE_ID            bundle identifier (default ai.storyteller.pdfcraft)
#   XCODE_DEVELOPER_DIR      Xcode whose iOS SDK and simulator to use
#                            (default: /Applications/Xcode.app/Contents/Developer)
set -euo pipefail
# shellcheck source=../env.sh
. "$(dirname "${BASH_SOURCE[0]}")/../env.sh"
HERE="$ROOT/packaging/ios"

DEVICE=0
PROFILE=release
SKIP_BUILD=0
RUN=0
SIM_NAME=""
while [ $# -gt 0 ]; do
  case "$1" in
    --device) DEVICE=1; shift ;;
    --debug) PROFILE=debug; shift ;;
    --skip-build) SKIP_BUILD=1; shift ;;
    --run)
      RUN=1
      shift
      if [ $# -gt 0 ] && [ "${1#--}" = "$1" ]; then SIM_NAME="$1"; shift; fi
      ;;
    -h | --help) sed -n '2,22p' "$0"; exit 0 ;;
    *) echo "unknown argument: $1" >&2; exit 2 ;;
  esac
done
if [ "$DEVICE" = 1 ] && [ "$RUN" = 1 ]; then
  echo "error: --run launches the simulator build; install a --device build with Xcode or Apple Configurator" >&2
  exit 2
fi

# Only the SDK and simctl come from Xcode, by path. Compilers and the linker run through the
# active developer directory (Command Line Tools are enough), and nothing goes through xcrun with
# Xcode selected: that refuses to run until the Xcode licence has been accepted.
XCODE="${XCODE_DEVELOPER_DIR:-/Applications/Xcode.app/Contents/Developer}"
if [ "$DEVICE" = 1 ]; then
  TARGET=aarch64-apple-ios
  PLATFORM=iPhoneOS
else
  TARGET=aarch64-apple-ios-sim
  PLATFORM=iPhoneSimulator
fi
SDKROOT="$XCODE/Platforms/$PLATFORM.platform/Developer/SDKs/$PLATFORM.sdk"
if [ ! -d "$SDKROOT" ]; then
  echo "error: no $PLATFORM SDK at $SDKROOT (install Xcode, or set XCODE_DEVELOPER_DIR)" >&2
  exit 1
fi
export SDKROOT
# Keep in sync with MinimumOSVersion in Info.plist.in.
export IPHONEOS_DEPLOYMENT_TARGET=15.0
BUNDLE_ID="${IOS_BUNDLE_ID:-ai.storyteller.pdfcraft}"
SHORT_VERSION="${VERSION%%-*}"
WORK="$CARGO_TARGET_DIR/ios-package/$PROFILE-$PLATFORM"
APP="$WORK/PdfCraft.app"

echo "==> PdfCraft $VERSION for iPadOS ($TARGET, $PROFILE)"

# ---- build -------------------------------------------------------------------------------------
if [ "$SKIP_BUILD" = 0 ]; then
  rustup target list --installed 2>/dev/null | grep -qx "$TARGET" || rustup target add "$TARGET"
  flags=()
  [ "$PROFILE" = release ] && flags+=(--release)
  (cd "$ROOT" && cargo build --locked -p pdfcraft-ios --target "$TARGET" ${flags[@]+"${flags[@]}"})
fi
BIN="$CARGO_TARGET_DIR/$TARGET/$PROFILE/pdfcraft-ios"
[ -f "$BIN" ] || { echo "error: $BIN not built" >&2; exit 1; }

# ---- bundle ------------------------------------------------------------------------------------
rm -rf "$APP"
mkdir -p "$APP"
cp "$BIN" "$APP/PdfCraft"
sed -e "s|@VERSION@|$VERSION|g" \
  -e "s|@SHORT_VERSION@|$SHORT_VERSION|g" \
  -e "s|@BUILD_SHA@|${PDFCRAFT_BUILD_SHA:-}|g" \
  -e "s|@BUNDLE_ID@|$BUNDLE_ID|g" \
  -e "s|@PLATFORM@|$PLATFORM|g" \
  "$HERE/Info.plist.in" >"$APP/Info.plist"
plutil -lint "$APP/Info.plist" >/dev/null
printf 'APPL????' >"$APP/PkgInfo"
# Home-screen icons, scaled from the committed full-bleed master (no new asset files).
ICON="$ROOT/assets/app-icon/hicolor/512x512/apps/ai.storyteller.pdfcraft.png"
for spec in "AppIcon76x76.png 76" "AppIcon76x76@2x.png 152" "AppIcon83.5x83.5@2x.png 167"; do
  set -- $spec
  sips -z "$2" "$2" "$ICON" --out "$APP/$1" >/dev/null
done
cp "$ROOT/LICENSE-MIT" "$ROOT/LICENSE-APACHE" "$ROOT/NOTICE" "$APP/"
copy_font_licences "$APP"

# ---- sign --------------------------------------------------------------------------------------
if [ "$DEVICE" = 0 ]; then
  codesign --force --sign - --timestamp=none "$APP"
else
  : "${IOS_SIGN_IDENTITY:?set IOS_SIGN_IDENTITY to your Apple Development or Distribution identity}"
  : "${IOS_PROVISIONING_PROFILE:?set IOS_PROVISIONING_PROFILE to a .mobileprovision for $BUNDLE_ID}"
  cp "$IOS_PROVISIONING_PROFILE" "$APP/embedded.mobileprovision"
  ENT="$WORK/entitlements.plist"
  security cms -D -i "$IOS_PROVISIONING_PROFILE" >"$WORK/profile.plist"
  plutil -extract Entitlements xml1 -o "$ENT" "$WORK/profile.plist"
  codesign --force --sign "$IOS_SIGN_IDENTITY" --entitlements "$ENT" --timestamp=none "$APP"
  IPA="$DIST/pdfcraft-$VERSION-ipados.ipa"
  rm -rf "$WORK/Payload" "$IPA"
  mkdir -p "$WORK/Payload"
  cp -R "$APP" "$WORK/Payload/"
  (cd "$WORK" && zip -qry "$IPA" Payload)
  echo "==> $IPA"
fi
codesign --verify --strict "$APP"
echo "==> $APP"

# ---- run in the simulator ----------------------------------------------------------------------
if [ "$RUN" = 1 ]; then
  SIMCTL="$XCODE/usr/bin/simctl"
  devices="$("$SIMCTL" list devices available 2>/dev/null)"
  pick() { grep -F "    $1 (" <<<"$devices" | head -1 | sed -E 's/.*\(([0-9A-F-]{36})\).*/\1/'; }
  UDID=""
  [ -n "$SIM_NAME" ] && UDID="$(pick "$SIM_NAME")"
  [ -z "$UDID" ] && [ -z "$SIM_NAME" ] && UDID="$(pick "iPad Air 11-inch (M4)")"
  [ -z "$UDID" ] && [ -z "$SIM_NAME" ] && UDID="$(grep -E '^    iPad' <<<"$devices" | head -1 | sed -E 's/.*\(([0-9A-F-]{36})\).*/\1/')"
  [ -n "$UDID" ] || { echo "error: no available iPad simulator${SIM_NAME:+ named \"$SIM_NAME\"}" >&2; exit 1; }
  "$SIMCTL" boot "$UDID" 2>/dev/null || true
  # Show the device. By path: the viewer lives inside Xcode, where `open -a` may not find it.
  # Xcode 27 shows simulators in DeviceHub; earlier versions have Simulator.app.
  if [ -d "$XCODE/Applications/Simulator.app" ]; then
    open "$XCODE/Applications/Simulator.app" --args -CurrentDeviceUDID "$UDID" || warn "could not open Simulator"
  elif [ -d "$XCODE/../Applications/DeviceHub.app" ]; then
    open "$XCODE/../Applications/DeviceHub.app" || warn "could not open DeviceHub"
  else
    warn "no Simulator or DeviceHub app in Xcode: the app runs, but no window shows it"
  fi
  "$SIMCTL" bootstatus "$UDID" -b >/dev/null
  "$SIMCTL" install "$UDID" "$APP"
  echo "==> launching $BUNDLE_ID on $UDID (Ctrl-C stops streaming, the app keeps running)"
  "$SIMCTL" launch --console-pty --terminate-running-process "$UDID" "$BUNDLE_ID"
fi
