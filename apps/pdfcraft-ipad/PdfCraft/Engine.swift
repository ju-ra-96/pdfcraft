import Foundation

/// PdfCraft's Rust engine (`apps/pdfcraft-ipad/src/lib.rs`): every tool of `pdfcraft-cli tools`,
/// one JSON request at a time. Calls are serialised by a lock; run them off the main thread.
final class PdfCraftEngine: @unchecked Sendable {
    static let shared = PdfCraftEngine()

    /// A tool's error, with the engine's message (written for people to read).
    struct Failure: LocalizedError {
        let kind: String
        let message: String
        var errorDescription: String? { message }
    }

    private let handle: OpaquePointer?
    private let lock = NSLock()
    /// The only folder the engine's tools may read or write: a scratch folder in the sandbox.
    let workDir: URL

    private init() {
        workDir = FileManager.default.temporaryDirectory.appendingPathComponent("PdfCraftEngine", isDirectory: true)
        try? FileManager.default.createDirectory(at: workDir, withIntermediateDirectories: true)
        handle = workDir.path.withCString { pdfcraft_engine_new($0) }
    }

    /// Run `tool` with `args`; returns the response's content items (`["json": …]` or `["png": …]`).
    func call(_ tool: String, _ args: [String: Any] = [:]) throws -> [[String: Any]] {
        lock.lock()
        defer { lock.unlock() }
        guard let handle else {
            throw Failure(kind: "internal", message: String(localized: "The PdfCraft engine couldn't start."))
        }
        let request = try JSONSerialization.data(withJSONObject: ["tool": tool, "args": args])
        let buffer = request.withUnsafeBytes { raw in
            pdfcraft_call(handle, raw.bindMemory(to: UInt8.self).baseAddress, raw.count)
        }
        defer { pdfcraft_buffer_free(buffer) }
        guard let ptr = buffer.ptr, buffer.len > 0 else {
            throw Failure(kind: "internal", message: String(localized: "The PdfCraft engine gave no answer."))
        }
        let response = try JSONSerialization.jsonObject(with: Data(bytes: ptr, count: buffer.len)) as? [String: Any] ?? [:]
        if let error = response["error"] as? [String: Any] {
            throw Failure(kind: error["kind"] as? String ?? "failed", message: error["message"] as? String ?? "")
        }
        return response["ok"] as? [[String: Any]] ?? []
    }

    /// The JSON result of `tool` (its first content item).
    @discardableResult
    func json(_ tool: String, _ args: [String: Any] = [:]) throws -> [String: Any] {
        try call(tool, args).first?["json"] as? [String: Any] ?? [:]
    }

    /// Open `pdf` in the engine, let `body` run tools on it (it gets the document id), and return
    /// the document saved afterwards. Nothing outside the engine's scratch folder is touched.
    func edit(_ pdf: Data, _ body: (Int) throws -> Void) throws -> Data {
        let name = UUID().uuidString
        let input = workDir.appendingPathComponent("\(name).pdf")
        let output = workDir.appendingPathComponent("\(name)-out.pdf")
        try pdf.write(to: input, options: .atomic)
        defer {
            try? FileManager.default.removeItem(at: input)
            try? FileManager.default.removeItem(at: output)
        }
        guard let doc = try json("doc_open", ["path": input.lastPathComponent])["doc"] as? Int else {
            throw Failure(kind: "failed", message: String(localized: "The document couldn't be opened."))
        }
        defer { _ = try? json("doc_close", ["doc": doc, "discard_changes": true]) }
        try body(doc)
        // An incremental update: the original bytes stay as they were, so signatures stay valid.
        try json("doc_save", ["doc": doc, "path": output.lastPathComponent, "full": false])
        return try Data(contentsOf: output)
    }

    /// Run a tool that writes a new file (`path`) from the document, and return that file.
    func export(_ pdf: Data, _ tool: String, _ args: [String: Any] = [:]) throws -> Data {
        let name = UUID().uuidString
        let input = workDir.appendingPathComponent("\(name).pdf")
        let output = workDir.appendingPathComponent("\(name)-export.pdf")
        try pdf.write(to: input, options: .atomic)
        defer {
            try? FileManager.default.removeItem(at: input)
            try? FileManager.default.removeItem(at: output)
        }
        guard let doc = try json("doc_open", ["path": input.lastPathComponent])["doc"] as? Int else {
            throw Failure(kind: "failed", message: String(localized: "The document couldn't be opened."))
        }
        defer { _ = try? json("doc_close", ["doc": doc, "discard_changes": true]) }
        var args = args
        args["doc"] = doc
        args["path"] = output.lastPathComponent
        try json(tool, args)
        return try Data(contentsOf: output)
    }
}
