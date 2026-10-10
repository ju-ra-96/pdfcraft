# PdfCraft for iPad

PdfCraft runs on iPad as a native app: the same Rust engine and egui interface as the desktop,
in one full-screen UIKit window (winit's iOS backend), drawn with wgpu on Metal. The crate is
`apps/pdfcraft-ios`; packaging is `packaging/ios/`.

It is developed against iPadOS 26 on the iPad Air (M4) with Apple Pencil Pro, and runs on any
iPad with iPadOS 15 or later.

## Apple Pencil and touch

On the page:

| Input | Does |
|---|---|
| Apple Pencil | The pointer: draws, highlights, selects text, moves comments, as a mouse does on the desktop |
| Pencil pressure | With **Draw**, a firmer press draws a wider line. Normal handwriting draws the set width, the lightest touch about a third of it, the firmest about 1.65× |
| One or two fingers | Scroll. A flick keeps gliding |
| Pinch | Zoom, around the fingers |
| Tap | Click: follow a link, fill a field, choose a comment |
| Long press | Right-click: context menus |
| Palm or finger while the Pencil draws | Ignored |

On the toolbars, panels, menus and dialogs, a finger is the pointer, so buttons, sliders and
lists work as you'd expect.

**Preferences ▸ Apple Pencil ▸ Only the Apple Pencil draws; fingers scroll and zoom** is on as
soon as a Pencil has touched the screen. Before that, or with it turned off, one finger on the page
draws and selects like the Pencil once it moves, and two fingers scroll and zoom. Notes works the
same way. Scripts set it with `set_option("pencil_only", "auto" | "on" | "off")`.

Pressure is saved in a standard Ink annotation. Its `/InkList` holds the stroke, which every PDF
viewer draws, and `/PCPressure` holds one pressure per point, which PdfCraft draws as a filled
outline whose width follows the pressure. The appearance stream is written into the file, so other
viewers show the varying width too. The Eraser cuts pressure along with the strokes. Agents add
the same strokes with `comment_add` (`"type": "ink"`, `"strokes"`, `"pressure"`).

How it works: egui-winit turns the first touch into a mouse, so on its own every finger would draw.
In tablet mode, `crates/ui-egui/src/touch.rs` rewrites each frame's touches before egui sees them.
On iPad a Pencil touch is the only kind that reports a force (no iPad has 3D Touch), which is how
the router tells the Pencil from a finger.

## Files

iPadOS gives the app a sandbox. Its **Documents** folder shows in the Files app as
**On My iPad ▸ PdfCraft**. Drag PDFs there in Files, or from a Mac in the Finder with the iPad
connected. Home lists that folder, and every Open, Insert, Combine and Save dialog is an in-app
picker over it (`crates/ui-egui/src/in_app_picker.rs`). The picker can't leave the folder, and Save
asks before replacing a file.

A PDF shared to PdfCraft from another app (Share ▸ PdfCraft, AirDrop) arrives in
`Documents/Inbox`. The app moves it into Documents, renaming it `name 2.pdf` if needed, and opens
it, even while PdfCraft is already open.

Settings and crash recovery live in `Library/Application Support/PdfCraft` inside the sandbox.

## Build and run

You need macOS with Xcode installed, and the Rust targets (`package.sh` adds them if missing).

```sh
# Simulator: build, bundle, boot an iPad simulator, install and launch, with its log here.
packaging/ios/package.sh --run                        # release build, iPad Pro 13-inch (M5)
packaging/ios/package.sh --debug --run "iPad Air 13-inch (M4)"
```

The script takes the iOS SDK and `simctl` from Xcode by path, and compiles and links with the
active developer tools (Command Line Tools are enough). So it works even before the Xcode licence
has been accepted.

### On your iPad

Apps on an iPad must be signed with an Apple development certificate and a provisioning profile
that names the device. A free Apple ID works for your own device, though its profiles expire after
7 days.

1. Accept the Xcode licence once: `sudo xcodebuild -license accept`.
2. Connect the iPad, unlock it, and trust the Mac. On the iPad, turn on **Settings ▸ Privacy &
   Security ▸ Developer Mode** and restart when asked.
3. Get a profile. In Xcode, sign in under **Settings ▸ Accounts**. Then create any iOS App project
   with the bundle identifier `ai.storyteller.pdfcraft` (or your own, below), pick your team under
   **Signing & Capabilities**, and run it once on the iPad. Xcode creates the profile in
   `~/Library/Developer/Xcode/UserData/Provisioning Profiles/`.
4. Build, sign and install:

   ```sh
   export IOS_SIGN_IDENTITY="Apple Development: you@example.com (TEAMID1234)"   # security find-identity -v -p codesigning
   export IOS_PROVISIONING_PROFILE=~/Library/Developer/Xcode/UserData/Provisioning\ Profiles/<uuid>.mobileprovision
   # export IOS_BUNDLE_ID=com.example.pdfcraft   # if your profile is for another identifier
   packaging/ios/package.sh --device
   xcrun devicectl list devices
   xcrun devicectl device install app --device <device id> target/ios-package/release-iPhoneOS/PdfCraft.app
   ```

   `--device` also writes `dist/release/pdfcraft-<version>-ipados.ipa`, which Apple Configurator
   or TestFlight can install.

## What doesn't work yet

- **Apple Pencil Pro hover, squeeze and double-tap, and haptics.** These need UIKit interactions
  (`UIHoverGestureRecognizer`, `UIPencilInteraction`) that winit doesn't provide. Adding them means
  Objective-C classes, i.e. `unsafe`, in the vendored winit, which needs the owner's sign-off
  (AGENTS.md §4).
- **Tilt (altitude and azimuth) and barrel roll.** winit reports the Pencil's altitude, but
  egui-winit drops it, and azimuth and roll aren't reported at all. Width follows pressure only.
- **Every Pencil sample.** The Pencil reports 240 times a second, but winit passes one sample per
  UIKit event, and the stroke takes one per frame (60 on iPad Air, 120 on ProMotion iPads).
  Strokes are smoothed along a curve through the samples, but very fast flourishes lose detail.
  Predicted touches (lower latency) aren't used either.
- **Files outside the app.** Opening files in place from iCloud Drive or other providers needs
  UIKit's document picker (`unsafe`, as above). Copy files into On My iPad ▸ PdfCraft, or share them
  to PdfCraft.
- **Split View, Slide Over and several windows.** The app is full-screen only
  (`UIRequiresFullScreen`).
- **The system clipboard.** egui-winit has no iOS clipboard, so copied text stays inside
  PdfCraft (copy and paste work within the app), and Snapshot and Create ▸ From clipboard, which
  use arboard, don't work.
- **The system text-selection menu and handles**, and the system **context menus**. A long press
  shows PdfCraft's own menu instead.
- **Tested on hardware:** not yet. It has been tested in the iOS Simulator (iPad Pro 13-inch M5,
  iPad Air 13-inch M4) and in headless UI tests (`crates/ui-egui/tests/tablet.rs`). The Simulator
  has no Apple Pencil, so pressure and palm rejection are covered by those tests.
