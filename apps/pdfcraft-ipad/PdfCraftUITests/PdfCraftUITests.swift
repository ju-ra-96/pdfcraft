import XCTest

/// PdfCraft for iPad end to end: Apple's document browser, PDFKit, PencilKit and the Rust engine
/// together. The Simulator has no Apple Pencil, so strokes are finger drags (PencilKit draws
/// with any input unless Settings ▸ Apple Pencil ▸ Only Draw with Apple Pencil is on).
///
/// Each test opens `Sample.pdf`, which the app writes to its Documents folder when launched with
/// `-PdfCraftUITestSample`, from Browse ▸ On My iPad ▸ PdfCraft.
final class PdfCraftUITests: XCTestCase {
    override func setUp() {
        continueAfterFailure = false
    }

    private func openSample(_ extra: [String] = []) -> XCUIApplication {
        let app = XCUIApplication()
        app.launchArguments += ["-PdfCraftUITest", "-PdfCraftUITestSample", "-AppleLanguages", "(en)", "-AppleLocale", "en_US"] + extra
        app.launch()
        let browse = app.buttons.matching(identifier: "Browse").firstMatch
        XCTAssertTrue(browse.waitForExistence(timeout: 30), "the document browser shows")
        browse.tap()
        // The browser reopens where it was; otherwise go to On My iPad ▸ PdfCraft.
        if !app.staticTexts["Sample"].waitForExistence(timeout: 5) {
            for name in ["On My iPad", "PdfCraft"] {
                // A folder in the grid, or a location in the sidebar.
                let cell = app.cells.containing(.staticText, identifier: name).firstMatch
                let label = app.staticTexts.matching(identifier: name).firstMatch
                if cell.waitForExistence(timeout: 5) { cell.tap() } else if label.exists { label.tap() }
            }
        }
        let name = app.staticTexts["Sample"]
        XCTAssertTrue(name.waitForExistence(timeout: 15), "Sample is listed")
        // The thumbnail opens the document (tapping the name renames it).
        let cell = app.cells.containing(.staticText, identifier: "Sample").firstMatch
        if cell.exists {
            cell.coordinate(withNormalizedOffset: CGVector(dx: 0.5, dy: 0.25)).tap()
        } else {
            name.coordinate(withNormalizedOffset: CGVector(dx: 0.5, dy: -2.5)).tap()
        }
        XCTAssertTrue(app.staticTexts["status"].waitForExistence(timeout: 30), "the document opens")
        return app
    }

    private func status(_ app: XCUIApplication) -> String { app.staticTexts["status"].label }

    private func waitForStatus(_ app: XCUIApplication, matches pattern: String, timeout: TimeInterval = 60) {
        let found = expectation(for: NSPredicate(format: "label MATCHES %@", pattern), evaluatedWith: app.staticTexts["status"])
        wait(for: [found], timeout: timeout)
    }

    private func tool(_ app: XCUIApplication, _ name: String) {
        app.buttons["tools"].tap()
        let item = app.buttons[name]
        XCTAssertTrue(item.waitForExistence(timeout: 5), "\(name) is in Tools")
        item.tap()
    }

    private func waitForStatus(_ app: XCUIApplication, contains part: String, timeout: TimeInterval = 30) {
        let found = expectation(for: NSPredicate(format: "label CONTAINS %@", part), evaluatedWith: app.staticTexts["status"])
        wait(for: [found], timeout: timeout)
    }

    private func attach(_ app: XCUIApplication, _ name: String) {
        let shot = XCTAttachment(screenshot: app.screenshot())
        shot.name = name
        shot.lifetime = .keepAlways
        add(shot)
    }

    private func annotations(_ app: XCUIApplication) -> Int {
        let words = status(app).split(separator: " ")
        guard let i = words.firstIndex(of: "annotations"), i + 1 < words.count else { return -1 }
        return Int(words[i + 1]) ?? -1
    }

    /// Draw in Markup, tap Done: the PencilKit stroke becomes an Ink annotation saved by the engine.
    func testMarkupStrokesBecomeInkAnnotations() {
        let app = openSample()
        let before = annotations(app)
        app.buttons["markup"].tap()
        attach(app, "markup-on")
        let start = app.coordinate(withNormalizedOffset: CGVector(dx: 0.35, dy: 0.55))
        let end = app.coordinate(withNormalizedOffset: CGVector(dx: 0.65, dy: 0.62))
        start.press(forDuration: 0.1, thenDragTo: end)
        waitForStatus(app, contains: "pending 1")
        attach(app, "stroke-drawn")
        app.buttons["markup"].tap()
        waitForStatus(app, contains: "annotations \(before + 1) pending 0")
        XCTAssertFalse(app.alerts.firstMatch.exists, "no error")
        attach(app, "stroke-saved")
    }

    /// Select a word with a long press, then Highlight: PDFKit adds the highlight annotation.
    func testSelectedTextIsHighlighted() {
        let app = openSample()
        let before = annotations(app)
        app.coordinate(withNormalizedOffset: CGVector(dx: 0.45, dy: 0.2)).press(forDuration: 1.2)
        attach(app, "selected")
        let menuItem = app.menuItems["Highlight"]
        let toolbarButton = app.buttons["Highlight"]
        XCTAssertTrue(menuItem.waitForExistence(timeout: 5) || toolbarButton.exists, "a Highlight command is offered")
        attach(app, "offered")
        if menuItem.exists { menuItem.tap() } else { toolbarButton.tap() }
        waitForStatus(app, contains: "annotations \(before + 1)")
        attach(app, "highlighted")
    }

    /// The engine's tools run inside the app, and the page commands work.
    func testToolsAndPages() {
        let app = openSample()
        app.buttons["tools"].tap()
        app.buttons["Reduce File Size"].tap()
        waitForStatus(app, contains: "revision 1")
        XCTAssertFalse(app.alerts.firstMatch.exists, "no error")
        app.buttons["pages"].tap()
        app.buttons["Insert Blank Page"].tap()
        waitForStatus(app, contains: "pages 4")
        attach(app, "four-pages")
        app.buttons["find"].tap()
        attach(app, "find")
        app.typeText("fox")
        attach(app, "found")
    }

    /// Create Document makes a blank PDF and opens it.
    func testCreateDocument() {
        #if targetEnvironment(simulator)
        // In the iOS 26.5 Simulator (with the iOS 27 SDK), SwiftUI's DocumentGroup writes the new
        // file, then the system's Files service fails to resolve its bookmark ("Could not resolve
        // bookmark", FP -1005) and shows "Unable to Import Document". Nothing of PdfCraft's runs at
        // that point. Recorded as an expected failure until it is checked on an iPad.
        XCTExpectFailure("Simulator: DocumentGroup's Create Document fails inside the Files service", strict: false)
        #endif
        let app = XCUIApplication()
        app.launchArguments += ["-PdfCraftUITest", "-AppleLanguages", "(en)", "-AppleLocale", "en_US"]
        app.launch()
        let create = app.buttons.matching(identifier: "Create Document").firstMatch
        XCTAssertTrue(create.waitForExistence(timeout: 30))
        create.tap()
        let opened = app.staticTexts["status"].waitForExistence(timeout: 20)
        attach(app, "created")
        XCTAssertTrue(opened, "the new document opens")
    }

    /// Select a word and Redact it: the engine removes exactly that word from the page.
    func testSelectedTextIsRedacted() {
        let app = openSample()
        waitForStatus(app, contains: "brown 20")
        // "brown" on line 1 (as in testSelectedTextIsHighlighted).
        app.coordinate(withNormalizedOffset: CGVector(dx: 0.45, dy: 0.2)).press(forDuration: 1.2)
        let menuItem = app.menuItems["Redact"]
        XCTAssertTrue(menuItem.waitForExistence(timeout: 5) || app.buttons["Redact"].exists, "Redact is offered")
        if menuItem.exists { menuItem.tap() } else { app.buttons["Redact"].tap() }
        waitForStatus(app, contains: "brown 19")
        XCTAssertFalse(app.alerts.firstMatch.exists, "no error")
        attach(app, "redacted")
    }

    /// Recognize Text with Apple's Vision: the scanned last page gets a text layer.
    func testRecognizeTextMakesScansSearchable() {
        let app = openSample()
        waitForStatus(app, contains: "lasttext 0")
        tool(app, "Recognize Text")
        waitForStatus(app, matches: ".*lasttext [1-9][0-9]*$", timeout: 90)
        XCTAssertTrue(app.alerts.staticTexts.containing(NSPredicate(format: "label CONTAINS 'Recognized'")).firstMatch.waitForExistence(timeout: 5))
        attach(app, "recognized")
    }

    /// Create a digital ID and sign: the document gets a signature, saved byte for byte.
    func testSignWithANewDigitalID() {
        let app = openSample()
        tool(app, "Sign with a Digital ID…")
        app.buttons["create-id"].tap()
        let name = app.textFields["id-name"]
        XCTAssertTrue(name.waitForExistence(timeout: 5))
        name.tap()
        name.typeText("Test Signer")
        let secret = app.secureTextFields["id-new-password"]
        secret.tap()
        secret.typeText("secret123")
        app.buttons["create"].tap()
        let sign = app.buttons["sign"]
        XCTAssertTrue(sign.waitForExistence(timeout: 10))
        attach(app, "sign-sheet")
        sign.tap()
        waitForStatus(app, contains: "signatures 1", timeout: 60)
        XCTAssertFalse(app.alerts.firstMatch.exists, "no error")
        attach(app, "signed")
    }

    /// Add Watermark through the engine.
    func testWatermark() {
        let app = openSample()
        tool(app, "Add Watermark…")
        let field = app.alerts.textFields.firstMatch
        XCTAssertTrue(field.waitForExistence(timeout: 5))
        field.typeText("CONFIDENTIAL")
        app.alerts.buttons["Add"].tap()
        waitForStatus(app, contains: "revision 1")
        XCTAssertFalse(app.alerts.firstMatch.exists, "no error")
        attach(app, "watermarked")
    }

    /// A textured ink (the pencil) is saved as PencilKit drew it: a picture stamp in its place.
    func testTexturedPencilStrokeIsSavedAsDrawn() {
        let app = openSample(["-PdfCraftUITestPencilInk"])
        app.buttons["markup"].tap()
        app.coordinate(withNormalizedOffset: CGVector(dx: 0.3, dy: 0.5)).press(forDuration: 0.1, thenDragTo: app.coordinate(withNormalizedOffset: CGVector(dx: 0.7, dy: 0.58)))
        waitForStatus(app, contains: "pending 1")
        app.buttons["markup"].tap()
        waitForStatus(app, contains: "stamps 1")
        XCTAssertFalse(app.alerts.firstMatch.exists, "no error")
        attach(app, "pencil-stroke")
    }
}
