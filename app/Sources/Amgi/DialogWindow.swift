import AppKit
import SwiftUI

extension View {
    /// Marks the window as a dialog, so tiling window managers such as yabai,
    /// AeroSpace and Amethyst float it instead of tiling it.
    func dialogWindow() -> some View {
        background(DialogWindow())
    }
}

private struct DialogWindow: NSViewRepresentable {
    func makeNSView(context: Context) -> NSView { DialogView() }
    func updateNSView(_ view: NSView, context: Context) {}

    private final class DialogView: NSView {
        override func viewDidMoveToWindow() {
            super.viewDidMoveToWindow()
            window?.setAccessibilitySubrole(.dialog)
        }
    }
}
