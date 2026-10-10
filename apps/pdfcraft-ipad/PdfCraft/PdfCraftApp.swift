import SwiftUI

/// PdfCraft for iPad: Apple's document browser (Files, iCloud Drive and every other provider,
/// recents, tags), one window per document, Stage Manager and Split View.
@main
struct PdfCraftApp: App {
    init() {
        // UI tests open a known document through the browser (PdfCraftUITests).
        if ProcessInfo.processInfo.arguments.contains("-PdfCraftUITestSample"),
           let documents = FileManager.default.urls(for: .documentDirectory, in: .userDomainMask).first {
            try? PDFFile.sampleData().write(to: documents.appendingPathComponent("Sample.pdf"), options: .atomic)
        }
    }

    var body: some Scene {
        DocumentGroup(newDocument: { PDFFile() }) { configuration in
            DocumentView(file: configuration.document)
        }
    }
}
