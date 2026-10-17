import AmgiCore
import SwiftUI

struct StudyView: View {
    @Environment(AppModel.self) private var model
    @AppStorage(autoplaySoundsKey) private var autoplay = true
    @State private var sounds = SoundPlayer()

    var body: some View {
        Group {
            if let card = model.current {
                CardContent(card: card, revealed: model.revealed, baseURL: deckDirectory(card), icons: model.tagIcons)
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
        .environment(sounds)
        // Play a card's sounds when it appears, then the answer's when it is revealed.
        .onChange(of: model.current.map { "\($0.id) \(model.revealed)" }, initial: true) { _, key in
            if key != nil && autoplay {
                playSounds()
            } else {
                sounds.stop()
            }
        }
        .onDisappear { sounds.stop() }
        .navigationTitle(model.folder?.lastPathComponent ?? "All Cards")
        .navigationSubtitle("\(model.totalDue) due · \(model.today.newAvailable) new")
        .toolbar {
            if let card = model.current, !soundURLs(card, side: .all).isEmpty {
                ToolbarItem {
                    Button {
                        playSounds(replay: true)
                    } label: {
                        Label("Replay Audio", systemImage: "speaker.wave.2")
                    }
                    .keyboardShortcut("r", modifiers: [])
                    .help("Replay Audio (R)")
                }
            }
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

    private enum Side {
        /// Sounds on the side showing.
        case all
        /// Sounds the revealed answer adds to the question, as Anki plays them.
        case added
    }

    private func soundURLs(_ card: StudyCard, side: Side) -> [URL] {
        let base = deckDirectory(card)
        let front = soundLinks(in: card.front, baseURL: base)
        guard model.revealed else { return front }
        // A cloze's answer is the whole card again; a basic one shows both sides.
        let back = soundLinks(in: card.back, baseURL: base)
        let shown = card.kind == .cloze ? back : front + back
        return side == .all ? shown : back.filter { !front.contains($0) }
    }

    /// Sounds for the side showing. On the answer side that is what the answer adds;
    /// a replay falls back to every sound shown when the answer adds none.
    private func playSounds(replay: Bool = false) {
        guard let card = model.current else { return }
        var urls = soundURLs(card, side: model.revealed ? .added : .all)
        if replay && urls.isEmpty {
            urls = soundURLs(card, side: .all)
        }
        if urls.isEmpty {
            sounds.stop()
        } else {
            sounds.play(urls)
        }
    }
}

private struct CardContent: View {
    let card: StudyCard
    let revealed: Bool
    let baseURL: URL?
    let icons: [String: String]

    var body: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 0) {
                HStack(spacing: 10) {
                    Text(card.deck)
                        .foregroundStyle(.tertiary)
                    TagPills(tags: card.tags, icons: icons)
                }
                .font(.subheadline)
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
                    CardImage(url: resolveMedia(path, angled: angled, baseURL: baseURL), alt: alt)
                case .sounds(let links):
                    HStack(spacing: 8) {
                        ForEach(Array(links.enumerated()), id: \.offset) { _, link in
                            SoundButton(url: resolveMedia(link.path, angled: link.angled, baseURL: baseURL))
                        }
                    }
                }
            }
        }
        .frame(maxWidth: .infinity, alignment: .leading)
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

/// A media link's destination; `angled` when written as `<path>`.
private struct MediaLink: Equatable {
    let path: String
    let angled: Bool
}

/// Resolve a media link written in a card. Relative paths are relative to `baseURL`.
private func resolveMedia(_ path: String, angled: Bool, baseURL: URL?) -> URL? {
    if let url = URL(string: path), url.scheme != nil { return url }
    // `<path>` is literal; a bare destination may be percent-encoded.
    let path = angled ? path : (path.removingPercentEncoding ?? path)
    return baseURL?.appending(path: path).standardized
}

/// Every sound linked in `source`, in order.
private func soundLinks(in source: String, baseURL: URL?) -> [URL] {
    MarkdownBlock.parse(source).flatMap { block -> [URL] in
        guard case .sounds(let links) = block else { return [] }
        return links.compactMap { resolveMedia($0.path, angled: $0.angled, baseURL: baseURL) }
    }
}

private enum MarkdownBlock {
    case text(String)
    case image(alt: String, path: String, angled: Bool)
    /// Sounds next to each other share a row.
    case sounds([MediaLink])

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
            let path = String(match.output.2 ?? match.output.3 ?? "")
            if isSound(path) {
                let link = MediaLink(path: path, angled: angled)
                if case .sounds(let links)? = blocks.last {
                    blocks[blocks.count - 1] = .sounds(links + [link])
                } else {
                    blocks.append(.sounds([link]))
                }
            } else {
                blocks.append(.image(alt: String(match.output.1), path: path, angled: angled))
            }
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

/// A button playing one sound. Missing and unplayable files are shown crossed out.
private struct SoundButton: View {
    @Environment(SoundPlayer.self) private var player
    let url: URL?

    var body: some View {
        let missing = url.map { $0.isFileURL && !FileManager.default.fileExists(atPath: $0.path(percentEncoded: false)) } ?? true
        let unplayable = missing || url.map(player.unplayable.contains) == true
        let name = url?.lastPathComponent ?? "Sound"
        Button {
            if let url { player.play([url]) }
        } label: {
            Image(systemName: unplayable ? "speaker.slash" : player.current == url ? "speaker.wave.2.fill" : "speaker.wave.2")
                .font(.title3)
                .frame(width: 36, height: 36)
                .contentShape(.circle)
        }
        .buttonStyle(.plain)
        .foregroundStyle(unplayable ? .secondary : .primary)
        .glassEffect(.regular.interactive(), in: .circle)
        .disabled(missing)
        .help(missing ? "Sound not found: \(url?.path(percentEncoded: false) ?? "")"
              : unplayable ? "Can't play \(name)" : "Play \(name)")
    }
}
