import AmgiCore
import AppKit
import SwiftUI

/// The detail panel: a header with the title and buttons, over the shared web view for
/// every pane, with the glass answer buttons floating over study cards.
struct DetailView: View {
    @Environment(AppModel.self) private var model
    @AppStorage(autoplaySoundsKey) private var autoplay = true
    @AppStorage(studyProgressKey) private var progress = true

    var body: some View {
        ZStack(alignment: .top) {
            web
                .frame(maxWidth: .infinity, maxHeight: .infinity)
                // Content scrolled up fades out under the header rather than being cut off,
                // over the same band as the sidebar's rows, just under the traffic lights.
                .mask {
                    VStack(spacing: 0) {
                        LinearGradient(colors: [.clear, .black], startPoint: .top, endPoint: .bottom)
                            .frame(height: 10)
                        Color.black
                    }
                }
                .padding(.top, titlebarDepth - panelInset)
            DetailHeader(title: title, subtitle: subtitle)
        }
    }

    private var web: some View {
        WebContent(host: model.web, state: model.webState(autoplay: autoplay, progress: progress))
            // Glass lives only on the floating controls, over the content, which scrolls
            // beneath them. The page leaves room below for them.
            .overlay(alignment: .bottom) {
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
    }

    private var title: String {
        switch model.selection {
        case .dashboard: "Dashboard"
        case .review: model.folder?.lastPathComponent ?? "All Cards"
        }
    }

    private var subtitle: String {
        switch model.selection {
        case .dashboard: model.folder?.lastPathComponent ?? ""
        case .review: model.studyCounts
        }
    }
}

/// How far the panel is inset from the window's edges.
let panelInset: CGFloat = 8

/// The panel's header, which centers on the traffic lights as the panel starts lower.
private let headerHeight = (titlebarHeight / 2 - panelInset) * 2

/// The title and subtitle on one line, then the pane's buttons, level with the traffic lights.
private struct DetailHeader: View {
    @Environment(AppModel.self) private var model
    let title: String
    let subtitle: String

    var body: some View {
        HStack(spacing: 4) {
            // One line, as the traffic lights beside it are: the subtitle follows the
            // title, and gives way first when space runs short.
            HStack(alignment: .firstTextBaseline, spacing: 8) {
                Text(title)
                    .font(.system(size: 13, weight: .semibold))
                    .foregroundStyle(.themeText)
                    .layoutPriority(1)
                if !subtitle.isEmpty {
                    Text(subtitle)
                        .font(.system(size: 12))
                        .foregroundStyle(.themeSecondaryText)
                }
            }
            .lineLimit(1)
            .padding(.trailing, 12)
            Spacer(minLength: 0)
            buttons
        }
        // Clears the traffic lights and sidebar button, which sit over the panel when
        // the sidebar is hidden.
        .padding(.leading, model.sidebarShown ? 16 : 124)
        .padding(.trailing, 10)
        .frame(height: headerHeight)
        .windowDragArea()
        .animation(.smooth(duration: 0.25), value: model.sidebarShown)
    }

    @ViewBuilder private var buttons: some View {
        switch model.selection {
        case .review:
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
        case .dashboard:
            EmptyView()
        }
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
