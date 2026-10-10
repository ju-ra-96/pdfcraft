# PdfCraft for iPad

PdfCraft for iPad is a native SwiftUI app built on Apple's frameworks, with PdfCraft's Rust engine underneath:

| Part | What it is |
|---|---|
| Documents | Apple's document browser (`DocumentGroup`): Files, iCloud Drive, other providers, recents, tags, one window per document, Stage Manager and Split View |
| Viewing | PDFKit: continuous pages, pinch zoom, page thumbnails, the system find bar (⌘F), form filling, text selection |
| Apple Pencil | PencilKit: Apple's tool picker and inks, pressure, tilt and barrel roll, hover preview, double-tap and squeeze (as set in Settings ▸ Apple Pencil), palm rejection, haptics, "Only Draw with Apple Pencil" |
| Sharing | The system share sheet |
| Text recognition | Apple's Vision framework reads scanned pages; the engine adds the words as invisible, searchable text |
| Pro tools | PdfCraft's engine (`apps/pdfcraft-ipad/src/lib.rs`): editable pressure-sensitive Ink, digital signatures, redaction, watermarks, inserting pages, file-size reduction, flattening, and every other tool of `pdfcraft-cli tools` |

It is developed against iPadOS 26 on the iPad Air 11-inch (M4) with Apple Pencil Pro. It needs iPadOS 18 or later.

## Using it

- **Open** a PDF from the document browser, from Files (Open With ▸ PdfCraft), or with Share ▸ PdfCraft from another app. **Create Document** makes a blank page.
- **Markup** (the pencil button, ⇧⌘E) shows Apple's tool picker; draw with the Apple Pencil. While Markup is on, fingers still scroll and zoom the pages unless you turn off **Only Draw with Apple Pencil** in Settings ▸ Apple Pencil.
  - PencilKit's own undo, eraser and lasso work on the strokes.
  - **Done** saves them into the document, in one undo step (see below). So does leaving the app.
- **Select text** (long press, or drag with the Pencil), then use **Highlight**, **Underline**, **Strike Through** or **Redact** from the edit menu or the toolbar.
- **Page** menu: rotate the current page, insert a blank page after it, delete it.
- **Tools** menu:
  - **Recognize Text**: Apple's Vision reads every page that has no text yet (a scan, a photo), and the words become searchable and selectable. The page image isn't changed.
  - **Sign with a Digital ID…**: import a `.p12`/`.pfx` from Files, or create a self-signed ID. Enter its password and an optional reason. The signature is invisible unless the document has an empty signature field. The signed file is saved byte for byte, so the signature stays valid. Later Pencil strokes and Tools are added as incremental updates, which a signature allows. A highlight, a page change or a filled field is PDFKit's own edit: it rewrites the file and invalidates the signature, so sign last.
  - **Redact Text…**: every match is removed for good, with a black box in its place.
  - **Protect with a Password…**: Apple's PDF encryption, applied each time the document saves.
  - **Insert Pages from a File…**: every page of a PDF picked in Files, after the current page.
  - **Add Watermark…**: text across every page, semi-transparent.
  - **Reduce File Size**
  - **Flatten Comments and Fields**
- Every change is undoable (⌘Z, or a three-finger swipe) and saves automatically.

## How Pencil strokes are kept

PencilKit works out each point's width from the Pencil's pressure, tilt, speed and roll. When strokes are saved, `InkConverter.swift` turns each one into an annotation:

- **Pens, the marker, the monoline and fountain pens** become standard Ink annotations:
  - the points go into `/InkList`, which every PDF viewer draws;
  - the widths go into PdfCraft's `/PCPressure`, one value per point.

  The engine writes an appearance stream: a filled outline whose width follows those values. So other viewers show the same varying stroke, and PdfCraft on any platform can still edit it (move it, recolour it, erase part of it).
- **Textured inks** (pencil, watercolor, crayon) have grain and blending that Ink can't hold. PencilKit draws them, and the picture is saved as a custom stamp in the stroke's exact rectangle (at 216 dpi), so they look as they did on screen. They can be moved and deleted, not recoloured.
- **A stroke cut by the pixel eraser** keeps only its visible parts: the stroke splits where the eraser crossed it.

PDFKit keeps `/PCPressure` and the appearance when it re-saves a document (checked with a synthetic file). Edits made by the engine are saved exactly as the engine wrote them, as incremental updates, until PDFKit itself changes the document (a highlight, a page turned, a field filled).

## Build and run

You need Xcode 16 or later and Rust (`rustup`). The project builds the engine itself, in a build phase (`build-engine.sh`) that runs cargo for the platform Xcode is building.

- **In Xcode:** open `apps/pdfcraft-ipad/PdfCraft.xcodeproj`, choose an iPad simulator or your iPad, and press Run.
- **From the command line** (simulator):

  ```sh
  xcodebuild -project apps/pdfcraft-ipad/PdfCraft.xcodeproj -scheme PdfCraft \
    -destination 'platform=iOS Simulator,name=iPad Air 11-inch (M4)' build
  ```

- **UI tests** (XCTest): `xcodebuild test` with the same arguments. They open a sample PDF from the document browser and:
  - create a new document;
  - draw in Markup and save the stroke through the engine, and save a textured pencil stroke as a picture;
  - highlight selected text;
  - redact a selected word (it must be gone from the page's text);
  - recognize the text of a scanned page with Vision;
  - create a digital ID and sign;
  - add a watermark, run Reduce File Size and Insert Blank Page, and open the find bar.

### On your iPad

1. In Xcode, select the **PdfCraft** target ▸ **Signing & Capabilities**. Under **Team**, pick your Apple ID; add it in Xcode ▸ Settings ▸ Accounts first if needed. A free Apple ID works for your own iPad, though its profile expires after 7 days.
2. Connect the iPad and unlock it. On the iPad, turn on **Settings ▸ Privacy & Security ▸ Developer Mode** and restart when asked.
3. Choose the iPad as the run destination and press Run.

If the bundle identifier `ai.storyteller.pdfcraft` is taken for your team, change it under the same Signing tab.

## What's missing

- **Create Document fails in the iOS 26.5 Simulator.** SwiftUI's document group writes the new file (it appears in On My iPad ▸ PdfCraft and opens from there). Then the system's Files service can't resolve its own bookmark for it ("Could not resolve bookmark", FP -1005) and shows "Unable to Import Document". None of PdfCraft's code runs at that point. The UI test records it as an expected failure in the Simulator; check it first on an iPad.
- **Not all of PdfCraft's tools have an iPad interface yet**: Bates numbering, headers and footers, forms authoring, comparing, exporting to Office and others. The engine has them; each is one `PdfCraftEngine.shared` call from the Tools menu.
- **Not yet tried on hardware.** It has been tested in the iOS Simulator (iPad Air 11-inch M4) and with the engine's Rust tests (`cargo test -p pdfcraft-ipad`). The Simulator has no Apple Pencil, so pressure, hover and squeeze come from PencilKit untested here; the UI tests draw with a finger.

## Layout

| Path | What |
|---|---|
| `apps/pdfcraft-ipad/PdfCraft.xcodeproj` | The Xcode project (app and UI-test targets, shared scheme) |
| `apps/pdfcraft-ipad/PdfCraft/` | The SwiftUI app: `PdfCraftApp.swift`, `DocumentView.swift` (toolbar and actions), `PDFController.swift` (PDFKit and PencilKit), `InkConverter.swift`, `ProTools.swift` (Vision text recognition, digital IDs), `SignSheet.swift`, `PDFFile.swift` (the document), `Engine.swift` (the engine bridge) |
| `apps/pdfcraft-ipad/src/lib.rs`, `include/pdfcraft.h` | The engine's C interface: one JSON request in, one JSON response out, over the automation tool table |
| `apps/pdfcraft-ipad/PdfCraftUITests/` | XCTest UI tests |

The C interface is the only `unsafe` code in the workspace: four exported functions, each with its safety contract, and no panic crosses them. The repository owner approved this exception to `AGENTS.md` §4 for the iPad app.
