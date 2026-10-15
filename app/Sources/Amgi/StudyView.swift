import AmgiCore
import SwiftUI

struct StudyView: View {
    @Environment(AppModel.self) private var model

    var body: some View {
        Group {
            if let card = model.current {
                CardContent(card: card, revealed: model.revealed, baseURL: deckDirectory(card))
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
        .navigationTitle(model.folder?.lastPathComponent ?? "All Cards")
        .navigationSubtitle("\(model.totalDue) due · \(model.today.newAvailable) new")
        .toolbar {
            if let card = model.current {
                ToolbarItem {
                    Button {
                        model.revealInEditor(path: card.path)
                    } label: {
                        Label("Edit Card", systemImage: "pencil")
                    }
                    .help("Open \(card.path), line \(card.line)")
                }
            }
        }
    }

    /// Image paths in a card are relative to its deck file.
    private func deckDirectory(_ card: StudyCard) -> URL? {
        model.folder?.appending(path: card.path).deletingLastPathComponent()
    }
}

private struct CardContent: View {
    let card: StudyCard
    let revealed: Bool
    let baseURL: URL?

    var body: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 0) {
                Text(([card.deck] + card.tags.map { "#\($0)" }).joined(separator: "  "))
                    .font(.subheadline)
                    .foregroundStyle(.tertiary)
                    .padding(.bottom, 20)

                switch card.kind {
                case .basic:
                    MarkdownText(card.front, baseURL: baseURL)
                        .font(.title)
                    if revealed {
                        Divider()
                            .padding(.vertical, 28)
                        MarkdownText(card.back, baseURL: baseURL)
                            .font(.title2)
                            .foregroundStyle(.secondary)
                            .transition(.opacity)
                    }
                case .cloze:
                    MarkdownText(revealed ? card.back : card.front, baseURL: baseURL, emphasis: .accentColor)
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

/// Inline Markdown that keeps the card's line breaks, with images `![alt](path)` on
/// rows of their own. Relative image paths resolve against `baseURL`. With `emphasis`,
/// bold runs (cloze gaps and answers) are tinted instead of only bolded.
private struct MarkdownText: View {
    let source: String
    var baseURL: URL?
    var emphasis: Color?

    init(_ source: String, baseURL: URL?, emphasis: Color? = nil) {
        self.source = source
        self.baseURL = baseURL
        self.emphasis = emphasis
    }

    var body: some View {
        VStack(alignment: .leading, spacing: 16) {
            ForEach(Array(MarkdownBlock.parse(source).enumerated()), id: \.offset) { _, block in
                switch block {
                case .text(let text):
                    Text(attributed(text))
                        .lineSpacing(6)
                case .image(let alt, let path, let angled):
                    CardImage(url: resolve(path, angled: angled), alt: alt)
                }
            }
        }
        .frame(maxWidth: .infinity, alignment: .leading)
    }

    private func resolve(_ path: String, angled: Bool) -> URL? {
        if let url = URL(string: path), url.scheme != nil { return url }
        // `<path>` is literal; a bare destination may be percent-encoded.
        let path = angled ? path : (path.removingPercentEncoding ?? path)
        return baseURL?.appending(path: path).standardized
    }

    private func attributed(_ source: String) -> AttributedString {
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

private enum MarkdownBlock {
    case text(String)
    case image(alt: String, path: String, angled: Bool)

    static func parse(_ source: String) -> [MarkdownBlock] {
        // `![alt](path)` or `![alt](<path>)`; alt text may hold balanced brackets.
        let image = #/!\[((?:[^\[\]]|\[[^\[\]]*\])*)\]\((?:<([^>\n]*)>|([^)\s]*))\)/#
        var blocks: [MarkdownBlock] = []
        var rest = source[...]
        func addText(_ text: Substring) {
            // Images sit on rows of their own, so space next to them is not needed.
            let text = text.trimmingCharacters(in: .whitespacesAndNewlines)
            if !text.isEmpty {
                blocks.append(.text(text))
            }
        }
        while let match = rest.firstMatch(of: image) {
            var before = rest[..<match.range.lowerBound]
            var after = rest[match.range.upperBound...]
            // Drop emphasis wrapped around the image, as in a cloze answer `**![](x.png)**`.
            let stars = min(before.reversed().prefix { $0 == "*" }.count, after.prefix { $0 == "*" }.count)
            before = before.dropLast(stars)
            after = after.dropFirst(stars)
            addText(before)
            let angled = match.output.2 != nil
            let path = match.output.2 ?? match.output.3 ?? ""
            blocks.append(.image(alt: String(match.output.1), path: String(path), angled: angled))
            rest = after
        }
        addText(rest)
        return blocks
    }
}

/// A card image at its natural size, scaled down to fit.
private struct CardImage: View {
    let url: URL?
    let alt: String
    @State private var image: NSImage?
    @State private var failed = false

    var body: some View {
        Group {
            if let image {
                Image(nsImage: image)
                    .resizable()
                    .scaledToFit()
                    .frame(maxWidth: image.size.width, maxHeight: 360, alignment: .leading)
                    .clipShape(.rect(cornerRadius: 8))
            } else if failed || url == nil {
                Label(alt.isEmpty ? (url?.lastPathComponent ?? "Image") : alt, systemImage: "photo")
                    .font(.body)
                    .foregroundStyle(.secondary)
                    .help("Image not found: \(url?.path(percentEncoded: false) ?? "")")
            } else {
                ProgressView()
                    .controlSize(.small)
            }
        }
        .task(id: url) { await load() }
    }

    private func load() async {
        image = nil
        failed = false
        guard let url else { return }
        // Read off the main thread; local files and remote images alike.
        let data: Data? = if url.isFileURL {
            await Task.detached { try? Data(contentsOf: url) }.value
        } else {
            try? await URLSession.shared.data(from: url).0
        }
        image = data.flatMap(NSImage.init(data:))
        failed = image == nil
    }
}
