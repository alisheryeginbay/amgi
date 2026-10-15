import AmgiCore
import SwiftUI

struct StudyView: View {
    @Environment(AppModel.self) private var model

    var body: some View {
        if let card = model.current {
            CardView(card: card, revealed: model.revealed)
                .id(card.id)
        } else {
            ContentUnavailableView(
                "All Done",
                systemImage: "checkmark.circle",
                description: Text("No cards are due in \(model.selectedDeck ?? "this collection") right now.")
            )
        }
    }
}

private struct CardView: View {
    @Environment(AppModel.self) private var model
    let card: StudyCard
    let revealed: Bool

    var body: some View {
        VStack(spacing: 0) {
            ScrollView {
                VStack(alignment: .leading, spacing: 20) {
                    switch card.kind {
                    case .basic:
                        MarkdownText(card.front)
                        if revealed {
                            Divider()
                            MarkdownText(card.back)
                        }
                    case .cloze:
                        MarkdownText(revealed ? card.back : card.front)
                    }
                }
                .font(.title2)
                .textSelection(.enabled)
                .frame(maxWidth: 640, alignment: .leading)
                .padding(40)
                .frame(maxWidth: .infinity)
            }

            Divider()
            footer
                .padding()
        }
        .navigationSubtitle(card.deck)
    }

    private var footer: some View {
        HStack {
            Button {
                model.revealInEditor(card)
            } label: {
                Text("\(card.path):\(card.line)")
                    .font(.caption.monospaced())
            }
            .buttonStyle(.link)
            .help("Open the source file")
            .frame(maxWidth: .infinity, alignment: .leading)

            if revealed {
                HStack(spacing: 8) {
                    ForEach(Rating.allCases) { rating in
                        AnswerButton(rating: rating, interval: card.intervals[rating.index])
                    }
                }
            } else {
                Button("Show Answer") { model.revealed = true }
                    .keyboardShortcut(.space, modifiers: [])
                    .controlSize(.large)
                    .buttonStyle(.borderedProminent)
            }

            Text(card.isNew ? "New" : "Review")
                .font(.caption)
                .foregroundStyle(card.isNew ? .blue : .green)
                .frame(maxWidth: .infinity, alignment: .trailing)
        }
    }
}

private struct AnswerButton: View {
    @Environment(AppModel.self) private var model
    let rating: Rating
    let interval: Int64

    var body: some View {
        Button {
            model.answer(rating)
        } label: {
            VStack(spacing: 2) {
                Text(rating.label)
                Text(formatInterval(seconds: interval))
                    .font(.caption)
                    .foregroundStyle(.secondary)
            }
            .frame(minWidth: 56)
        }
        .controlSize(.large)
        .keyboardShortcut(KeyEquivalent(Character("\(rating.index + 1)")), modifiers: [])
        .help("\(rating.label) (\(rating.index + 1))")
    }
}

/// Markdown with inline styling; line breaks inside a card field are kept.
private struct MarkdownText: View {
    let source: String

    init(_ source: String) {
        self.source = source
    }

    var body: some View {
        let options = AttributedString.MarkdownParsingOptions(interpretedSyntax: .inlineOnlyPreservingWhitespace)
        let text = (try? AttributedString(markdown: source, options: options)) ?? AttributedString(source)
        Text(text)
            .frame(maxWidth: .infinity, alignment: .leading)
    }
}
