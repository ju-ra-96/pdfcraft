import PDFKit
import SwiftUI
import UniformTypeIdentifiers
import os

let log = Logger(subsystem: "ai.storyteller.pdfcraft", category: "document")

/// A PDF opened from the system document browser (Files, iCloud Drive, other providers).
///
/// PDFKit holds the document while it is open; edits made by PdfCraft's engine round-trip
/// through its bytes ([`replace(with:actionName:undoManager:)`]). Every change registers with the
/// window's undo manager, which is also how SwiftUI knows to autosave.
final class PDFFile: ReferenceFileDocument {
    static var readableContentTypes: [UTType] { [.pdf] }

    @Published private(set) var document: PDFDocument
    /// Bumped whenever `document` is replaced, so views reload.
    @Published private(set) var revision = 0
    /// The password the document is written with (it was opened with one, or protected here):
    /// PDFKit encrypts each save with it.
    private(set) var password: String?
    /// The file exactly as PdfCraft's engine last wrote it (an incremental update, which keeps
    /// digital signatures valid), saved as it is until PDFKit changes the document in place.
    private var engineBytes: Data?

    /// A new document: one blank US Letter page.
    init() {
        let renderer = UIGraphicsPDFRenderer(bounds: CGRect(x: 0, y: 0, width: 612, height: 792))
        let data = renderer.pdfData { context in context.beginPage() }
        document = PDFDocument(data: data) ?? PDFDocument()
    }

    /// Two pages of text and a scanned page (a picture of text, no text layer), for the UI
    /// tests (`-PdfCraftUITestSample` writes it to Documents).
    static func sampleData() -> Data {
        let bounds = CGRect(x: 0, y: 0, width: 612, height: 792)
        let scan = UIGraphicsImageRenderer(size: bounds.size, format: { let f = UIGraphicsImageRendererFormat(); f.scale = 3; return f }()).image { _ in
            UIColor.white.setFill()
            UIRectFill(bounds)
            ("Scanned invoice" as NSString).draw(at: CGPoint(x: 72, y: 90), withAttributes: [.font: UIFont.boldSystemFont(ofSize: 30), .foregroundColor: UIColor.black])
            ("Total amount due: 1234 euros" as NSString).draw(at: CGPoint(x: 72, y: 160), withAttributes: [.font: UIFont.systemFont(ofSize: 22), .foregroundColor: UIColor.black])
        }
        let renderer = UIGraphicsPDFRenderer(bounds: bounds)
        return renderer.pdfData { context in
            for page in 1...2 {
                context.beginPage()
                ("Sample page \(page)" as NSString).draw(at: CGPoint(x: 72, y: 72), withAttributes: [.font: UIFont.boldSystemFont(ofSize: 28)])
                for line in 0..<20 {
                    let text = "Line \(line + 1): The quick brown fox jumps over the lazy dog." as NSString
                    text.draw(at: CGPoint(x: 72, y: 130 + CGFloat(line) * 26), withAttributes: [.font: UIFont.systemFont(ofSize: 16)])
                }
            }
            context.beginPage()
            scan.draw(in: bounds)
        }
    }

    init(configuration: ReadConfiguration) throws {
        log.info("reading \(configuration.file.filename ?? "?", privacy: .private) (\(configuration.contentType.identifier, privacy: .public))")
        guard let data = configuration.file.regularFileContents else {
            log.error("no file contents")
            throw CocoaError(.fileReadCorruptFile)
        }
        // An encrypted document opens locked; DocumentView asks for its password.
        guard let document = PDFDocument(data: data) else { throw CocoaError(.fileReadCorruptFile) }
        self.document = document
    }

    func snapshot(contentType: UTType) throws -> Data {
        log.info("snapshot \(contentType.identifier, privacy: .public)")
        if let engineBytes, password == nil { return engineBytes }
        let options: [PDFDocumentWriteOption: Any] = password.map { [.userPasswordOption: $0, .ownerPasswordOption: $0] } ?? [:]
        guard let data = document.dataRepresentation(options: options) else { throw CocoaError(.fileWriteUnknown) }
        return data
    }

    func fileWrapper(snapshot: Data, configuration: WriteConfiguration) throws -> FileWrapper {
        FileWrapper(regularFileWithContents: snapshot)
    }

    /// The document's current bytes, unencrypted (what the engine works on).
    var data: Data? { engineBytes ?? document.dataRepresentation() }

    /// Replace the document with `data` (an engine edit), undoably.
    func replace(with data: Data, actionName: String, undoManager: UndoManager?) {
        guard let new = PDFDocument(data: data) else { return }
        let old = self.data
        document = new
        engineBytes = data
        revision += 1
        if let old {
            undoManager?.registerUndo(withTarget: self) { file in
                file.replace(with: old, actionName: actionName, undoManager: undoManager)
            }
            undoManager?.setActionName(actionName)
        }
    }

    /// PDFKit changed the document in place (a form field filled): save what PDFKit holds.
    func changedInPlace() {
        engineBytes = nil
    }

    /// Run `change` on the PDFKit document in place (highlights, page edits), undoably.
    func change(_ actionName: String, undoManager: UndoManager?, _ change: (PDFDocument) -> Void) {
        let old = data
        change(document)
        engineBytes = nil
        revision += 1
        if let old {
            undoManager?.registerUndo(withTarget: self) { file in
                file.replace(with: old, actionName: actionName, undoManager: undoManager)
            }
            undoManager?.setActionName(actionName)
        }
    }

    /// Unlock an encrypted document with `password`. Returns whether it opened.
    func unlock(_ password: String) -> Bool {
        let ok = document.unlock(withPassword: password)
        if ok {
            // Saved encrypted again with the same password.
            self.password = password
            revision += 1
        }
        return ok
    }

    /// Require `password` to open the document from now on, undoably.
    func protect(with password: String, undoManager: UndoManager?) {
        let old = self.password
        self.password = password
        undoManager?.registerUndo(withTarget: self) { file in
            file.password = old
        }
        undoManager?.setActionName(String(localized: "Protect"))
    }
}
