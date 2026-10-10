import PDFKit
import PencilKit
import SwiftUI
import UniformTypeIdentifiers

/// One open document: page thumbnails, the page view, and the toolbar.
struct DocumentView: View {
    @ObservedObject var file: PDFFile
    @StateObject private var controller = PDFController()
    @Environment(\.undoManager) private var undoManager
    @Environment(\.scenePhase) private var scenePhase
    @AppStorage("authorName") private var author = ""

    @State private var showThumbnails = true
    @State private var busy: String?
    @State private var failure: String?
    @State private var askPassword = false
    @State private var askProtect = false
    @State private var askRedact = false
    @State private var password = ""
    @State private var redactText = ""
    @State private var askWatermark = false
    @State private var watermarkText = ""
    @State private var showSign = false
    @State private var pickPages = false
    @State private var notice: String?

    var body: some View {
        HStack(spacing: 0) {
            if showThumbnails {
                ThumbnailsView(
                    controller: controller, document: file.document, revision: file.revision,
                    onRotate: { index, degrees in rotate(degrees, page: index) },
                    onInsertAfter: { index in insertBlankPage(after: index) },
                    onDelete: { index in deletePage(index) }
                )
                .frame(width: 124)
                    .transition(.move(edge: .leading))
                Divider()
            }
            PDFKitView(controller: controller)
        }
        .ignoresSafeArea(edges: .bottom)
        .toolbarRole(.editor)
        .toolbar { toolbar }
        .onAppear(perform: setUp)
        .onChange(of: file.revision) { controller.show(file.document) }
        .onChange(of: scenePhase) { _, phase in
            // Strokes become part of the document before it is saved in the background.
            if phase != .active { controller.commitStrokes() }
        }
        .onReceive(NotificationCenter.default.publisher(for: UITextField.textDidEndEditingNotification)) { note in
            // PDFKit fills form fields itself; record the change so the document saves.
            if let field = note.object as? UIView, field.isDescendant(of: controller.pdfView) {
                markEdited(String(localized: "Fill Form"))
            }
        }
        .overlay { if let busy { BusyView(title: busy) } }
        .overlay(alignment: .bottomLeading) {
            // UI tests read the document's state here (PdfCraftUITests).
            if ProcessInfo.processInfo.arguments.contains("-PdfCraftUITest") {
                Text(testStatus).font(.caption2).padding(4).accessibilityIdentifier("status")
            }
        }
        .alert(String(localized: "That didn't work"), isPresented: .constant(failure != nil), presenting: failure) { _ in
            Button("OK") { failure = nil }
        } message: { Text($0) }
        .alert(String(localized: "This document is protected"), isPresented: $askPassword) {
            SecureField(String(localized: "Password"), text: $password)
            Button(String(localized: "Open")) { unlock() }
            Button(String(localized: "Cancel"), role: .cancel) {}
        } message: { Text("Enter the password to open it.") }
        .alert(String(localized: "Protect with a Password"), isPresented: $askProtect) {
            SecureField(String(localized: "Password"), text: $password)
            Button(String(localized: "Protect")) { protect() }
            Button(String(localized: "Cancel"), role: .cancel) {}
        } message: { Text("Anyone opening this document will need the password.") }
        .alert(String(localized: "Redact Text"), isPresented: $askRedact) {
            TextField(String(localized: "Word or phrase"), text: $redactText)
            Button(String(localized: "Redact"), role: .destructive) { redactEverywhere() }
            Button(String(localized: "Cancel"), role: .cancel) {}
        } message: { Text("Every match is removed from the document for good, and a black box is drawn in its place.") }
        .alert(String(localized: "Add Watermark"), isPresented: $askWatermark) {
            TextField(String(localized: "Text"), text: $watermarkText)
            Button(String(localized: "Add")) { addWatermark() }
            Button(String(localized: "Cancel"), role: .cancel) {}
        } message: { Text("The text goes across every page, semi-transparent.") }
        .alert(String(localized: "PdfCraft"), isPresented: .constant(notice != nil), presenting: notice) { _ in
            Button("OK") { notice = nil }
        } message: { Text($0) }
        .sheet(isPresented: $showSign) {
            SignSheet { name, password, reason in sign(with: name, password: password, reason: reason) }
        }
        .fileImporter(isPresented: $pickPages, allowedContentTypes: [.pdf]) { result in
            if case .success(let url) = result { insertPages(from: url) }
        }
    }

    /// What UI tests check: comments in the document, and strokes not yet saved into it.
    private var testStatus: String {
        let document = file.document
        let annotations = (0..<document.pageCount).compactMap { document.page(at: $0) }.reduce(0) { $0 + $1.annotations.count }
        let words = document.page(at: 0)?.string?.components(separatedBy: "brown").count ?? 1
        let pages = (0..<document.pageCount).compactMap { document.page(at: $0) }
        let signatures = pages.reduce(0) { n, page in n + page.annotations.filter { $0.widgetFieldType == .signature }.count }
        let stamps = pages.reduce(0) { n, page in n + page.annotations.filter { $0.type == "Stamp" }.count }
        let lastPageText = pages.last?.string?.count ?? 0
        return "annotations \(annotations) pending \(controller.pendingStrokes) revision \(file.revision) brown \(words - 1) signatures \(signatures) stamps \(stamps) pages \(document.pageCount) lasttext \(lastPageText)"
    }

    // MARK: Toolbar

    @ToolbarContentBuilder
    private var toolbar: some ToolbarContent {
        ToolbarItem(placement: .topBarLeading) {
            Button {
                withAnimation { showThumbnails.toggle() }
            } label: {
                Label(String(localized: "Pages"), systemImage: "sidebar.left")
            }
            .accessibilityIdentifier("thumbnails")
        }
        if controller.hasSelection && !controller.markup {
            ToolbarItemGroup(placement: .secondaryAction) {
                Button { markup(.highlight) } label: { Label(String(localized: "Highlight"), systemImage: "highlighter") }
                Button { markup(.underline) } label: { Label(String(localized: "Underline"), systemImage: "underline") }
                Button { markup(.strikeOut) } label: { Label(String(localized: "Strike Through"), systemImage: "strikethrough") }
                Button(role: .destructive) { markup(.redact) } label: { Label(String(localized: "Redact"), systemImage: "rectangle.fill") }
            }
        }
        ToolbarItemGroup(placement: .topBarTrailing) {
            Button { controller.find() } label: { Label(String(localized: "Find"), systemImage: "magnifyingglass") }
                .keyboardShortcut("f", modifiers: .command)
                .accessibilityIdentifier("find")
            Button {
                controller.setMarkup(!controller.markup)
            } label: {
                if controller.markup {
                    Text(String(localized: "Done")).bold()
                } else {
                    Label(String(localized: "Markup"), systemImage: "pencil.tip.crop.circle")
                }
            }
            .keyboardShortcut("e", modifiers: [.command, .shift])
            .accessibilityIdentifier("markup")
            pagesMenu.accessibilityIdentifier("pages")
            toolsMenu.accessibilityIdentifier("tools")
            ShareLink(item: PDFExport(file: file), preview: SharePreview(String(localized: "PDF document"))) {
                Label(String(localized: "Share"), systemImage: "square.and.arrow.up")
            }
        }
    }

    private var pagesMenu: some View {
        Menu {
            Button { rotate(-90, page: controller.pageIndex) } label: { Label(String(localized: "Rotate Left"), systemImage: "rotate.left") }
            Button { rotate(90, page: controller.pageIndex) } label: { Label(String(localized: "Rotate Right"), systemImage: "rotate.right") }
            Button { insertBlankPage(after: controller.pageIndex) } label: { Label(String(localized: "Insert Blank Page"), systemImage: "doc.badge.plus") }
            Divider()
            Button(role: .destructive) { deletePage(controller.pageIndex) } label: { Label(String(localized: "Delete Page"), systemImage: "trash") }
                .disabled(controller.pageCount <= 1)
        } label: {
            Label(String(localized: "Page"), systemImage: "doc.on.doc")
        }
    }

    private var toolsMenu: some View {
        Menu {
            Section {
                Button { recognizeText() } label: { Label(String(localized: "Recognize Text"), systemImage: "text.viewfinder") }
                Button { showSign = true } label: { Label(String(localized: "Sign with a Digital ID…"), systemImage: "signature") }
                Button { askRedact = true } label: { Label(String(localized: "Redact Text…"), systemImage: "eye.slash") }
                Button { password = ""; askProtect = true } label: { Label(String(localized: "Protect with a Password…"), systemImage: "lock") }
            }
            Section {
                Button { pickPages = true } label: { Label(String(localized: "Insert Pages from a File…"), systemImage: "doc.on.doc") }
                Button { watermarkText = ""; askWatermark = true } label: { Label(String(localized: "Add Watermark…"), systemImage: "drop") }
                Button { reduceSize() } label: { Label(String(localized: "Reduce File Size"), systemImage: "arrow.down.right.and.arrow.up.left") }
                Button { flatten() } label: { Label(String(localized: "Flatten Comments and Fields"), systemImage: "square.3.layers.3d.down.right") }
            }
        } label: {
            Label(String(localized: "Tools"), systemImage: "wand.and.stars")
        }
    }

    // MARK: Actions

    private func setUp() {
        controller.author = author.isEmpty ? UIDevice.current.name : author
        controller.engineEdit = { name, work in runEngine(name, work) }
        controller.pdfView.onMarkup = { kind in markup(kind) }
        controller.show(file.document)
        if file.document.isLocked { askPassword = true }
    }

    private func unlock() {
        if !file.unlock(password) {
            failure = String(localized: "That password isn't right.")
        }
        password = ""
    }

    /// Run `work` on the document's bytes in PdfCraft's engine, off the main thread; the result
    /// replaces the document in one undo step.
    private func runEngine(_ name: String, _ work: @escaping (Data) throws -> Data, done: (() -> Void)? = nil) {
        guard let data = file.data else { return }
        busy = name
        let undo = undoManager
        Task.detached(priority: .userInitiated) {
            let result = Result { try work(data) }
            await MainActor.run {
                busy = nil
                switch result {
                case .success(let new):
                    file.replace(with: new, actionName: name, undoManager: undo)
                    done?()
                case .failure(let error): failure = error.localizedDescription
                }
            }
        }
    }

    private func markEdited(_ name: String) {
        file.changedInPlace()
        undoManager?.registerUndo(withTarget: file) { _ in }
        undoManager?.setActionName(name)
    }

    private func markup(_ kind: MarkupPDFView.Markup) {
        guard let selection = controller.pdfView.currentSelection else { return }
        if kind == .redact {
            redact(selection)
            return
        }
        let (type, color, name): (PDFAnnotationSubtype, UIColor, String) = switch kind {
        case .highlight: (.highlight, UIColor.systemYellow.withAlphaComponent(0.5), String(localized: "Highlight"))
        case .underline: (.underline, .systemBlue, String(localized: "Underline"))
        default: (.strikeOut, .systemRed, String(localized: "Strike Through"))
        }
        let who = controller.author
        file.change(name, undoManager: undoManager) { _ in
            for line in selection.selectionsByLine() {
                for page in line.pages {
                    let annotation = PDFAnnotation(bounds: line.bounds(for: page), forType: type, withProperties: nil)
                    annotation.color = color
                    annotation.userName = who
                    annotation.modificationDate = Date()
                    page.addAnnotation(annotation)
                }
            }
        }
        controller.pdfView.clearSelection()
    }

    /// Redact the selected text (PdfCraft's engine removes it from the page for good).
    private func redact(_ selection: PDFSelection) {
        guard let document = controller.pdfView.document else { return }
        var marks: [(Int, [Double])] = []
        for line in selection.selectionsByLine() {
            for page in line.pages {
                let index = document.index(for: page)
                guard index != NSNotFound else { continue }
                // The engine's geometry starts at the top-left of the displayed page.
                let box = page.bounds(for: .cropBox)
                let r = line.bounds(for: page)
                marks.append((index + 1, [r.minX - box.minX, box.maxY - r.maxY, r.maxX - box.minX, box.maxY - r.minY].map(Double.init)))
            }
        }
        controller.pdfView.clearSelection()
        guard !marks.isEmpty else { return }
        runEngine(String(localized: "Redact")) { data in
            try PdfCraftEngine.shared.edit(data) { doc in
                for (page, rect) in marks {
                    try PdfCraftEngine.shared.json("redact_mark", ["doc": doc, "page": page, "rect": rect])
                }
                try PdfCraftEngine.shared.json("redact_apply", ["doc": doc])
            }
        }
    }

    private func redactEverywhere() {
        let text = redactText.trimmingCharacters(in: .whitespacesAndNewlines)
        redactText = ""
        guard !text.isEmpty else { return }
        runEngine(String(localized: "Redact")) { data in
            try PdfCraftEngine.shared.edit(data) { doc in
                try PdfCraftEngine.shared.json("redact_mark", ["doc": doc, "find": text])
                try PdfCraftEngine.shared.json("redact_apply", ["doc": doc])
            }
        }
    }

    private func protect() {
        let secret = password
        password = ""
        guard !secret.isEmpty else { return }
        // Apple's encryption, applied each time the document is written (PDFFile.snapshot).
        file.protect(with: secret, undoManager: undoManager)
    }

    /// Read pages without text with Apple's Vision, and add the words as searchable text.
    private func recognizeText() {
        controller.commitStrokes()
        var found = 0
        runEngine(String(localized: "Recognize Text")) { data in
            let (out, words) = try TextRecognizer.recognize(data)
            found = words
            guard words > 0 else {
                throw PdfCraftEngine.Failure(kind: "failed", message: String(localized: "No text was found to recognize: the pages already have text, or none could be read."))
            }
            return out
        } done: {
            notice = String(localized: "Recognized \(found) words. The pages are now searchable.")
        }
    }

    private func sign(with name: String, password: String, reason: String) {
        controller.commitStrokes()
        runEngine(String(localized: "Sign")) { data in
            try DigitalIDs.sign(data, with: name, password: password, reason: reason)
        }
    }

    private func addWatermark() {
        let text = watermarkText.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !text.isEmpty else { return }
        runEngine(String(localized: "Add Watermark")) { data in
            try PdfCraftEngine.shared.edit(data) { doc in
                try PdfCraftEngine.shared.json("doc_watermark", ["doc": doc, "text": text])
            }
        }
    }

    /// Insert every page of a picked PDF after the current page.
    private func insertPages(from url: URL) {
        let engine = PdfCraftEngine.shared
        let copy = engine.workDir.appendingPathComponent("\(UUID().uuidString).pdf")
        let scoped = url.startAccessingSecurityScopedResource()
        defer { if scoped { url.stopAccessingSecurityScopedResource() } }
        do {
            try FileManager.default.copyItem(at: url, to: copy)
        } catch {
            failure = error.localizedDescription
            return
        }
        let at = controller.pageIndex + 2
        runEngine(String(localized: "Insert Pages")) { data in
            defer { try? FileManager.default.removeItem(at: copy) }
            return try engine.edit(data) { doc in
                try engine.json("page_insert_file", ["doc": doc, "path": copy.lastPathComponent, "at": at])
            }
        }
    }

    private func reduceSize() {
        runEngine(String(localized: "Reduce File Size")) { data in
            try PdfCraftEngine.shared.export(data, "doc_reduce")
        }
    }

    private func flatten() {
        controller.commitStrokes()
        runEngine(String(localized: "Flatten")) { data in
            try PdfCraftEngine.shared.edit(data) { doc in
                try PdfCraftEngine.shared.json("doc_flatten", ["doc": doc])
            }
        }
    }

    private func rotate(_ degrees: Int, page index: Int) {
        guard let page = file.document.page(at: index) else { return }
        file.change(String(localized: "Rotate Page"), undoManager: undoManager) { _ in
            page.rotation = (page.rotation + degrees + 360) % 360
        }
    }

    private func insertBlankPage(after index: Int) {
        let size = file.document.page(at: index)?.bounds(for: .mediaBox).size ?? CGSize(width: 612, height: 792)
        let blank = UIGraphicsPDFRenderer(bounds: CGRect(origin: .zero, size: size)).pdfData { $0.beginPage() }
        guard let page = PDFDocument(data: blank)?.page(at: 0) else { return }
        file.change(String(localized: "Insert Page"), undoManager: undoManager) { document in
            document.insert(page, at: min(index + 1, document.pageCount))
        }
        controller.go(to: index + 1)
    }

    private func deletePage(_ index: Int) {
        file.change(String(localized: "Delete Page"), undoManager: undoManager) { document in
            if document.pageCount > 1 { document.removePage(at: index) }
        }
    }
}

/// A spinner over the page while the engine works.
private struct BusyView: View {
    let title: String

    var body: some View {
        ProgressView(title)
            .padding(24)
            .background(.regularMaterial, in: RoundedRectangle(cornerRadius: 16, style: .continuous))
    }
}

/// The document for the share sheet.
struct PDFExport: Transferable {
    let file: PDFFile

    static var transferRepresentation: some TransferRepresentation {
        DataRepresentation(exportedContentType: .pdf) { export in
            try await MainActor.run { try export.file.snapshot(contentType: .pdf) }
        }
    }
}
