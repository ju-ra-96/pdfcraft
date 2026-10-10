import Foundation
import PDFKit
import UniformTypeIdentifiers
import Vision

/// Recognize Text with Apple's Vision framework: each page without text is drawn, read, and its
/// words added by PdfCraft's engine as invisible text in their places (`ocr_add_words`), so a
/// scanned page becomes searchable and selectable. The page image is not changed.
enum TextRecognizer {
    /// Pixels on a page's longer side when it is read.
    static let resolution: CGFloat = 2400

    /// The words on `page` (if it has no text yet), as `ocr_add_words` takes them: text and
    /// [x0, y0, x1, y1] in points from the top-left of the displayed page.
    static func words(on page: PDFPage) throws -> [[String: Any]] {
        if let text = page.string, !text.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty { return [] }
        let box = page.bounds(for: .cropBox)
        let turned = page.rotation % 180 != 0
        let size = turned ? CGSize(width: box.height, height: box.width) : box.size
        guard size.width > 0, size.height > 0 else { return [] }
        let k = resolution / max(size.width, size.height)
        let image = page.thumbnail(of: CGSize(width: size.width * k, height: size.height * k), for: .cropBox)
        guard let cg = image.cgImage else { return [] }
        let request = VNRecognizeTextRequest()
        request.recognitionLevel = .accurate
        request.usesLanguageCorrection = true
        try VNImageRequestHandler(cgImage: cg).perform([request])
        var words: [[String: Any]] = []
        for observation in request.results ?? [] {
            guard let candidate = observation.topCandidates(1).first else { continue }
            let text = candidate.string
            text.enumerateSubstrings(in: text.startIndex..<text.endIndex, options: .byWords) { word, range, _, _ in
                guard let word, let box = try? candidate.boundingBox(for: range)?.boundingBox else { return }
                // Vision's boxes are normalised, with the origin at the bottom-left.
                let rect = [box.minX * size.width, (1 - box.maxY) * size.height, box.maxX * size.width, (1 - box.minY) * size.height]
                guard rect[2] > rect[0], rect[3] > rect[1] else { return }
                words.append(["text": word, "rect": rect.map(Double.init)])
            }
        }
        return words
    }

    /// Recognise every page of `pdf` and add the words through the engine; returns the new
    /// document and how many words were found.
    static func recognize(_ pdf: Data) throws -> (Data, Int) {
        guard let document = PDFDocument(data: pdf) else {
            throw PdfCraftEngine.Failure(kind: "failed", message: String(localized: "The document couldn't be read."))
        }
        var pages: [(Int, [[String: Any]])] = []
        for index in 0..<document.pageCount {
            guard let page = document.page(at: index) else { continue }
            let found = try words(on: page)
            if !found.isEmpty { pages.append((index + 1, found)) }
        }
        let count = pages.reduce(0) { $0 + $1.1.count }
        guard count > 0 else { return (pdf, 0) }
        let engine = PdfCraftEngine.shared
        let out = try engine.edit(pdf) { doc in
            for (page, words) in pages {
                try engine.json("ocr_add_words", ["doc": doc, "page": page, "words": words])
            }
        }
        return (out, count)
    }
}

/// Digital IDs (PKCS #12 files) kept in the app's sandbox, for signing documents. Their
/// passwords are not stored: they are asked for each time, as in Acrobat.
enum DigitalIDs {
    static var folder: URL {
        let base = FileManager.default.urls(for: .applicationSupportDirectory, in: .userDomainMask).first ?? FileManager.default.temporaryDirectory
        return base.appendingPathComponent("DigitalIDs", isDirectory: true)
    }

    /// The IDs, by name (the file name without `.p12`).
    static func list() -> [String] {
        let files = (try? FileManager.default.contentsOfDirectory(at: folder, includingPropertiesForKeys: nil)) ?? []
        return files.filter { ["p12", "pfx"].contains($0.pathExtension.lowercased()) }
            .map { $0.deletingPathExtension().lastPathComponent }
            .sorted { $0.localizedStandardCompare($1) == .orderedAscending }
    }

    static func url(_ name: String) -> URL? {
        ["p12", "pfx"].map { folder.appendingPathComponent(name).appendingPathExtension($0) }
            .first { FileManager.default.fileExists(atPath: $0.path) }
    }

    /// Keep a copy of a picked .p12/.pfx file (security-scoped, from the document picker).
    static func importFile(_ source: URL) throws -> String {
        try FileManager.default.createDirectory(at: folder, withIntermediateDirectories: true)
        let scoped = source.startAccessingSecurityScopedResource()
        defer { if scoped { source.stopAccessingSecurityScopedResource() } }
        let name = source.deletingPathExtension().lastPathComponent
        let target = folder.appendingPathComponent(name).appendingPathExtension("p12")
        try? FileManager.default.removeItem(at: target)
        try FileManager.default.copyItem(at: source, to: target)
        return name
    }

    /// Create a self-signed ID through the engine (`sign_id_create`).
    static func create(name: String, email: String, password: String) throws -> String {
        try FileManager.default.createDirectory(at: folder, withIntermediateDirectories: true)
        let engine = PdfCraftEngine.shared
        let file = "\(UUID().uuidString).p12"
        var args: [String: Any] = ["name": name, "password": password, "path": file]
        if !email.isEmpty { args["email"] = email }
        try engine.json("sign_id_create", args)
        let made = engine.workDir.appendingPathComponent(file)
        let safe = name.replacingOccurrences(of: "/", with: "-")
        let target = folder.appendingPathComponent(safe).appendingPathExtension("p12")
        try? FileManager.default.removeItem(at: target)
        try FileManager.default.moveItem(at: made, to: target)
        return safe
    }

    /// Sign `pdf` with the ID `name` (an invisible signature unless the document has an empty
    /// signature field) and return the signed file, byte for byte.
    static func sign(_ pdf: Data, with name: String, password: String, reason: String) throws -> Data {
        guard let id = url(name) else {
            throw PdfCraftEngine.Failure(kind: "failed", message: String(localized: "That digital ID is gone."))
        }
        let engine = PdfCraftEngine.shared
        let copy = engine.workDir.appendingPathComponent("\(UUID().uuidString).p12")
        try FileManager.default.copyItem(at: id, to: copy)
        defer { try? FileManager.default.removeItem(at: copy) }
        let base = UUID().uuidString
        let input = engine.workDir.appendingPathComponent("\(base).pdf")
        let signed = engine.workDir.appendingPathComponent("\(base)-signed.pdf")
        try pdf.write(to: input, options: .atomic)
        defer {
            try? FileManager.default.removeItem(at: input)
            try? FileManager.default.removeItem(at: signed)
        }
        guard let doc = try engine.json("doc_open", ["path": input.lastPathComponent])["doc"] as? Int else {
            throw PdfCraftEngine.Failure(kind: "failed", message: String(localized: "The document couldn't be opened."))
        }
        defer { _ = try? engine.json("doc_close", ["doc": doc, "discard_changes": true]) }
        // Signing saves the signed file itself (as in Acrobat).
        var args: [String: Any] = ["doc": doc, "id": copy.lastPathComponent, "password": password, "out": signed.lastPathComponent]
        if !reason.isEmpty { args["reason"] = reason }
        try engine.json("sign_document", args)
        return try Data(contentsOf: signed)
    }
}

extension UTType {
    /// .p12 / .pfx digital ID files (the system's PKCS #12 type).
    static var digitalID: UTType { UTType("com.rsa.pkcs-12") ?? UTType(filenameExtension: "p12") ?? .data }
}
