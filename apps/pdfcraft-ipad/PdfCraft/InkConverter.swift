import PencilKit
import UIKit

/// Turns PencilKit strokes into PdfCraft annotations.
///
/// - **Pens, markers and the monoline and fountain pens** become Ink annotations. PencilKit already
///   worked out each point's width from the Pencil's pressure, tilt and speed (and the barrel roll,
///   for tools that use it). The stroke is saved as a standard `/InkList` that every viewer draws,
///   with those widths as PdfCraft pressures (`/PCPressure`), so it keeps its shape and stays
///   editable in PdfCraft on any platform.
/// - **Textured inks** (pencil, watercolor, crayon) have grain and blending that Ink can't hold.
///   They are drawn by PencilKit itself and saved as a picture, a custom stamp in the stroke's
///   exact rectangle, so they look as they did on screen.
/// - **Strokes cut by the pixel eraser** keep only their visible parts: a stroke's mask is the
///   region still shown, and the points outside it are dropped, splitting the stroke where the
///   eraser crossed it.
enum InkConverter {
    /// The engine draws a pressure `p` at `MIN + 1.3·√p` times the stroke's width
    /// (`pdfcraft_annot::pressure_width_factor`).
    static let minFactor = 0.35
    static let maxFactor = 1.65
    /// Points kept per stroke at most (a very long stroke is sampled more coarsely).
    static let maxPoints = 4000
    /// Pixels per point of a textured stroke's picture (print quality: 216 dpi).
    static let pictureScale: CGFloat = 3

    /// What saving a stroke takes: an Ink annotation, or a picture placed as a stamp.
    enum Save {
        /// `comment_add` arguments, without `doc`.
        case ink([String: Any])
        /// A PNG of the stroke, its page and where it goes: [x0, y0, x1, y1] from the page's
        /// top-left.
        case picture(page: Int, png: Data, rect: [Double])
    }

    /// How to save `stroke`, drawn on a canvas laid over its page (points, origin at the page's
    /// top-left, as the engine's geometry is). `nil` for a stroke with nothing left to save.
    /// Call on the main thread (textured strokes are drawn by PencilKit).
    static func save(_ stroke: PKStroke, page: Int, author: String) -> Save? {
        if textured(stroke.ink) {
            return picture(stroke).map { .picture(page: page, png: $0.0, rect: $0.1) }
        }
        return inkArguments(for: stroke, page: page, author: author).map { .ink($0) }
    }

    static func textured(_ ink: PKInk) -> Bool {
        if ink.inkType == .pencil { return true }
        if #available(iOS 17.0, *) {
            return ink.inkType == .watercolor || ink.inkType == .crayon
        }
        return false
    }

    /// The stroke drawn by PencilKit, in its light-appearance colours, and its rectangle.
    static func picture(_ stroke: PKStroke) -> (Data, [Double])? {
        let drawing = PKDrawing(strokes: [stroke])
        let bounds = drawing.bounds.integral
        guard bounds.width >= 1, bounds.height >= 1, bounds.width * bounds.height < 4_000_000 else { return nil }
        var image: UIImage?
        UITraitCollection(userInterfaceStyle: .light).performAsCurrent {
            image = drawing.image(from: bounds, scale: pictureScale)
        }
        guard let png = image?.pngData() else { return nil }
        return (png, [bounds.minX, bounds.minY, bounds.maxX, bounds.maxY].map(Double.init))
    }

    /// `comment_add` arguments for an Ink annotation of `stroke` (without `doc`).
    static func inkArguments(for stroke: PKStroke, page: Int, author: String) -> [String: Any]? {
        let path = stroke.path
        guard path.count > 0 else { return nil }
        var step = 1.0
        var samples = Array(path.interpolatedPoints(by: .distance(step)))
        while samples.count > maxPoints {
            step *= 2
            samples = Array(path.interpolatedPoints(by: .distance(step)))
        }
        if samples.isEmpty { samples = Array(path) }

        // Runs of points still visible: the pixel eraser leaves a mask of what's shown, in the
        // stroke's own coordinates.
        var runs: [[(CGPoint, Double)]] = [[]]
        for sample in samples {
            if let mask = stroke.mask, !mask.contains(sample.location) {
                if !(runs.last?.isEmpty ?? true) { runs.append([]) }
                continue
            }
            let p = sample.location.applying(stroke.transform)
            guard p.x.isFinite, p.y.isFinite else { continue }
            runs[runs.count - 1].append((p, Double(max(sample.size.width, 0.1))))
        }
        runs = runs.filter { !$0.isEmpty }
        guard !runs.isEmpty else { return nil }
        // A lone point (a dot, or a sliver left by the eraser) still draws.
        runs = runs.map { run in run.count == 1 ? [run[0], (CGPoint(x: run[0].0.x + 0.01, y: run[0].0.y), run[0].1)] : run }

        let widths = runs.flatMap { $0.map(\.1) }
        guard let widest = widths.max(), let narrowest = widths.min() else { return nil }
        var args: [String: Any] = [
            "page": page, "type": "ink", "author": author,
            "color": hex(stroke.ink.color), "opacity": opacity(stroke.ink),
            "strokes": runs.map { $0.map { [Double($0.0.x), Double($0.0.y)] } },
        ]
        if widest - narrowest < max(0.05 * widest, 0.05) {
            args["width"] = widest
        } else {
            // The widest point draws at the largest factor; the rest in proportion.
            let base = widest / maxFactor
            args["width"] = base
            args["pressure"] = runs.map { run in
                run.map { _, w in
                    let factor = min(max(w / base, minFactor), maxFactor)
                    let p = (factor - minFactor) / 1.3
                    return min(max(p * p, 0), 1)
                }
            }
        }
        return args
    }

    /// `#RRGGBB` of a PencilKit ink colour, as it is in the light appearance (PencilKit shows
    /// dark-mode variants of the same colour on screen).
    static func hex(_ color: UIColor) -> String {
        let light = color.resolvedColor(with: UITraitCollection(userInterfaceStyle: .light))
        var r: CGFloat = 0, g: CGFloat = 0, b: CGFloat = 0, a: CGFloat = 0
        guard light.getRed(&r, green: &g, blue: &b, alpha: &a) else { return "#000000" }
        let c = { (v: CGFloat) in Int((min(max(v, 0), 1) * 255).rounded()) }
        return String(format: "#%02X%02X%02X", c(r), c(g), c(b))
    }

    /// How opaque the saved stroke is: the ink's alpha, and see-through for the marker.
    static func opacity(_ ink: PKInk) -> Double {
        var alpha: CGFloat = 1
        ink.color.getRed(nil, green: nil, blue: nil, alpha: &alpha)
        let tool: Double = ink.inkType == .marker ? 0.5 : 1
        return min(max(Double(alpha) * tool, 0.1), 1)
    }
}
