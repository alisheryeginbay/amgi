import AmgiCore
import AppKit
import SwiftUI

/// The window's content: a header with the collection and buttons over the web view the
/// cards show in, with the glass answer buttons floating over the card.
struct ReviewView: View {
    @Environment(AppModel.self) private var model
    @AppStorage(autoplaySoundsKey) private var autoplay = true
    @AppStorage(studyProgressKey) private var progress = true

    var body: some View {
        ZStack(alignment: .top) {
            web
                .frame(maxWidth: .infinity, maxHeight: .infinity)
                // Content scrolled up fades out under the header rather than being cut off,
                // just under the traffic lights.
                .mask {
                    VStack(spacing: 0) {
                        LinearGradient(colors: [.clear, .black], startPoint: .top, endPoint: .bottom)
                            .frame(height: 10)
                        Color.black
                    }
                }
                .padding(.top, titlebarDepth)
            ReviewHeader()
        }
    }

    private var web: some View {
        WebContent(host: model.web, state: model.webState(autoplay: autoplay, progress: progress))
            // Glass lives only on the floating controls, over the content, which scrolls
            // beneath them. The page leaves room below for them.
            .overlay(alignment: .bottom) {
                if let card = model.current {
                    AnswerControls(card: card, revealed: model.revealed)
                        .padding(.bottom, 24)
                }
            }
            .overlay {
                if model.current == nil {
                    ContentUnavailableView(
                        "All caught up",
                        systemImage: "checkmark",
                        description: Text("Nothing left to study right now.")
                    )
                }
            }
    }
}

/// The collection's name and what's left today on one line, then the buttons, level
/// with the traffic lights.
private struct ReviewHeader: View {
    @Environment(AppModel.self) private var model
    @State private var showingWarnings = false

    var body: some View {
        HStack(spacing: 4) {
            // One line, as the traffic lights beside it are: the counts follow the name,
            // and give way first when space runs short.
            HStack(alignment: .firstTextBaseline, spacing: 8) {
                Text(model.folder?.lastPathComponent ?? "")
                    .font(.system(size: 13, weight: .semibold))
                    .foregroundStyle(.themeText)
                    .layoutPriority(1)
                Text(model.studyCounts)
                    .font(.system(size: 12))
                    .foregroundStyle(.themeSecondaryText)
            }
            .lineLimit(1)
            .padding(.trailing, 12)
            Spacer(minLength: 0)
            buttons
        }
        // Clears the traffic lights.
        .padding(.leading, trafficLightsWidth)
        .padding(.trailing, 10)
        .frame(height: titlebarHeight)
        .windowDragArea()
    }

    @ViewBuilder private var buttons: some View {
        if !model.warnings.isEmpty {
            HeaderButton(
                "Problems", systemImage: "exclamationmark.triangle",
                help: "\(model.warnings.count) problems found while reading cards"
            ) {
                showingWarnings.toggle()
            }
            .popover(isPresented: $showingWarnings, arrowEdge: .bottom) { WarningList() }
        }
        if let card = model.current {
            if card.hasSounds {
                HeaderButton("Replay Audio", systemImage: "speaker.wave.2", help: "Replay Audio (R)") {
                    model.web.command("replay")
                }
            }
            // ⌘S is caught by `AppModel`, so it works from any focus.
            HeaderButton("More", systemImage: "ellipsis", help: "More") {
                model.showMoreMenu()
            }
            .background(MenuAnchorView { model.moreMenuAnchor = $0 })
            HeaderButton("Edit Card", systemImage: "pencil", help: "Open \(card.path), line \(card.line)") {
                model.revealInEditor(path: card.path)
            }
        }
    }
}

/// Where the header's title starts: past the traffic lights.
private let trafficLightsWidth: CGFloat = 96

/// Problems found in the deck files, each with its file and line.
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

/// A plain icon button for the header, shaded under the pointer.
private struct HeaderButton: View {
    let title: String
    let systemImage: String
    let help: String
    let action: () -> Void
    @State private var hovering = false
    @Environment(\.isEnabled) private var isEnabled

    init(_ title: String, systemImage: String, help: String, action: @escaping () -> Void) {
        self.title = title
        self.systemImage = systemImage
        self.help = help
        self.action = action
    }

    var body: some View {
        Button(action: action) {
            Image(systemName: systemImage)
                .font(.system(size: 14))
                .foregroundStyle(isEnabled ? (hovering ? .themeText : .themeSecondaryText) : .themeTertiaryText)
                .frame(width: 28, height: 28)
                .background(Color(nsColor: Theme.fill).opacity(hovering && isEnabled ? 1 : 0), in: .rect(cornerRadius: 6))
                .contentShape(.rect)
        }
        .buttonStyle(.plain)
        .accessibilityLabel(title)
        .help(help)
        .onHover { hovering = $0 }
    }
}

/// Hands over a header button's own view, for a popup menu to open under.
private struct MenuAnchorView: NSViewRepresentable {
    let onView: (NSView) -> Void

    func makeNSView(context: Context) -> NSView {
        let view = NSView()
        onView(view)
        return view
    }

    func updateNSView(_ view: NSView, context: Context) {
        onView(view)
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
                            model.answer(rating, via: .pointer)
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
                        model.reveal(via: .pointer)
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
