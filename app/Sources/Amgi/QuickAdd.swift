import AmgiCore
import AppKit
import SwiftUI

/// The deck cards were last added to, picked again next time.
private let quickAddDeckKey = "quickAddDeck"
/// What was last added: `basic`, `cloze`, or `note:` and a note type's name.
private let quickAddKindKey = "quickAddKind"

/// A floating panel for adding a card from anywhere, opened with ⌥⌘N. It takes
/// typing without bringing Amgi's main window to the front.
@MainActor
final class QuickAddPanel: NSObject, NSWindowDelegate {
    private var panel: NSPanel?

    var isShown: Bool { panel?.isVisible == true }

    func show(model: AppModel) {
        let panel = self.panel ?? makePanel(model: model)
        self.panel = panel
        if !panel.isVisible { position(panel) }
        panel.makeKeyAndOrderFront(nil)
    }

    func close() {
        panel?.orderOut(nil)
    }

    private func makePanel(model: AppModel) -> NSPanel {
        let panel = KeyPanel(
            contentRect: NSRect(x: 0, y: 0, width: 520, height: 300),
            styleMask: [.titled, .closable, .fullSizeContentView, .nonactivatingPanel],
            backing: .buffered,
            defer: true
        )
        panel.title = "Add Card"
        panel.titleVisibility = .hidden
        panel.titlebarAppearsTransparent = true
        for button in [NSWindow.ButtonType.closeButton, .miniaturizeButton, .zoomButton] {
            panel.standardWindowButton(button)?.isHidden = true
        }
        panel.isMovableByWindowBackground = true
        panel.level = .floating
        // Tiling window managers float it rather than tile it.
        panel.setAccessibilitySubrole(.floatingWindow)
        panel.collectionBehavior = [.canJoinAllSpaces, .fullScreenAuxiliary]
        panel.hidesOnDeactivate = false
        panel.isReleasedWhenClosed = false
        panel.delegate = self
        // The panel follows the form's height as fields come and go.
        let host = NSHostingController(rootView: QuickAddView(close: { [weak self] in self?.close() }).environment(model))
        host.sizingOptions = [.preferredContentSize]
        panel.contentViewController = host
        return panel
    }

    /// Centered, a little above the middle of the screen with the pointer, like Spotlight.
    private func position(_ panel: NSPanel) {
        let mouse = NSEvent.mouseLocation
        guard let screen = NSScreen.screens.first(where: { $0.frame.contains(mouse) }) ?? NSScreen.main else { return }
        let area = screen.visibleFrame
        let size = panel.frame.size
        panel.setFrameTopLeftPoint(NSPoint(x: area.midX - size.width / 2, y: area.maxY - area.height * 0.2))
    }

    /// Clicking elsewhere puts the panel away; what was typed stays for next time.
    func windowDidResignKey(_ notification: Notification) {
        close()
    }
}

/// A panel that can take typing while another app stays active, and closes on Escape.
private final class KeyPanel: NSPanel {
    override var canBecomeKey: Bool { true }

    override func cancelOperation(_ sender: Any?) {
        orderOut(nil)
    }
}

private struct QuickAddView: View {
    @Environment(AppModel.self) private var model
    let close: () -> Void

    @AppStorage(quickAddDeckKey) private var deck = ""
    @AppStorage(quickAddKindKey) private var kind = "basic"
    @State private var front = ""
    @State private var back = ""
    /// A note's field values by field name, so they carry over to a note type with the same fields.
    @State private var values: [String: String] = [:]
    @State private var tags = ""
    @State private var error: String?
    @FocusState private var focus: Field?

    private enum Field: Hashable { case front, back, note(Int), tags }

    var body: some View {
        Group {
            if model.folder == nil {
                VStack(spacing: 12) {
                    Text("Open a folder of flash cards to add cards to it.")
                        .foregroundStyle(.secondary)
                    Button("Open Folder…") {
                        close()
                        NSApp.activate()
                        model.chooseFolder()
                    }
                }
                .frame(maxWidth: .infinity)
                .padding(.vertical, 24)
            } else {
                form
            }
        }
        .padding(20)
        .frame(width: 520)
        .onAppear { focus = firstField }
    }

    /// The kind chosen: basic or cloze, or a note type that still exists.
    private var kindSelection: Binding<String> {
        Binding(
            get: { noteTypeInfo == nil && kind.hasPrefix("note:") ? "basic" : kind },
            set: { kind = $0 }
        )
    }

    private var cloze: Bool { kind == "cloze" }

    private var noteTypeInfo: NoteTypeInfo? {
        guard kind.hasPrefix("note:") else { return nil }
        let name = kind.dropFirst("note:".count)
        return model.noteTypes.first { $0.name == name }
    }

    /// The fields of the chosen note type; empty for basic and cloze cards.
    private var noteFields: [String] { noteTypeInfo?.fields ?? [] }

    private var firstField: Field { noteFields.isEmpty ? .front : .note(0) }

    private var noteValues: [String] { noteFields.map { values[$0] ?? "" } }

    /// How many cards the note makes, once something is typed; nil for basic and cloze cards.
    private var noteCards: Int? {
        guard let noteTypeInfo, noteValues.contains(where: { !$0.isEmpty }) else { return nil }
        return model.noteCardCount(noteType: noteTypeInfo.name, values: noteValues)
    }

    private var form: some View {
        VStack(alignment: .leading, spacing: 14) {
            HStack {
                Picker("Type", selection: kindSelection) {
                    Text("Basic").tag("basic")
                    Text("Cloze").tag("cloze")
                    if !model.noteTypes.isEmpty {
                        Divider()
                        ForEach(model.noteTypes, id: \.name) { type in
                            Text(type.name).tag("note:\(type.name)")
                        }
                    }
                }
                .labelsHidden()
                .fixedSize()
                Spacer()
                if model.deckNames.isEmpty {
                    // A new folder: the deck's file is created with the card.
                    TextField("Deck", text: $deck, prompt: Text("Deck name"))
                        .textFieldStyle(.roundedBorder)
                        .frame(width: 180)
                } else {
                    Picker("Deck", selection: deckSelection) {
                        ForEach(model.deckNames, id: \.name) { deck in
                            Text(label(deck)).tag(deck.name)
                        }
                    }
                    .fixedSize()
                }
            }

            if noteFields.isEmpty {
                field(cloze ? "Text" : "Question", text: $front, prompt: cloze ? "The capital of [France] is [Paris|city]." : "", focus: .front)
                if !cloze {
                    field("Answer", text: $back, prompt: "", focus: .back)
                }
            } else {
                ForEach(Array(noteFields.enumerated()), id: \.element) { index, name in
                    field(name, text: value(name), prompt: "", focus: .note(index))
                        .dropDestination(for: URL.self) { files, _ in drop(files, into: name) }
                }
            }
            field("Tags", text: $tags, prompt: "Separated by spaces", focus: .tags, multiline: false)

            HStack(alignment: .firstTextBaseline) {
                if let error {
                    Text(error)
                        .foregroundStyle(.red)
                        .fixedSize(horizontal: false, vertical: true)
                } else if let noteCards {
                    Text(noteCards == 1 ? "Makes 1 card" : "Makes \(noteCards) cards")
                        .foregroundStyle(.secondary)
                        .monospacedDigit()
                }
                Spacer()
                Button("Cancel", action: close)
                    .keyboardShortcut(.cancelAction)
                Button(noteFields.isEmpty ? "Add Card" : "Add Note", action: add)
                    .keyboardShortcut(.return, modifiers: .command)
                    .buttonStyle(.glassProminent)
                    .disabled(noteFields.isEmpty ? front.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty : (noteCards ?? 0) == 0)
            }
        }
        .onChange(of: kind) {
            error = nil
            focus = firstField
        }
        // A deck with notes offers its note type.
        .onChange(of: deckSelection.wrappedValue, initial: true) {
            if let noteType = model.deckNames.first(where: { $0.name == deckSelection.wrappedValue })?.noteType {
                kind = "note:\(noteType)"
            }
        }
    }

    private func value(_ name: String) -> Binding<String> {
        Binding(get: { values[name] ?? "" }, set: { values[name] = $0 })
    }

    /// Copies dropped images and sounds into the media folder and links them in the field.
    private func drop(_ files: [URL], into name: String) {
        let links = files.filter(\.isFileURL).compactMap { file -> String? in
            switch model.addMedia(deck: deckSelection.wrappedValue, file: file) {
            case .success(let link): return link
            case .failure(let failure):
                error = failure.localizedDescription
                return nil
            }
        }
        guard !links.isEmpty else { return }
        let current = values[name] ?? ""
        values[name] = ([current].filter { !$0.isEmpty } + links).joined(separator: " ")
        error = nil
    }

    private func field(_ title: String, text: Binding<String>, prompt: String, focus field: Field, multiline: Bool = true) -> some View {
        VStack(alignment: .leading, spacing: 4) {
            Text(title)
                .font(.callout)
                .foregroundStyle(.secondary)
            TextField(title, text: text, prompt: Text(prompt), axis: multiline ? .vertical : .horizontal)
                .labelsHidden()
                .lineLimit(multiline ? 1...8 : 1...1)
                .textFieldStyle(.roundedBorder)
                .focused($focus, equals: field)
                .onSubmit { advance(from: field) }
        }
    }

    /// Return moves to the next field, then adds the card.
    private func advance(from field: Field) {
        switch field {
        case .front: focus = cloze ? .tags : .back
        case .back: focus = .tags
        case .note(let index): focus = index + 1 < noteFields.count ? .note(index + 1) : .tags
        case .tags: add()
        }
    }

    /// The deck last used, or the first while none is.
    private var deckSelection: Binding<String> {
        Binding(
            get: { model.deckNames.isEmpty || model.deckNames.contains { $0.name == deck } ? deck : model.deckNames[0].name },
            set: { deck = $0 }
        )
    }

    /// The deck's path, with its front matter name when it has one.
    private func label(_ deck: DeckName) -> String {
        deck.title == deckTitle(deck.name) ? deck.name : "\(deck.name) (\(deck.title))"
    }

    private func add() {
        if !noteFields.isEmpty {
            addNote()
            return
        }
        let card = NewCard(
            deck: deckSelection.wrappedValue,
            kind: cloze ? .cloze : .basic,
            front: front,
            back: cloze ? "" : back,
            tags: tags.split(whereSeparator: \.isWhitespace).map(String.init)
        )
        if let message = model.addCard(card) {
            error = message
            return
        }
        // Tags often carry over to the next card; the rest starts afresh.
        front = ""
        back = ""
        error = nil
        focus = .front
        close()
    }

    private func addNote() {
        guard let noteTypeInfo else { return }
        let note = NewNote(
            deck: deckSelection.wrappedValue,
            noteType: noteTypeInfo.name,
            values: noteValues,
            tags: tags.split(whereSeparator: \.isWhitespace).map(String.init)
        )
        if let message = model.addNote(note) {
            error = message
            return
        }
        values = [:]
        error = nil
        focus = firstField
        close()
    }
}
