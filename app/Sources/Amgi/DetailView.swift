import AmgiCore
import AppKit
import SwiftUI

/// Whether browsing shows the selected card beside the table.
private let browsePreviewKey = "browsePreview"

/// The detail column: the shared web view for every pane, under native titles and
/// toolbar items, with the glass answer buttons floating over study cards.
struct DetailView: View {
    @Environment(AppModel.self) private var model
    @AppStorage(autoplaySoundsKey) private var autoplay = true
    @AppStorage(browsePreviewKey) private var preview = true

    var body: some View {
        WebContent(host: model.web, state: model.webState(autoplay: autoplay, preview: preview))
            // Glass lives only on the floating controls, above the content.
            .safeAreaInset(edge: .bottom) {
                if model.selection == .review, let card = model.current {
                    AnswerControls(card: card, revealed: model.revealed)
                        .padding(.bottom, 24)
                }
            }
            .overlay {
                if model.selection == .review && model.current == nil {
                    ContentUnavailableView(
                        "All caught up",
                        systemImage: "checkmark",
                        description: Text("Nothing left to study right now.")
                    )
                }
            }
            .navigationTitle(title)
            .navigationSubtitle(subtitle)
            .toolbar { DetailToolbar() }
    }

    private var title: String {
        switch model.selection {
        case .dashboard: "Dashboard"
        case .review: model.folder?.lastPathComponent ?? "All Cards"
        case .tag(let tag): model.icon(forTag: tag).map { "\($0) \(tag)" } ?? "#\(tag)"
        case .deck(let name): model.deckNames.first { $0.name == name }?.title ?? deckTitle(name)
        case .leeches: "Leeches"
        }
    }

    private var subtitle: String {
        switch model.selection {
        case .dashboard: model.folder?.lastPathComponent ?? ""
        case .review: "\(model.totalDue) due · \(model.today.newAvailable) new"
        case .deck, .tag, .leeches: model.browsed.count == 1 ? "1 card" : "\(model.browsed.count) cards"
        }
    }
}

private struct DetailToolbar: ToolbarContent {
    @Environment(AppModel.self) private var model
    @AppStorage(browsePreviewKey) private var preview = true

    var body: some ToolbarContent {
        switch model.selection {
        case .review:
            if let card = model.current {
                if card.hasSounds {
                    ToolbarItem {
                        Button {
                            model.web.command("replay")
                        } label: {
                            Label("Replay Audio", systemImage: "speaker.wave.2")
                        }
                        .help("Replay Audio (R)")
                    }
                }
                ToolbarItem {
                    Button {
                        model.setSuspended([card.id], true)
                    } label: {
                        Label("Suspend Card", systemImage: "pause.circle")
                    }
                    .help(card.kind == .cloze
                        ? "Suspend Card: tags it #suspended, so none of this cloze's gaps come up"
                        : "Suspend Card: tags it #suspended, so it doesn't come up")
                }
                ToolbarItem {
                    Button {
                        model.revealInEditor(path: card.path)
                    } label: {
                        Label("Edit Card", systemImage: "pencil")
                    }
                    .help("Open \(card.path), line \(card.line)")
                }
            }
        case .deck, .tag, .leeches:
            ToolbarItem {
                let card = model.selectedBrowseCard
                Button {
                    if let card { model.revealInEditor(path: card.path) }
                } label: {
                    Label("Edit Card", systemImage: "pencil")
                }
                .disabled(card == nil)
                .help(card.map { "Open \($0.path), line \($0.line)" } ?? "Edit Card")
            }
            ToolbarItem {
                Button {
                    preview.toggle()
                } label: {
                    Label(preview ? "Hide Card" : "Show Card", systemImage: "rectangle.righthalf.inset.filled")
                }
                .help(preview ? "Hide the selected card" : "Show the selected card beside the table")
            }
            ToolbarItem {
                @Bindable var model = model
                SearchField(text: $model.search, prompt: "Search Cards")
                    .frame(width: 200)
            }
        case .dashboard:
            ToolbarItem { EmptyView() }
        }
    }
}

/// The toolbar search field. A native one, because `searchable` would show on every pane.
private struct SearchField: NSViewRepresentable {
    @Binding var text: String
    let prompt: String

    func makeNSView(context: Context) -> NSSearchField {
        let field = NSSearchField()
        field.placeholderString = prompt
        field.sendsSearchStringImmediately = true
        field.delegate = context.coordinator
        return field
    }

    func updateNSView(_ field: NSSearchField, context: Context) {
        context.coordinator.text = $text
        if field.stringValue != text { field.stringValue = text }
    }

    func makeCoordinator() -> Coordinator { Coordinator(text: $text) }

    @MainActor
    final class Coordinator: NSObject, NSSearchFieldDelegate {
        var text: Binding<String>

        init(text: Binding<String>) {
            self.text = text
        }

        func controlTextDidChange(_ notification: Notification) {
            guard let field = notification.object as? NSSearchField else { return }
            text.wrappedValue = field.stringValue
        }
    }
}

/// Rating buttons once the answer shows, "Show Answer" before. The keys (Space, 1–4)
/// are handled by the page; `WebHost` forwards them there while focus is elsewhere.
private struct AnswerControls: View {
    @Environment(AppModel.self) private var model
    @Namespace private var glass
    let card: StudyCard
    let revealed: Bool

    var body: some View {
        GlassEffectContainer(spacing: 8) {
            HStack(spacing: 8) {
                if revealed {
                    ForEach(Rating.allCases) { rating in
                        Button {
                            model.answerWithAnimation(rating)
                        } label: {
                            VStack(spacing: 1) {
                                Text(rating.label)
                                Text(formatInterval(seconds: card.intervals[rating.index]))
                                    .font(.caption)
                                    .foregroundStyle(.secondary)
                                    .monospacedDigit()
                            }
                            .frame(width: 72, height: 44)
                            .contentShape(.capsule)
                        }
                        .buttonStyle(.plain)
                        .glassEffect(.regular.interactive(), in: .capsule)
                        .glassEffectID(rating.label, in: glass)
                        .help("\(rating.label) (\(rating.index + 1))")
                    }
                } else {
                    Button {
                        model.revealWithAnimation()
                    } label: {
                        Text("Show Answer")
                            .frame(width: 160, height: 44)
                            .contentShape(.capsule)
                    }
                    .buttonStyle(.plain)
                    .glassEffect(.regular.interactive(), in: .capsule)
                    .glassEffectID("show", in: glass)
                    .help("Show Answer (Space)")
                }
            }
        }
    }
}

private extension StudyCard {
    /// Whether the card links a sound, for the replay button. The page decides which play.
    var hasSounds: Bool {
        let link = /!\[[^\]]*\]\(<?[^)>]*\.(?i:mp3|m4a|m4b|aac|wav|aif|aiff|caf|flac|ogg|oga|opus|mp4|m4v|mov|webm)>?\)/
        return front.contains(link) || back.contains(link)
    }
}
