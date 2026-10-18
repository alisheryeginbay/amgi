import AmgiCore
import SwiftUI
import WebKit

struct ContentView: View {
    @Environment(AppModel.self) private var model
    @Environment(\.openWindow) private var openWindow

    var body: some View {
        Group {
            if model.folder == nil {
                // A fixed size: the scene sizes its window to the content.
                WelcomeView()
                    .frame(width: 480, height: 400)
                    // macOS's Move & Resize > Center is off for windows of a fixed size.
                    .background(CenterWindow())
            } else {
                NavigationSplitView {
                    Sidebar()
                        .navigationSplitViewColumnWidth(min: 190, ideal: 220, max: 300)
                } detail: {
                    DetailView()
                }
                .frame(minWidth: CollectionWindowFrame.minSize.width, minHeight: CollectionWindowFrame.minSize.height)
                // The welcome screen's window is small, so take back the size last used here.
                .background(CollectionWindowFrame())
            }
        }
        .onAppear {
            model.openCardInfoWindow = { [openWindow] in openWindow(id: "card-info") }
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
        .sheet(item: Binding(get: { model.editingView }, set: { model.editingView = $0 })) { draft in
            SavedViewEditor(draft: draft)
                .environment(model)
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
        .toolbar(removing: .title)
        .toolbarBackgroundVisibility(.hidden, for: .windowToolbar)
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
        if let url = Bundle.module.url(forResource: "Amgi", withExtension: "svg"),
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

/// Only for show: clicks, selection and its context menu pass through to the window.
private final class PassiveWebView: WKWebView {
    override func hitTest(_ point: NSPoint) -> NSView? { nil }
}

private struct Sidebar: View {
    @Environment(AppModel.self) private var model
    @State private var showingWarnings = false
    @State private var expanded: Set<String> = []
    @AppStorage("decksExpanded") private var decksExpanded = true
    @AppStorage("tagsExpanded") private var tagsExpanded = true
    @AppStorage("viewsExpanded") private var viewsExpanded = true

    var body: some View {
        @Bindable var model = model
        List(selection: $model.selection) {
            Label("Dashboard", systemImage: "chart.bar.xaxis")
                .tag(SidebarItem.dashboard)
            Section("Review") {
                ReviewRow()
                    .tag(SidebarItem.review)
            }
            // Sections collapse from the chevron shown when hovering the header.
            if !model.deckNames.isEmpty {
                Section("Decks", isExpanded: $decksExpanded) {
                    ForEach(DeckNode.tree(decks: model.deckNames)) { node in
                        DeckTree(node: node, expanded: $expanded)
                    }
                }
            }
            if !model.savedViews.isEmpty {
                Section("Saved Views", isExpanded: $viewsExpanded) {
                    ForEach(model.savedViews, id: \.id) { view in
                        SavedViewRow(view: view)
                    }
                }
            }
            if !model.tags.isEmpty {
                Section("Tags", isExpanded: $tagsExpanded) {
                    ForEach(model.tags, id: \.self) { tag in
                        TagRow(tag: tag)
                    }
                }
            }
        }
        // The scroll bar appearing re-tiles the list and cancels its expand and collapse
        // animations, so the sidebar scrolls without one.
        .scrollIndicators(.never)
        .toolbar {
            if !model.warnings.isEmpty {
                ToolbarItem {
                    Button {
                        showingWarnings.toggle()
                    } label: {
                        Label("Warnings", systemImage: "exclamationmark.triangle")
                    }
                    .help("\(model.warnings.count) problems found while reading cards")
                    .popover(isPresented: $showingWarnings, arrowEdge: .bottom) { WarningList() }
                }
            }
        }
    }
}

// Rows that show counts read them in views of their own, so an answer re-renders those
// rows rather than the whole list.

private struct ReviewRow: View {
    @Environment(AppModel.self) private var model

    var body: some View {
        Label(model.folder?.lastPathComponent ?? "All Cards", systemImage: "rectangle.stack")
            .lineLimit(1)
            .badge(Int(model.totalDue))
    }
}

/// A deck in the sidebar: a deck file, a folder, or both (`lang.md` and `lang/`).
private struct DeckNode: Identifiable {
    let id: String
    let title: String
    let children: [DeckNode]

    /// The top-level decks.
    static func tree(decks: [DeckName]) -> [DeckNode] {
        // `decks` lists parents before children, so grouping keeps file order.
        let byParent = Dictionary(grouping: decks) { deck in
            deck.name.split(separator: "/").dropLast().joined(separator: "/")
        }
        func children(of parent: String) -> [DeckNode] {
            (byParent[parent] ?? []).map { deck in
                DeckNode(id: deck.name, title: deck.title, children: children(of: deck.name))
            }
        }
        return children(of: "")
    }
}

private struct DeckTree: View {
    let node: DeckNode
    @Binding var expanded: Set<String>

    var body: some View {
        if node.children.isEmpty {
            row
        } else {
            DisclosureGroup(isExpanded: isExpanded) {
                ForEach(node.children) { child in
                    DeckTree(node: child, expanded: $expanded)
                }
            } label: {
                row
            }
        }
    }

    private var row: some View {
        Label(node.title, systemImage: node.children.isEmpty ? "rectangle.portrait.on.rectangle.portrait" : "folder")
            .lineLimit(1)
            .tag(SidebarItem.deck(node.id))
    }

    private var isExpanded: Binding<Bool> {
        Binding(
            get: { expanded.contains(node.id) },
            set: { open in
                if open { expanded.insert(node.id) } else { expanded.remove(node.id) }
            }
        )
    }
}

private struct TagRow: View {
    @Environment(AppModel.self) private var model
    let tag: String
    @State private var choosingIcon = false

    var body: some View {
        let icon = model.icon(forTag: tag)
        Label {
            Text(tag)
        } icon: {
            if let icon { Text(icon) } else { Image(systemName: "number") }
        }
        .lineLimit(1)
        .tag(SidebarItem.tag(tag))
        .contextMenu {
            Button("Choose Icon…") { choosingIcon = true }
            if icon != nil {
                Button("Remove Icon") { model.setIcon(nil, forTag: tag) }
            }
        }
        .popover(isPresented: $choosingIcon, arrowEdge: .trailing) {
            EmojiPicker(current: icon) { model.setIcon($0, forTag: tag) }
        }
    }
}

private struct SavedViewRow: View {
    @Environment(AppModel.self) private var model
    let view: SavedView
    @State private var choosingIcon = false

    var body: some View {
        Label {
            Text(view.name)
        } icon: {
            if let icon = view.icon { Text(icon) } else { Image(systemName: "line.3.horizontal.decrease.circle") }
        }
        .lineLimit(1)
        .help(view.query)
        .tag(SidebarItem.view(view.id))
        .contextMenu {
            Button("Edit View…") { model.editSavedView(view) }
            Button("Choose Icon…") { choosingIcon = true }
            if view.icon != nil {
                Button("Remove Icon") { model.setIcon(nil, for: view) }
            }
            Divider()
            Button("Delete View") { model.deleteSavedView(view) }
        }
        .popover(isPresented: $choosingIcon, arrowEdge: .trailing) {
            EmojiPicker(current: view.icon) { model.setIcon($0, for: view) }
        }
    }
}

/// Names a saved view and its search.
private struct SavedViewEditor: View {
    @Environment(AppModel.self) private var model
    @Environment(\.dismiss) private var dismiss
    @State var draft: SavedViewDraft

    private var complete: Bool {
        !draft.name.trimmingCharacters(in: .whitespaces).isEmpty
            && !draft.query.trimmingCharacters(in: .whitespaces).isEmpty
    }

    var body: some View {
        VStack(alignment: .leading, spacing: 16) {
            Text(draft.id == nil ? "New Saved View" : "Edit Saved View")
                .font(.headline)
            Form {
                TextField("Name:", text: $draft.name, prompt: Text("Due this week"))
                TextField("Search:", text: $draft.query, prompt: Text("due<7 -is:suspended"))
                    .font(.body.monospaced())
            }
            Text("Searches work as in the browse search field, such as `#verbs`, `deck:french`, `is:due` or `lapses>3`.")
                .font(.caption)
                .foregroundStyle(.secondary)
                .fixedSize(horizontal: false, vertical: true)
            HStack {
                Spacer()
                Button("Cancel", role: .cancel) { dismiss() }
                    .keyboardShortcut(.cancelAction)
                Button(draft.id == nil ? "Add" : "Save") {
                    model.commit(draft)
                    dismiss()
                }
                .keyboardShortcut(.defaultAction)
                .disabled(!complete)
            }
        }
        .padding(20)
        .frame(width: 400)
    }
}

private struct WarningList: View {
    @Environment(AppModel.self) private var model

    var body: some View {
        List(Array(model.warnings.enumerated()), id: \.offset) { _, warning in
            VStack(alignment: .leading, spacing: 2) {
                Text(warning.message)
                Text("\(warning.path):\(warning.line)")
                    .font(.caption.monospaced())
                    .foregroundStyle(.secondary)
            }
        }
        .scrollContentBackground(.hidden)
        .frame(width: 360, height: 220)
    }
}
