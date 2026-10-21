import AppKit
import SwiftUI

/// An emoji with what search matches it against.
struct Emoji: Hashable {
    let character: String
    let name: String
    /// Folded words of the name, shortcodes and keywords, for prefix matching.
    var words: [String] = []
    var nameWords: [String] = []
    var foldedName = ""
    /// Such as `thumbsup` for 👍.
    var shortcodes: [String] = []
}

struct EmojiCategory {
    let name: String
    let symbol: String
    let emoji: [Emoji]
}

/// The emoji from `Emoji.tsv`, written by `scripts/build-emoji.py`.
enum EmojiCatalog {
    static let categories: [EmojiCategory] = load()

    private static let byCharacter: [String: Emoji] = Dictionary(
        categories.flatMap(\.emoji).map { ($0.character, $0) },
        uniquingKeysWith: { first, _ in first }
    )

    private static let symbols = [
        "Smileys & Emotion": "face.smiling",
        "People & Body": "hand.wave",
        "Animals & Nature": "pawprint",
        "Food & Drink": "fork.knife",
        "Travel & Places": "car",
        "Activities": "soccerball",
        "Objects": "lightbulb",
        "Symbols": "heart",
        "Flags": "flag",
    ]

    static func emoji(for character: String) -> Emoji? { byCharacter[character] }

    /// Emoji whose name or keywords start with every word of `query`, best matches first,
    /// from one category or all of them.
    static func search(_ query: String, in category: String? = nil) -> [Emoji] {
        let query = fold(query.trimmingCharacters(in: .whitespaces))
        // An emoji typed or pasted into the field finds itself.
        if query.count == 1, let character = query.first, character.isEmoji {
            return [emoji(for: String(character)) ?? Emoji(character: String(character), name: "")]
        }
        let terms = query.split(whereSeparator: \.isWhitespace).map(String.init)
        guard !terms.isEmpty else { return [] }
        let shortcode = terms.joined(separator: "_")
        var matches: [(rank: Int, emoji: Emoji)] = []
        for emoji in categories.filter({ category == nil || $0.name == category }).flatMap(\.emoji) {
            guard terms.allSatisfy({ term in emoji.words.contains { $0.hasPrefix(term) } }) else { continue }
            let rank =
                if emoji.foldedName == query || emoji.shortcodes.contains(shortcode) { 0 }
                else if terms.allSatisfy(emoji.nameWords.contains) { 1 }
                else if emoji.foldedName.hasPrefix(query) { 2 }
                else if terms.allSatisfy({ term in emoji.nameWords.contains { $0.hasPrefix(term) } }) { 3 }
                else { 4 }
            matches.append((rank, emoji))
        }
        // Sorting by rank keeps catalog order within a rank.
        return matches.enumerated()
            .sorted { ($0.element.rank, $0.offset) < ($1.element.rank, $1.offset) }
            .map(\.element.emoji)
    }

    private static func fold(_ text: String) -> String {
        text.folding(options: [.caseInsensitive, .diacriticInsensitive], locale: nil)
    }

    private static func words(_ text: String) -> [String] {
        text.split(whereSeparator: { " -_:,“”.".contains($0) }).map(String.init)
    }

    private static func load() -> [EmojiCategory] {
        guard let url = Bundle.resources.url(forResource: "Emoji", withExtension: "tsv"),
              let text = try? String(contentsOf: url, encoding: .utf8)
        else { return [] }
        var categories: [EmojiCategory] = []
        var name = ""
        var emoji: [Emoji] = []
        func finish() {
            if !emoji.isEmpty {
                categories.append(EmojiCategory(name: name, symbol: symbols[name] ?? "circle", emoji: emoji))
            }
            emoji = []
        }
        for line in text.split(separator: "\n") {
            if line.hasPrefix("@") {
                finish()
                name = String(line.dropFirst())
                continue
            }
            let fields = line.split(separator: "\t", omittingEmptySubsequences: false)
            guard fields.count == 4 else { continue }
            let foldedName = fold(String(fields[1]))
            let nameWords = words(foldedName)
            emoji.append(Emoji(
                character: String(fields[0]),
                name: String(fields[1]),
                words: nameWords + words(fold(fields[2] + " " + fields[3])),
                nameWords: nameWords,
                foldedName: foldedName,
                shortcodes: fields[2].split(separator: " ").map(String.init)
            ))
        }
        finish()
        return categories
    }
}

/// Emoji chosen lately, most recent first.
enum RecentEmoji {
    private static let key = "recentEmoji"
    static let limit = 18

    static func load() -> [String] {
        UserDefaults.standard.stringArray(forKey: key) ?? []
    }

    static func add(_ character: String) {
        var recent = load().filter { $0 != character }
        recent.insert(character, at: 0)
        UserDefaults.standard.set(Array(recent.prefix(limit)), forKey: key)
    }
}

private extension Character {
    var isEmoji: Bool {
        guard let first = unicodeScalars.first else { return false }
        return first.properties.isEmojiPresentation || (first.properties.isEmoji && unicodeScalars.count > 1)
    }
}

/// A searchable emoji grid in the style of Raycast's: type to search, arrow keys to move,
/// Return to choose, Escape to clear the search or close.
/// Popovers are hosted outside the window's view tree, so it takes no environment.
struct EmojiPicker: View {
    let current: String?
    /// Called with the chosen emoji, or nil to remove the icon.
    let onChoose: (String?) -> Void

    private static let columns = 9
    private static let cellSize: CGFloat = 34
    private static let spacing: CGFloat = 2

    /// Fits the grid, plus room for a scroll bar when they are always shown.
    private static var width: CGFloat {
        let scroller = NSScroller.preferredScrollerStyle == .legacy
            ? NSScroller.scrollerWidth(for: .regular, scrollerStyle: .legacy)
            : 0
        return CGFloat(columns) * (cellSize + spacing) + 20 + scroller
    }

    @Environment(\.dismiss) private var dismiss
    @State private var query = ""
    @State private var category: String?
    @State private var grid = EmojiGrid()
    @State private var selection = 0
    @State private var pointer = PointerTracker()

    var body: some View {
        ScrollViewReader { proxy in
            VStack(spacing: 0) {
                searchBar(proxy)
                Divider()
                content
                Divider()
                footer
            }
            .frame(width: Self.width)
            .onAppear {
                pointer.reset()
                rebuild()
            }
            .onChange(of: query) { rebuild(proxy) }
            .onChange(of: category) { rebuild(proxy) }
        }
    }

    private func searchBar(_ proxy: ScrollViewProxy) -> some View {
        HStack(spacing: 8) {
            Image(systemName: "magnifyingglass")
                .foregroundStyle(.secondary)
            PickerSearchField(text: $query, placeholder: "Search emoji…") { command in
                handle(command, proxy)
            }
            Menu {
                Picker("Category", selection: $category) {
                    Text("All Categories").tag(String?.none)
                    Divider()
                    ForEach(EmojiCatalog.categories, id: \.name) { category in
                        Label(category.name, systemImage: category.symbol).tag(Optional(category.name))
                    }
                }
                .pickerStyle(.inline)
            } label: {
                Text(category ?? "All")
            }
            .menuStyle(.button)
            .buttonStyle(.borderless)
            .fixedSize()
            .help("Show one category")
        }
        .padding(.horizontal, 12)
        .padding(.vertical, 10)
    }

    @ViewBuilder
    private var content: some View {
        if grid.items.isEmpty {
            ContentUnavailableView.search(text: query)
                .frame(height: 300)
        } else {
            ScrollView {
                LazyVStack(alignment: .leading, spacing: Self.spacing) {
                    ForEach(grid.sections) { section in
                        Text(section.title)
                            .font(.caption.weight(.semibold))
                            .foregroundStyle(.secondary)
                            .padding(.horizontal, 4)
                            .padding(.top, 8)
                            .padding(.bottom, 2)
                        ForEach(section.rows) { row in
                            HStack(spacing: Self.spacing) {
                                ForEach(row.start..<row.start + row.count, id: \.self) { index in
                                    cell(index)
                                }
                            }
                            .id(row.id)
                        }
                    }
                }
                .padding(.horizontal, 10)
                .padding(.bottom, 8)
            }
            .frame(height: 300)
        }
    }

    private func cell(_ index: Int) -> some View {
        let emoji = grid.items[index]
        return Text(emoji.character)
            .font(.system(size: 22))
            .frame(width: Self.cellSize, height: Self.cellSize)
            .background {
                if index == selection {
                    RoundedRectangle(cornerRadius: 7).fill(Color.accentColor.opacity(0.25))
                } else if emoji.character == current {
                    RoundedRectangle(cornerRadius: 7).strokeBorder(Color.accentColor.opacity(0.6))
                }
            }
            .contentShape(.rect)
            .onHover { inside in
                // Only a moving pointer selects, so the grid scrolling under a still one doesn't.
                if inside, pointer.moved() { selection = index }
            }
            .onTapGesture { choose(emoji.character) }
            .help(emoji.name)
    }

    private var footer: some View {
        HStack(spacing: 8) {
            if grid.items.indices.contains(selection) {
                let emoji = grid.items[selection]
                Text(emoji.character)
                    .font(.system(size: 18))
                Text(emoji.name.prefix(1).uppercased() + emoji.name.dropFirst())
                    .foregroundStyle(.secondary)
                    .lineLimit(1)
                    .truncationMode(.tail)
            }
            Spacer(minLength: 8)
            if current != nil {
                Button("Remove Icon") {
                    onChoose(nil)
                    dismiss()
                }
                .buttonStyle(.borderless)
                Divider().frame(height: 14)
            }
            Button {
                if grid.items.indices.contains(selection) { choose(grid.items[selection].character) }
            } label: {
                HStack(spacing: 6) {
                    Text("Choose")
                    Text("↩")
                        .font(.caption.weight(.medium))
                        .frame(minWidth: 18, minHeight: 16)
                        .background(.quaternary, in: .rect(cornerRadius: 4))
                }
            }
            .buttonStyle(.borderless)
            .foregroundStyle(.primary)
            .disabled(grid.items.isEmpty)
        }
        .font(.callout)
        .padding(.horizontal, 12)
        .padding(.vertical, 8)
    }

    private func rebuild(_ proxy: ScrollViewProxy? = nil) {
        var groups: [(title: String, emoji: [Emoji])] = []
        if !query.trimmingCharacters(in: .whitespaces).isEmpty {
            groups = [("Results", EmojiCatalog.search(query, in: category))]
        } else {
            if category == nil {
                let recent = RecentEmoji.load().map {
                    EmojiCatalog.emoji(for: $0) ?? Emoji(character: $0, name: "")
                }
                if !recent.isEmpty { groups.append(("Recently Used", recent)) }
            }
            for item in EmojiCatalog.categories where category == nil || item.name == category {
                groups.append((item.name, item.emoji))
            }
        }
        grid = EmojiGrid(groups: groups, columns: Self.columns)
        selection = 0
        if let first = grid.rows.first { proxy?.scrollTo(first.id, anchor: .top) }
    }

    private func handle(_ command: PickerSearchField.Command, _ proxy: ScrollViewProxy) {
        switch command {
        case .move(let direction):
            guard !grid.items.isEmpty else { return }
            selection = grid.index(movingFrom: selection, direction)
            proxy.scrollTo(grid.rows[grid.row(of: selection)].id)
        case .submit:
            if grid.items.indices.contains(selection) { choose(grid.items[selection].character) }
        case .cancel:
            if query.isEmpty { dismiss() } else { query = "" }
        }
    }

    private func choose(_ character: String) {
        RecentEmoji.add(character)
        onChoose(character)
        dismiss()
    }
}

enum MoveDirection {
    case up, down, left, right
}

/// The picker's emoji laid out in sections of fixed-width rows, indexed in reading order.
private struct EmojiGrid {
    struct Row: Identifiable {
        let id: String
        /// Index of the row's first emoji in `items`.
        let start: Int
        let count: Int
    }

    struct Section: Identifiable {
        let id: String
        let title: String
        let rows: [Row]
    }

    private(set) var sections: [Section] = []
    private(set) var items: [Emoji] = []
    private(set) var rows: [Row] = []

    init() {}

    init(groups: [(title: String, emoji: [Emoji])], columns: Int) {
        for group in groups where !group.emoji.isEmpty {
            var sectionRows: [Row] = []
            for start in stride(from: 0, to: group.emoji.count, by: columns) {
                let row = Row(
                    id: "\(group.title)#\(start)",
                    start: items.count + start,
                    count: min(columns, group.emoji.count - start)
                )
                sectionRows.append(row)
            }
            items += group.emoji
            rows += sectionRows
            sections.append(Section(id: group.title, title: group.title, rows: sectionRows))
        }
    }

    func row(of index: Int) -> Int {
        rows.lastIndex { $0.start <= index } ?? 0
    }

    /// Left and right step through the emoji in order; up and down keep the column,
    /// or take the last one in a shorter row.
    func index(movingFrom index: Int, _ direction: MoveDirection) -> Int {
        switch direction {
        case .left:
            return max(index - 1, 0)
        case .right:
            return min(index + 1, items.count - 1)
        case .up, .down:
            let current = row(of: index)
            let target = direction == .up ? current - 1 : current + 1
            guard rows.indices.contains(target) else { return index }
            let column = index - rows[current].start
            return rows[target].start + min(column, rows[target].count - 1)
        }
    }
}

/// Tells pointer movement apart from content scrolling under a still pointer.
private final class PointerTracker {
    private var last = NSPoint.zero

    func reset() { last = NSEvent.mouseLocation }

    func moved() -> Bool {
        let location = NSEvent.mouseLocation
        defer { last = location }
        return location != last
    }
}

/// A borderless search field that passes arrow keys, Return and Escape to the picker
/// instead of moving the cursor.
private struct PickerSearchField: NSViewRepresentable {
    enum Command {
        case move(MoveDirection)
        case submit
        case cancel
    }

    @Binding var text: String
    let placeholder: String
    let onCommand: (Command) -> Void

    func makeNSView(context: Context) -> NSTextField {
        let field = FocusingTextField()
        field.isBordered = false
        field.drawsBackground = false
        field.focusRingType = .none
        field.font = .systemFont(ofSize: 15)
        field.placeholderString = placeholder
        field.usesSingleLineMode = true
        field.cell?.lineBreakMode = .byTruncatingTail
        field.delegate = context.coordinator
        return field
    }

    func updateNSView(_ field: NSTextField, context: Context) {
        context.coordinator.parent = self
        if field.stringValue != text { field.stringValue = text }
    }

    func makeCoordinator() -> Coordinator { Coordinator(parent: self) }

    final class Coordinator: NSObject, NSTextFieldDelegate {
        var parent: PickerSearchField

        init(parent: PickerSearchField) { self.parent = parent }

        func controlTextDidChange(_ notification: Notification) {
            guard let field = notification.object as? NSTextField else { return }
            parent.text = field.stringValue
        }

        func control(_ control: NSControl, textView: NSTextView, doCommandBy selector: Selector) -> Bool {
            let command: Command? = switch selector {
            case #selector(NSResponder.moveUp(_:)): .move(.up)
            case #selector(NSResponder.moveDown(_:)): .move(.down)
            case #selector(NSResponder.moveLeft(_:)): .move(.left)
            case #selector(NSResponder.moveRight(_:)): .move(.right)
            case #selector(NSResponder.insertNewline(_:)): .submit
            case #selector(NSResponder.cancelOperation(_:)): .cancel
            default: nil
            }
            guard let command else { return false }
            parent.onCommand(command)
            return true
        }
    }

    /// Takes keyboard focus as soon as it's shown.
    private final class FocusingTextField: NSTextField {
        override func viewDidMoveToWindow() {
            super.viewDidMoveToWindow()
            DispatchQueue.main.async { [weak self] in
                guard let self, let window = self.window else { return }
                window.makeFirstResponder(self)
            }
        }
    }
}
