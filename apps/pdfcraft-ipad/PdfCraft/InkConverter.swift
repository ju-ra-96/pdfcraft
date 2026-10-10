import PencilKit
import UIKit

/// Turns PencilKit strokes into PdfCraft Ink annotations (`comment_add` arguments).
///
/// PencilKit already worked out each point's width from the Pencil's pressure, tilt and speed
/// (and the barrel roll, for tools that use it). The stroke is saved as a standard `/InkList`
/// that every viewer draws, with those widths as PdfCraft pressures (`/PCPressure`), so it keeps
/// its shape and stays editable in PdfCraft on any platform.
enum InkConverter {
    /// The engine draws a pressure `p` at `MIN + 1.3·√p` times the stroke's width
    /// (`pdfcraft_annot::pressure_width_factor`).
    static let minFactor = 0.35
    static let maxFactor = 1.65
    /// Points kept per stroke at most (a very long stroke is sampled more coarsely).
    static let maxPoints = 4000

    /// `comment_add` arguments for `stroke`, drawn on a canvas laid over its page (points, origin
    /// at the page's top-left, as the engine's geometry is). `nil` for an empty stroke.
    static func arguments(for stroke: PKStroke, page: Int, doc: Int, author: String) -> [String: Any]? {
        let path = stroke.path
        guard path.count > 0 else { return nil }
        var step = 1.0
        var samples = Array(path.interpolatedPoints(by: .distance(step)))
        while samples.count > maxPoints {
            step *= 2
            samples = Array(path.interpolatedPoints(by: .distance(step)))
        }
        if samples.isEmpty { samples = Array(path) }
        var points: [[Double]] = []
        var widths: [Double] = []
        for sample in samples {
            let p = sample.location.applying(stroke.transform)
            guard p.x.isFinite, p.y.isFinite else { continue }
            points.append([Double(p.x), Double(p.y)])
            widths.append(Double(max(sample.size.width, 0.1)))
        }
        guard !points.isEmpty, let widest = widths.max(), let narrowest = widths.min() else { return nil }
        if points.count == 1 { points.append([points[0][0] + 0.01, points[0][1]]); widths.append(widths[0]) }

        var args: [String: Any] = [
            "doc": doc, "page": page, "type": "ink", "author": author,
            "color": hex(stroke.ink.color), "opacity": opacity(stroke.ink),
            "strokes": [points],
        ]
        if widest - narrowest < max(0.05 * widest, 0.05) {
            args["width"] = widest
        } else {
            // The widest point draws at the largest factor; the rest in proportion.
            let base = widest / maxFactor
            args["width"] = base
            args["pressure"] = [widths.map { w in
                let factor = min(max(w / base, minFactor), maxFactor)
                let p = (factor - minFactor) / 1.3
                return min(max(p * p, 0), 1)
            }]
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

    /// How opaque the saved stroke is: the ink's alpha, and see-through for highlighting tools.
    static func opacity(_ ink: PKInk) -> Double {
        var alpha: CGFloat = 1
        ink.color.getRed(nil, green: nil, blue: nil, alpha: &alpha)
        let tool: Double
        switch ink.inkType {
        case .marker: tool = 0.5
        case .pencil: tool = 0.85
        default:
            if #available(iOS 17.0, *), ink.inkType == .watercolor { tool = 0.6 }
            else if #available(iOS 17.0, *), ink.inkType == .crayon { tool = 0.9 }
            else { tool = 1 }
        }
        return min(max(Double(alpha) * tool, 0.1), 1)
    }
}
