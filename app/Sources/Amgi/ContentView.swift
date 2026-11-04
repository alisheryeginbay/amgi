import AmgiCore
import AppKit
import SwiftUI
import WebKit

struct ContentView: View {
    @Environment(AppModel.self) private var model
    @Environment(\.openWindow) private var openWindow
    @Environment(\.openSettings) private var openSettings

    var body: some View {
        Group {
            if model.folder == nil {
                // A fixed size: the scene sizes its window to the content.
                WelcomeView()
                    .frame(width: 480, height: 400)
                    // macOS's Move & Resize > Center is off for windows of a fixed size.
                    .background(CenterWindow())
            } else {
                CollectionLayout()
                .frame(minWidth: CollectionWindowFrame.minSize.width, minHeight: CollectionWindowFrame.minSize.height)
                // The welcome screen's window is small, so take back the size last used here.
                .background(CollectionWindowFrame())
            }
        }
        .background(WindowChrome())
        .onAppear {
            model.openCardInfoWindow = { [openWindow] in openWindow(id: "card-info") }
            model.openShortcutsWindow = { [openWindow] in openWindow(id: "keyboard-shortcuts") }
            model.openSettingsWindow = { [openSettings] in openSettings() }
        }
        .alert(
            "Something went wrong",
            isPresented: Binding(get: { model.errorMessage != nil }, set: { if !$0 { model.errorMessage = nil } })
        ) {
            Button("OK", role: .cancel) {}
        } message: {
            Text(model.errorMessage ?? "")
        }
        .alert(
            "Anki Import Finished",
            isPresented: Binding(get: { model.importSummary != nil }, set: { if !$0 { model.importSummary = nil } }),
            presenting: model.importSummary
        ) { _ in
            Button("OK", role: .cancel) {}
        } message: { summary in
            Text(summary.message)
        }
    }
}

private struct WelcomeView: View {
    @Environment(AppModel.self) private var model

    var body: some View {
        VStack(spacing: 28) {
            HandwrittenLogo()
                .frame(width: 300, height: 181)
            VStack(spacing: 16) {
                Text("Open a folder of Markdown flash cards.")
                    .foregroundStyle(.secondary)
                Button("Open Folder…") { model.chooseFolder() }
                    .buttonStyle(.glassProminent)
                    .controlSize(.large)
                    .keyboardShortcut("o")
            }
        }
        .frame(maxWidth: .infinity, maxHeight: .infinity)
    }
}

/// "Amgi", then 암기, written by hand in a loop: the README's `media/amgi.svg`. It's
/// animated with CSS, so a web view draws it.
private struct HandwrittenLogo: NSViewRepresentable {
    func makeNSView(context: Context) -> WKWebView {
        let view = PassiveWebView()
        // Transparent, and the SVG follows the app's light or dark appearance.
        view.setValue(false, forKey: "drawsBackground")
        view.underPageBackgroundColor = .clear
        view.setAccessibilityElement(true)
        view.setAccessibilityLabel("Amgi")
        if let url = Bundle.resources.url(forResource: "Amgi", withExtension: "svg"),
           let svg = try? String(contentsOf: url, encoding: .utf8) {
            view.loadHTMLString(Self.page(svg), baseURL: nil)
        }
        return view
    }

    func updateNSView(_ view: WKWebView, context: Context) {}

    private static func page(_ svg: String) -> String {
        """
        <!doctype html>
        <meta name="color-scheme" content="light dark">
        <style>
        html, body { margin: 0; height: 100%; overflow: hidden; background: transparent; }
        svg { display: block; width: 100%; height: 100%; }
        </style>
        \(svg)
        """
    }
}

/// Centers its window on screen when it appears, once the window has taken its new size.
private struct CenterWindow: NSViewRepresentable {
    func makeNSView(context: Context) -> NSView { CenteringView() }
    func updateNSView(_ view: NSView, context: Context) {}

    private final class CenteringView: NSView {
        override func viewDidMoveToWindow() {
            super.viewDidMoveToWindow()
            guard let window else { return }
            DispatchQueue.main.async { [weak window] in window?.center() }
        }
    }
}

/// Gives the window the frame it last had with a collection open, or a centered default
/// the first time, and saves the frame as it's moved or resized. Saved under its own
/// name, apart from the welcome screen's fixed size.
private struct CollectionWindowFrame: NSViewRepresentable {
    static let minSize = CGSize(width: 720, height: 460)
    private static let defaultSize = CGSize(width: 1000, height: 680)
    private static let name = "Amgi Collection Window"

    func makeNSView(context: Context) -> NSView { FrameView() }
    func updateNSView(_ view: NSView, context: Context) {}

    private final class FrameView: NSView {
        private var observers: [NSObjectProtocol] = []

        override func viewWillMove(toWindow newWindow: NSWindow?) {
            super.viewWillMove(toWindow: newWindow)
            stopObserving()
        }

        override func viewDidMoveToWindow() {
            super.viewDidMoveToWindow()
            guard let window else { return }
            // After SwiftUI has sized the window for the new content.
            DispatchQueue.main.async { [weak self, weak window] in
                guard let self, let window, self.window === window else { return }
                if !window.setFrameUsingName(CollectionWindowFrame.name) {
                    window.setContentSize(CollectionWindowFrame.defaultSize)
                    window.center()
                }
                self.observe(window)
            }
        }

        private func observe(_ window: NSWindow) {
            observers = [NSWindow.didMoveNotification, NSWindow.didResizeNotification].map { name in
                NotificationCenter.default.addObserver(forName: name, object: window, queue: .main) { [weak window] _ in
                    MainActor.assumeIsolated {
                        guard let window, !window.styleMask.contains(.fullScreen) else { return }
                        // Skip the welcome screen's size, in case it lands before this view is gone.
                        let size = window.contentRect(forFrameRect: window.frame).size
                        guard size.width >= CollectionWindowFrame.minSize.width,
                              size.height >= CollectionWindowFrame.minSize.height
                        else { return }
                        window.saveFrame(usingName: CollectionWindowFrame.name)
                    }
                }
            }
        }

        private func stopObserving() {
            observers.forEach(NotificationCenter.default.removeObserver)
            observers = []
        }
    }
}

/// The sidebar beside the detail panel, which is inset from the window's edges with
/// rounded corners. Both reach up into the title bar.
private struct CollectionLayout: View {
    @Environment(AppModel.self) private var model
    @AppStorage("sidebarWidth") private var sidebarWidth = 220.0

    var body: some View {
        HStack(spacing: 0) {
            if model.sidebarShown {
                Sidebar()
                    .frame(width: sidebarWidth)
                    .overlay(alignment: .trailing) { SidebarResizer(width: $sidebarWidth) }
                    .transition(.move(edge: .leading))
            }
            DetailView()
                .background(Color(nsColor: Theme.panel))
                .clipShape(.rect(cornerRadius: 12, style: .continuous))
                .overlay {
                    RoundedRectangle(cornerRadius: 12, style: .continuous)
                        .strokeBorder(Color(nsColor: Theme.separator))
                }
                .padding(.leading, model.sidebarShown ? 0 : panelInset)
                .padding([.top, .bottom, .trailing], panelInset)
        }
        .animation(.smooth(duration: 0.25), value: model.sidebarShown)
        .ignoresSafeArea()
        // The sidebar and the gaps around the panel.
        .background(Color(nsColor: Theme.sidebar).ignoresSafeArea())
        .background(TitlebarSidebarToggle(toggle: model.toggleSidebar))
    }
}

/// Drags the sidebar's edge to resize it.
private struct SidebarResizer: View {
    @Binding var width: Double
    @State private var start: Double?

    var body: some View {
        Color.clear
            .frame(width: 8)
            .contentShape(.rect)
            .pointerStyle(.frameResize(position: .trailing))
            .gesture(
                DragGesture(minimumDistance: 1, coordinateSpace: .global)
                    .onChanged { drag in
                        let start = start ?? width
                        self.start = start
                        width = min(max(start + drag.translation.width, 190), 320)
                    }
                    .onEnded { _ in start = nil }
            )
    }
}

/// Lets the content fill the whole window under a clear title bar, and moves the
/// traffic lights down level with the panel's header. An empty toolbar would lower them
/// too, but its edge shows as a line across the window while it's inactive.
private struct WindowChrome: NSViewRepresentable {
    func makeNSView(context: Context) -> NSView { ChromeView() }
    func updateNSView(_ view: NSView, context: Context) {}

    private final class ChromeView: NSView {
        private var observers: [NSObjectProtocol] = []

        override func viewWillMove(toWindow newWindow: NSWindow?) {
            super.viewWillMove(toWindow: newWindow)
            observers.forEach(NotificationCenter.default.removeObserver)
            observers = []
        }

        override func viewDidMoveToWindow() {
            super.viewDidMoveToWindow()
            guard let window, let close = window.standardWindowButton(.closeButton),
                  let titlebar = close.superview, let container = titlebar.superview
            else { return }
            window.styleMask.insert(.fullSizeContentView)
            window.titlebarAppearsTransparent = true
            window.titleVisibility = .hidden
            window.titlebarSeparatorStyle = .none
            // AppKit puts the buttons back whenever it lays out the title bar.
            let center = NotificationCenter.default
            observers = [close, titlebar, container].map { view in
                view.postsFrameChangedNotifications = true
                return center.addObserver(forName: NSView.frameDidChangeNotification, object: view, queue: .main) { [weak window] _ in
                    MainActor.assumeIsolated { window?.placeTitlebarButtons() }
                }
            }
            observers += [NSWindow.didResizeNotification, NSWindow.didExitFullScreenNotification].map { name in
                center.addObserver(forName: name, object: window, queue: .main) { [weak window] _ in
                    MainActor.assumeIsolated { window?.placeTitlebarButtons() }
                }
            }
            window.placeTitlebarButtons()
        }
    }
}

extension NSWindow {
    /// The sidebar button placed after the traffic lights, if any.
    static let sidebarToggleIdentifier = NSUserInterfaceItemIdentifier("sidebarToggle")

    /// Centers the traffic lights on the sidebar's top row, as with a toolbar, with the
    /// sidebar button after them. Again once AppKit
    /// is done laying out, as some changes, such as the title, move them after this.
    func placeTitlebarButtons() {
        placeTitlebarButtonsNow()
        DispatchQueue.main.async { [weak self] in self?.placeTitlebarButtonsNow() }
    }

    private func placeTitlebarButtonsNow() {
        guard !styleMask.contains(.fullScreen),
              let close = standardWindowButton(.closeButton), let titlebar = close.superview,
              let container = titlebar.superview
        else { return }
        // Only as tall as its buttons need, centered on the header row: scroll views
        // reaching under the title bar get a hard edge from AppKit.
        let height = titlebarDepth
        let frame = NSRect(x: 0, y: self.frame.height - height, width: self.frame.width, height: height)
        if container.frame != frame { container.frame = frame }
        if titlebar.frame != container.bounds { titlebar.frame = container.bounds }
        /// The origin that centers a view of this height on the header row.
        func y(_ viewHeight: CGFloat) -> CGFloat {
            let top = (titlebarHeight / 2 - viewHeight / 2).rounded()
            return titlebar.isFlipped ? top : height - top - viewHeight
        }
        var x: CGFloat = 19
        for type in [NSWindow.ButtonType.closeButton, .miniaturizeButton, .zoomButton] {
            guard let button = standardWindowButton(type) else { continue }
            let origin = NSPoint(x: x, y: y(button.frame.height))
            if button.frame.origin != origin { button.setFrameOrigin(origin) }
            x = button.frame.maxX + 9
        }
        if let toggle = titlebar.subviews.first(where: { $0.identifier == Self.sidebarToggleIdentifier }) {
            let size = toggle.fittingSize
            let origin = NSPoint(x: x - 9 + 18, y: y(size.height))
            if toggle.frame != NSRect(origin: origin, size: size) { toggle.frame = NSRect(origin: origin, size: size) }
        }
    }
}

/// Shows or hides the sidebar from a plain icon just after the traffic lights, which
/// stays put whether the sidebar is shown or not.
private struct TitlebarSidebarToggle: NSViewRepresentable {
    let toggle: () -> Void

    func makeNSView(context: Context) -> NSView { HostView() }

    func updateNSView(_ view: NSView, context: Context) {
        (view as? HostView)?.button.onClick = toggle
    }

    private final class HostView: NSView {
        let button = ToggleButton()

        override func viewWillMove(toWindow newWindow: NSWindow?) {
            super.viewWillMove(toWindow: newWindow)
            button.removeFromSuperview()
        }

        override func viewDidMoveToWindow() {
            super.viewDidMoveToWindow()
            guard let window, let titlebar = window.standardWindowButton(.zoomButton)?.superview else { return }
            button.identifier = NSWindow.sidebarToggleIdentifier
            titlebar.addSubview(button)
            window.placeTitlebarButtons()
        }
    }

    /// Secondary until hovered, like the icons beside it in Finder's title bar.
    private final class ToggleButton: NSButton {
        var onClick: () -> Void = {}

        init() {
            super.init(frame: .zero)
            let symbol = NSImage(systemSymbolName: "sidebar.left", accessibilityDescription: "Show or Hide Sidebar")
            image = symbol?.withSymbolConfiguration(.init(pointSize: 15, weight: .regular))
            imagePosition = .imageOnly
            isBordered = false
            contentTintColor = Theme.secondaryText
            toolTip = "Show or Hide Sidebar (⌃⌘S)"
            target = self
            action = #selector(click)
        }

        required init?(coder: NSCoder) { fatalError("init(coder:) is not used") }

        @objc private func click() { onClick() }

        override func updateTrackingAreas() {
            super.updateTrackingAreas()
            trackingAreas.forEach(removeTrackingArea)
            addTrackingArea(NSTrackingArea(rect: bounds, options: [.mouseEnteredAndExited, .activeAlways, .inVisibleRect], owner: self))
        }

        override func mouseEntered(with event: NSEvent) { contentTintColor = Theme.text }
        override func mouseExited(with event: NSEvent) { contentTintColor = Theme.secondaryText }
    }
}

/// Only for show: clicks, selection and its context menu pass through to the window.
private final class PassiveWebView: WKWebView {
    override func hitTest(_ point: NSPoint) -> NSView? { nil }
}
