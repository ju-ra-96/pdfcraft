import PDFKit
import PencilKit
import SwiftUI

/// The page view: PDFKit's `PDFView` with a PencilKit canvas over every page for Markup.
///
/// PencilKit does the Apple Pencil: Apple's tool picker, pressure, tilt and barrel roll, hover
/// preview, double-tap and squeeze (as set in Settings ▸ Apple Pencil), palm rejection and
/// haptics, and "Only Draw with Apple Pencil" from Settings (`drawingPolicy = .default`).
/// Strokes stay PencilKit strokes (with PencilKit's own undo and eraser) until Markup ends or the
/// document saves; then they become PdfCraft Ink annotations ([`InkConverter`]).
@MainActor
final class PDFController: NSObject, ObservableObject {
    let pdfView = MarkupPDFView()
    let toolPicker = PKToolPicker()

    @Published private(set) var markup = false
    @Published private(set) var hasSelection = false
    @Published private(set) var pageIndex = 0
    @Published private(set) var pageCount = 0
    /// Strokes drawn and not yet saved into the document.
    @Published private(set) var pendingStrokes = 0

    /// Run an engine edit of the document's bytes (set by the document view, which owns undo).
    var engineEdit: ((String, @escaping (Data) throws -> Data) -> Void)?
    var author = "PdfCraft"

    private var canvases: [ObjectIdentifier: (page: PDFPage, canvas: PKCanvasView)] = [:]

    override init() {
        super.init()
        pdfView.autoScales = true
        pdfView.displayMode = .singlePageContinuous
        pdfView.displayDirection = .vertical
        pdfView.usePageViewController(false)
        pdfView.pageShadowsEnabled = true
        pdfView.isFindInteractionEnabled = true
        pdfView.backgroundColor = .secondarySystemBackground
        pdfView.pageOverlayViewProvider = self
        NotificationCenter.default.addObserver(self, selector: #selector(selectionChanged), name: .PDFViewSelectionChanged, object: pdfView)
        NotificationCenter.default.addObserver(self, selector: #selector(pageChanged), name: .PDFViewPageChanged, object: pdfView)
    }

    /// Show `document` (a new one after an engine edit): the overlays start again empty.
    func show(_ document: PDFDocument) {
        pageCount = document.pageCount
        guard pdfView.document !== document else {
            // Changed in place (pages inserted, removed or turned).
            pdfView.layoutDocumentView()
            return
        }
        let page = pdfView.currentPage.flatMap { pdfView.document?.index(for: $0) }
        for (_, entry) in canvases { toolPicker.removeObserver(entry.canvas) }
        canvases.removeAll()
        pendingStrokes = 0
        pdfView.document = document
        if let page, let target = document.page(at: min(page, max(document.pageCount - 1, 0))) {
            pdfView.go(to: target)
        }
        if markup { activateCanvas() }
    }

    // MARK: Markup

    func setMarkup(_ on: Bool) {
        guard on != markup else { return }
        if !on { commitStrokes() }
        markup = on
        pdfView.isInMarkupMode = on
        for (_, entry) in canvases { entry.canvas.isUserInteractionEnabled = on }
        if on {
            pdfView.clearSelection()
            activateCanvas()
        } else {
            for (_, entry) in canvases {
                toolPicker.setVisible(false, forFirstResponder: entry.canvas)
                entry.canvas.resignFirstResponder()
            }
        }
    }

    /// Give the tool picker to the canvas of the page in view.
    private func activateCanvas() {
        guard let page = pdfView.currentPage, let canvas = canvas(for: page) else { return }
        toolPicker.setVisible(true, forFirstResponder: canvas)
        canvas.becomeFirstResponder()
    }

    private func canvas(for page: PDFPage) -> PKCanvasView? {
        if let entry = canvases[ObjectIdentifier(page)] { return entry.canvas }
        let canvas = PKCanvasView()
        canvas.backgroundColor = .clear
        canvas.isOpaque = false
        canvas.drawingPolicy = .default
        canvas.isScrollEnabled = false
        canvas.delegate = self
        canvas.isUserInteractionEnabled = markup
        toolPicker.addObserver(canvas)
        canvases[ObjectIdentifier(page)] = (page, canvas)
        return canvas
    }

    /// Save every pending stroke into the document as Ink, in one undo step.
    func commitStrokes() {
        guard let document = pdfView.document else { return }
        var calls: [(PKStroke, Int)] = []
        for (_, entry) in canvases {
            let index = document.index(for: entry.page)
            guard index != NSNotFound else { continue }
            for stroke in entry.canvas.drawing.strokes { calls.append((stroke, index + 1)) }
        }
        guard !calls.isEmpty else { return }
        let author = self.author
        let strokes = calls
        for (_, entry) in canvases { entry.canvas.drawing = PKDrawing() }
        pendingStrokes = 0
        engineEdit?(String(localized: "Draw")) { data in
            try PdfCraftEngine.shared.edit(data) { doc in
                for (stroke, page) in strokes {
                    if let args = InkConverter.arguments(for: stroke, page: page, doc: doc, author: author) {
                        try PdfCraftEngine.shared.json("comment_add", args)
                    }
                }
            }
        }
    }

    // MARK: Pages

    var currentPage: PDFPage? { pdfView.currentPage }

    func go(to index: Int) {
        if let page = pdfView.document?.page(at: index) { pdfView.go(to: page) }
    }

    func find() {
        pdfView.findInteraction.presentFindNavigator(showingReplace: false)
    }

    // MARK: Notifications

    @objc private func selectionChanged() {
        hasSelection = !(pdfView.currentSelection?.string ?? "").isEmpty
    }

    @objc private func pageChanged() {
        if let page = pdfView.currentPage, let index = pdfView.document?.index(for: page), index != NSNotFound {
            pageIndex = index
        }
    }
}

extension PDFController: PDFPageOverlayViewProvider {
    nonisolated func pdfView(_ view: PDFView, overlayViewFor page: PDFPage) -> UIView? {
        MainActor.assumeIsolated { canvas(for: page) }
    }
}

extension PDFController: PKCanvasViewDelegate {
    nonisolated func canvasViewDidBeginUsingTool(_ canvasView: PKCanvasView) {
        MainActor.assumeIsolated {
            // The tool picker follows the page being drawn on.
            if !canvasView.isFirstResponder {
                toolPicker.setVisible(true, forFirstResponder: canvasView)
                canvasView.becomeFirstResponder()
            }
        }
    }

    nonisolated func canvasViewDrawingDidChange(_ canvasView: PKCanvasView) {
        MainActor.assumeIsolated {
            pendingStrokes = canvases.values.reduce(0) { $0 + $1.canvas.drawing.strokes.count }
        }
    }
}

/// `PDFView` with text-markup commands in the selection's edit menu.
final class MarkupPDFView: PDFView {
    /// Mark up the selection (set by the document view).
    var onMarkup: ((Markup) -> Void)?

    enum Markup {
        case highlight, underline, strikeOut, redact
    }

    override func buildMenu(with builder: any UIMenuBuilder) {
        super.buildMenu(with: builder)
        guard !(currentSelection?.string ?? "").isEmpty, onMarkup != nil else { return }
        let actions = [
            UIAction(title: String(localized: "Highlight"), image: UIImage(systemName: "highlighter")) { [weak self] _ in self?.onMarkup?(.highlight) },
            UIAction(title: String(localized: "Underline"), image: UIImage(systemName: "underline")) { [weak self] _ in self?.onMarkup?(.underline) },
            UIAction(title: String(localized: "Strike Through"), image: UIImage(systemName: "strikethrough")) { [weak self] _ in self?.onMarkup?(.strikeOut) },
            UIAction(title: String(localized: "Redact"), image: UIImage(systemName: "rectangle.fill"), attributes: .destructive) { [weak self] _ in self?.onMarkup?(.redact) },
        ]
        builder.insertChild(UIMenu(title: "", options: .displayInline, children: actions), atStartOfMenu: .root)
    }
}

/// The page view in SwiftUI.
struct PDFKitView: UIViewRepresentable {
    let controller: PDFController

    func makeUIView(context: Context) -> MarkupPDFView { controller.pdfView }
    func updateUIView(_ view: MarkupPDFView, context: Context) {}
}

/// Page thumbnails beside the page view: tap to go to a page; a long press (or right-click)
/// offers the page commands.
struct ThumbnailsView: View {
    @ObservedObject var controller: PDFController
    let document: PDFDocument
    let revision: Int
    var onRotate: (Int, Int) -> Void
    var onInsertAfter: (Int) -> Void
    var onDelete: (Int) -> Void

    var body: some View {
        ScrollViewReader { proxy in
            ScrollView {
                LazyVStack(spacing: 14) {
                    ForEach(0..<document.pageCount, id: \.self) { index in
                        ThumbnailCell(document: document, index: index, revision: revision, selected: index == controller.pageIndex)
                            .id(index)
                            .onTapGesture { controller.go(to: index) }
                            .contextMenu {
                                Button { onRotate(index, -90) } label: { Label(String(localized: "Rotate Left"), systemImage: "rotate.left") }
                                Button { onRotate(index, 90) } label: { Label(String(localized: "Rotate Right"), systemImage: "rotate.right") }
                                Button { onInsertAfter(index) } label: { Label(String(localized: "Insert Blank Page After"), systemImage: "doc.badge.plus") }
                                Divider()
                                Button(role: .destructive) { onDelete(index) } label: { Label(String(localized: "Delete Page"), systemImage: "trash") }
                                    .disabled(document.pageCount <= 1)
                            }
                    }
                }
                .padding(.vertical, 16)
                .padding(.horizontal, 12)
            }
            .onChange(of: controller.pageIndex) { _, index in
                withAnimation { proxy.scrollTo(index, anchor: .center) }
            }
        }
        .background(Color(uiColor: .secondarySystemBackground))
    }
}

/// One page's thumbnail and number.
private struct ThumbnailCell: View {
    let document: PDFDocument
    let index: Int
    let revision: Int
    let selected: Bool
    @State private var image: UIImage?

    var body: some View {
        VStack(spacing: 6) {
            Group {
                if let image {
                    Image(uiImage: image).resizable().scaledToFit()
                } else {
                    Rectangle().fill(Color(uiColor: .systemBackground)).aspectRatio(0.77, contentMode: .fit)
                }
            }
            .frame(maxWidth: 92, maxHeight: 124)
            .background(Color.white)
            .clipShape(RoundedRectangle(cornerRadius: 3, style: .continuous))
            .shadow(color: .black.opacity(0.15), radius: 2, y: 1)
            .padding(3)
            .overlay(
                RoundedRectangle(cornerRadius: 6, style: .continuous)
                    .stroke(selected ? Color.accentColor : .clear, lineWidth: 3)
            )
            Text("\(index + 1)")
                .font(.caption)
                .foregroundStyle(selected ? Color.accentColor : .secondary)
        }
        .accessibilityElement(children: .combine)
        .accessibilityLabel(String(localized: "Page \(index + 1)"))
        .accessibilityAddTraits(selected ? [.isButton, .isSelected] : .isButton)
        .task(id: "\(revision)-\(index)") {
            guard let page = document.page(at: index) else { return }
            image = page.thumbnail(of: CGSize(width: 184, height: 248), for: .cropBox)
        }
    }
}
