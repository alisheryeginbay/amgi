import AppKit

/// The menu bar icon: the app icon's five-petal flower with its eye, in one colour.
/// Drawn in code from the same geometry as `Amgi.icon/Assets/petals.svg`.
enum MenuBarIcon {
    /// Point size of the icon; the menu bar's own symbols are about this tall.
    static let size: CGFloat = 16

    /// A template image, so the menu bar tints it for light, dark and highlighted states.
    static let image: NSImage = {
        let image = NSImage(size: NSSize(width: size, height: size), flipped: true) { rect in
            guard let cg = NSGraphicsContext.current?.cgContext else { return false }
            draw(in: cg, rect: rect)
            return true
        }
        image.isTemplate = true
        return image
    }()

    static func draw(in cg: CGContext, rect: CGRect) {
        // The flower reaches about 103 units from its centre (petals.svg, before scaling).
        let scale = rect.width / 2 / 103.5
        cg.translateBy(x: rect.midX, y: rect.midY)
        cg.scaleBy(x: scale, y: scale)
        cg.setFillColor(.black)

        for degrees in stride(from: 0, to: 360, by: 72) {
            cg.addPath(petal(rotated: CGFloat(degrees)))
            cg.fillPath()
        }
        // The middle, with the eye's white ring as a hole (the petals start outside it)
        // and its yellow centre as a dot.
        cg.addPath(disc(52))
        cg.addPath(disc(31))
        cg.fillPath(using: .evenOdd)
        cg.addPath(disc(15))
        cg.fillPath()
    }

    private static func disc(_ r: CGFloat) -> CGPath {
        CGPath(ellipseIn: CGRect(x: -r, y: -r, width: r * 2, height: r * 2), transform: nil)
    }

    /// One petal pointing up, turned `degrees` clockwise about the centre.
    private static func petal(rotated degrees: CGFloat) -> CGPath {
        let turn = CGAffineTransform(rotationAngle: degrees * .pi / 180)
        let p = CGMutablePath()
        p.move(to: CGPoint(x: -26.89, y: -34.83), transform: turn)
        p.addCurve(to: CGPoint(x: -44, y: -72), control1: CGPoint(x: -36.30, y: -45.66), control2: CGPoint(x: -44, y: -58.83), transform: turn)
        p.addCurve(to: CGPoint(x: 0, y: -100), control1: CGPoint(x: -44, y: -96), control2: CGPoint(x: -22, y: -108), transform: turn)
        p.addCurve(to: CGPoint(x: 44, y: -72), control1: CGPoint(x: 22, y: -108), control2: CGPoint(x: 44, y: -96), transform: turn)
        p.addCurve(to: CGPoint(x: 26.89, y: -34.83), control1: CGPoint(x: 44, y: -58.83), control2: CGPoint(x: 36.30, y: -45.66), transform: turn)
        p.closeSubpath()
        return p
    }
}
