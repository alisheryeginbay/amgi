import AmgiCore
import SwiftUI

struct StudyView: View {
    @Environment(AppModel.self) private var model

    private var selected: DeckInfo? {
        model.decks.first { $0.name == model.selectedDeck }
    }

    var body: some View {
        Group {
            if let card = model.current {
                CardContent(card: card, revealed: model.revealed)
                    .id(card.id)
                    .transition(.opacity)
                    // Glass lives only on the floating controls, above the content.
                    .safeAreaInset(edge: .bottom) {
                        AnswerControls(card: card, revealed: model.revealed)
                            .padding(.bottom, 24)
                    }
            } else {
                ContentUnavailableView(
                    "All caught up",
                    systemImage: "checkmark",
                    description: Text("Nothing left to study right now.")
                )
            }
        }
        .navigationTitle(model.selectedDeck.map(deckTitle) ?? "All Cards")
        .navigationSubtitle(subtitle)
        .toolbar {
            if let card = model.current {
                ToolbarItem {
                    Button {
                        model.revealInEditor(card)
                    } label: {
                        Label("Edit Card", systemImage: "pencil")
                    }
                    .help("Open \(card.path), line \(card.line)")
                }
            }
        }
    }

    private var subtitle: String {
        let due = selected?.due ?? model.totalDue
        let new = selected?.new ?? model.today.newAvailable
        return "\(due) due · \(new) new"
    }
}

private struct CardContent: View {
    let card: StudyCard
    let revealed: Bool

    var body: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 0) {
                Text(card.deck)
                    .font(.subheadline)
                    .foregroundStyle(.tertiary)
                    .padding(.bottom, 20)

                switch card.kind {
                case .basic:
                    MarkdownText(card.front)
                        .font(.title)
                    if revealed {
                        Divider()
                            .padding(.vertical, 28)
                        MarkdownText(card.back)
                            .font(.title2)
                            .foregroundStyle(.secondary)
                            .transition(.opacity)
                    }
                case .cloze:
                    MarkdownText(revealed ? card.back : card.front, emphasis: .accentColor)
                        .font(.title)
                }
            }
            .textSelection(.enabled)
            .frame(maxWidth: 600, alignment: .leading)
            .padding(.horizontal, 48)
            .padding(.top, 72)
            .frame(maxWidth: .infinity)
        }
        .scrollBounceBehavior(.basedOnSize)
    }
}

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
                            withAnimation(.smooth(duration: 0.25)) { model.answer(rating) }
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
                        .keyboardShortcut(KeyEquivalent(Character("\(rating.index + 1)")), modifiers: [])
                        .help("\(rating.label) (\(rating.index + 1))")
                    }
                } else {
                    Button {
                        withAnimation(.smooth(duration: 0.3)) { model.revealed = true }
                    } label: {
                        Text("Show Answer")
                            .frame(width: 160, height: 44)
                            .contentShape(.capsule)
                    }
                    .buttonStyle(.plain)
                    .glassEffect(.regular.interactive(), in: .capsule)
                    .glassEffectID("show", in: glass)
                    .keyboardShortcut(.space, modifiers: [])
                    .help("Show Answer (Space)")
                }
            }
        }
    }
}

/// Inline Markdown that keeps the card's line breaks. With `emphasis`, bold runs
/// (cloze gaps and answers) are tinted instead of only bolded.
private struct MarkdownText: View {
    let source: String
    var emphasis: Color?

    init(_ source: String, emphasis: Color? = nil) {
        self.source = source
        self.emphasis = emphasis
    }

    var body: some View {
        Text(attributed)
            .lineSpacing(6)
            .frame(maxWidth: .infinity, alignment: .leading)
    }

    private var attributed: AttributedString {
        let options = AttributedString.MarkdownParsingOptions(interpretedSyntax: .inlineOnlyPreservingWhitespace)
        var text = (try? AttributedString(markdown: source, options: options)) ?? AttributedString(source)
        if let emphasis {
            let bold = text.runs
                .filter { $0.inlinePresentationIntent?.contains(.stronglyEmphasized) == true }
                .map(\.range)
            for range in bold {
                text[range].foregroundColor = emphasis
            }
        }
        return text
    }
}
