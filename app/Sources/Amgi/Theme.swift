import AppKit
import SwiftUI

/// Plain neutral grays, a step softer than the system's: off-white and off-black rather
/// than pure white and black, with the Mac's own accent color. The web panes use the
/// same palette, in `web/src/app.css`.
enum Theme {
    /// The window's background, behind the cards.
    static let panel = color(light: 0xFCFCFC, dark: 0x212121)
    static let text = color(light: 0x2B2B2B, dark: 0xD8D8D8)
    static let secondaryText = color(light: 0x7A7A7A, dark: 0x8C8C8C)
    static let tertiaryText = color(light: 0xB5B5B5, dark: 0x5C5C5C)
    /// Under a header button while the pointer is over it.
    static let fill = color(light: 0xEBEBEB, dark: 0x2B2B2B)

    private static func color(light: UInt32, dark: UInt32) -> NSColor {
        NSColor(name: nil) { appearance in
            let hex = appearance.bestMatch(from: [.aqua, .darkAqua]) == .darkAqua ? dark : light
            return NSColor(
                srgbRed: CGFloat(hex >> 16 & 0xff) / 255,
                green: CGFloat(hex >> 8 & 0xff) / 255,
                blue: CGFloat(hex & 0xff) / 255,
                alpha: 1
            )
        }
    }
}

extension ShapeStyle where Self == Color {
    static var themeText: Color { Color(nsColor: Theme.text) }
    static var themeSecondaryText: Color { Color(nsColor: Theme.secondaryText) }
    static var themeTertiaryText: Color { Color(nsColor: Theme.tertiaryText) }
}
