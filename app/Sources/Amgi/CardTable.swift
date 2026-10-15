import AmgiCore
import AppKit
import SwiftUI

/// The browse table, in AppKit. SwiftUI's `Table` diffs its rows on every change, and
/// when thousands of cards come or go at once (a deck's subdeck back to the deck, or a
/// search cleared) it builds a view for each of them and freezes for seconds. This one
/// reloads instead, which costs the same for ten cards or ten thousand.
struct CardTable: NSViewRepresentable {
    let cards: [BrowseCard]
    /// Lowercased tag → icon.
    let icons: [String: String]
    @Binding var selection: Set<BrowseCard.ID>
    let open: (BrowseCard) -> Void

    func makeCoordinator() -> Coordinator { Coordinator(self) }

    func makeNSView(context: Context) -> NSScrollView {
        let table = NSTableView()
        table.style = .inset
        table.usesAlternatingRowBackgroundColors = true
        table.allowsMultipleSelection = true
        table.rowHeight = 24
        table.columnAutoresizingStyle = .uniformColumnAutoresizingStyle
        for column in Column.allCases {
            table.addTableColumn(column.tableColumn())
        }
        table.autosaveName = "BrowseTable"
        table.autosaveTableColumns = true

        let coordinator = context.coordinator
        table.dataSource = coordinator
        table.delegate = coordinator
        table.target = coordinator
        table.doubleAction = #selector(Coordinator.openClicked)
        let menu = NSMenu()
        menu.delegate = coordinator
        table.menu = menu
        coordinator.table = table

        let scroll = NSScrollView()
        scroll.documentView = table
        scroll.hasVerticalScroller = true
        return scroll
    }

    func updateNSView(_ scroll: NSScrollView, context: Context) {
        context.coordinator.parent = self
        context.coordinator.update()
    }

    enum Column: String, CaseIterable {
        case question, answer, deck, tags, due, reviews, lapses

        func tableColumn() -> NSTableColumn {
            let column = NSTableColumn(identifier: NSUserInterfaceItemIdentifier(rawValue))
            let (title, width, minWidth): (String, CGFloat, CGFloat) = switch self {
            case .question: ("Question", 260, 80)
            case .answer: ("Answer", 220, 80)
            case .deck: ("Deck", 100, 50)
            case .tags: ("Tags", 110, 40)
            case .due: ("Due", 90, 60)
            case .reviews: ("Reviews", 56, 40)
            case .lapses: ("Lapses", 52, 40)
            }
            column.title = title
            column.width = width
            column.minWidth = minWidth
            if self != .tags {
                column.sortDescriptorPrototype = NSSortDescriptor(key: rawValue, ascending: true)
            }
            if [.reviews, .lapses].contains(self) {
                column.headerCell.alignment = .right
            }
            return column
        }
    }

    @MainActor
    final class Coordinator: NSObject, NSTableViewDataSource, NSTableViewDelegate, NSMenuDelegate {
        var parent: CardTable
        weak var table: NSTableView?
        /// Cards as given, and as shown after sorting.
        private var source: [BrowseCard] = []
        private var icons: [String: String] = [:]
        private var rows: [BrowseCard] = []
        /// Set while the selection is changed from SwiftUI, so it isn't reported back.
        private var applyingSelection = false

        private static let dueFormat = Date.FormatStyle.dateTime.day().month().year()

        init(_ parent: CardTable) {
            self.parent = parent
        }

        func update() {
            guard let table else { return }
            if parent.cards != source || parent.icons != icons {
                source = parent.cards
                icons = parent.icons
                rows = sorted(source, by: table.sortDescriptors)
                table.reloadData()
            }
            applySelection()
        }

        private func applySelection() {
            guard let table else { return }
            let shown = Set(table.selectedRowIndexes.map { rows[$0].id })
            guard shown != parent.selection else { return }
            applyingSelection = true
            table.selectRowIndexes(IndexSet(rows.indices.filter { parent.selection.contains(rows[$0].id) }), byExtendingSelection: false)
            applyingSelection = false
        }

        private func sorted(_ cards: [BrowseCard], by descriptors: [NSSortDescriptor]) -> [BrowseCard] {
            guard let descriptor = descriptors.first, let key = descriptor.key.flatMap(Column.init) else { return cards }
            let ascending = descriptor.ascending
            func order(_ a: BrowseCard, _ b: BrowseCard) -> ComparisonResult {
                func compare<T: Comparable>(_ x: T, _ y: T) -> ComparisonResult {
                    x < y ? .orderedAscending : x > y ? .orderedDescending : .orderedSame
                }
                return switch key {
                case .question: a.front.localizedStandardCompare(b.front)
                case .answer: a.back.localizedStandardCompare(b.back)
                case .deck: a.deck.localizedStandardCompare(b.deck)
                case .due: compare(a.dueOrder, b.dueOrder)
                case .reviews: compare(a.reps, b.reps)
                case .lapses: compare(a.lapses, b.lapses)
                case .tags: .orderedSame
                }
            }
            // Ties keep file order.
            return cards.enumerated().sorted { a, b in
                switch order(a.element, b.element) {
                case .orderedSame: a.offset < b.offset
                case .orderedAscending: ascending
                case .orderedDescending: !ascending
                }
            }.map(\.element)
        }

        // MARK: Data source and delegate

        func numberOfRows(in tableView: NSTableView) -> Int { rows.count }

        func tableView(_ tableView: NSTableView, sortDescriptorsDidChange oldDescriptors: [NSSortDescriptor]) {
            rows = sorted(source, by: tableView.sortDescriptors)
            tableView.reloadData()
            applySelection()
        }

        func tableViewSelectionDidChange(_ notification: Notification) {
            guard !applyingSelection, let table else { return }
            let ids = Set(table.selectedRowIndexes.map { rows[$0].id })
            // After the table's own update, so SwiftUI doesn't re-enter it.
            DispatchQueue.main.async { [parent] in
                if parent.selection != ids { parent.selection = ids }
            }
        }

        func tableView(_ tableView: NSTableView, viewFor tableColumn: NSTableColumn?, row: Int) -> NSView? {
            guard let tableColumn, let column = Column(rawValue: tableColumn.identifier.rawValue) else { return nil }
            let card = rows[row]
            if column == .tags {
                let view = tableView.makeView(withIdentifier: tableColumn.identifier, owner: nil) as? TagPillsCell
                    ?? TagPillsCell(identifier: tableColumn.identifier)
                view.pills = card.tags.map { tag in [icons[tag.lowercased()], tag].compactMap(\.self).joined(separator: " ") }
                return view
            }
            let cell = tableView.makeView(withIdentifier: tableColumn.identifier, owner: nil) as? NSTableCellView
                ?? TextCell(identifier: tableColumn.identifier)
            guard let field = cell.textField else { return cell }
            field.alignment = .left
            field.toolTip = nil
            field.textColor = .labelColor
            switch column {
            case .question:
                field.attributedStringValue = InlineMarkdown.render(card.front, color: .labelColor)
            case .answer:
                field.attributedStringValue = InlineMarkdown.render(card.back, color: .secondaryLabelColor)
            case .deck:
                field.stringValue = deckTitle(card.deck)
                field.toolTip = card.deck
            case .due:
                if let due = card.due {
                    field.stringValue = Date(timeIntervalSince1970: TimeInterval(due)).formatted(Self.dueFormat)
                } else {
                    field.stringValue = "New"
                    field.textColor = .tertiaryLabelColor
                }
            case .reviews:
                field.stringValue = "\(card.reps)"
                field.alignment = .right
            case .lapses:
                field.stringValue = "\(card.lapses)"
                field.alignment = .right
            case .tags:
                break
            }
            return cell
        }

        // MARK: Opening cards

        @objc func openClicked() {
            guard let table, rows.indices.contains(table.clickedRow) else { return }
            parent.open(rows[table.clickedRow])
        }

        func menuNeedsUpdate(_ menu: NSMenu) {
            menu.removeAllItems()
            guard let table, rows.indices.contains(table.clickedRow) else { return }
            // Right-clicking a row outside the selection acts on that row alone.
            let selected = table.selectedRowIndexes
            let targets = selected.contains(table.clickedRow) ? Array(selected) : [table.clickedRow]
            guard targets.count == 1 else { return }
            let item = NSMenuItem(title: "Edit Card", action: #selector(openFromMenu(_:)), keyEquivalent: "")
            item.target = self
            item.representedObject = rows[targets[0]].id
            menu.addItem(item)
        }

        @objc private func openFromMenu(_ item: NSMenuItem) {
            guard let id = item.representedObject as? String, let card = rows.first(where: { $0.id == id }) else { return }
            parent.open(card)
        }
    }
}

/// A one-line label cell, truncated at the end.
private final class TextCell: NSTableCellView {
    init(identifier: NSUserInterfaceItemIdentifier) {
        super.init(frame: .zero)
        self.identifier = identifier
        let field = NSTextField(labelWithString: "")
        field.lineBreakMode = .byTruncatingTail
        field.cell?.truncatesLastVisibleLine = true
        field.font = .monospacedDigitSystemFont(ofSize: NSFont.systemFontSize, weight: .regular)
        field.translatesAutoresizingMaskIntoConstraints = false
        field.setContentCompressionResistancePriority(.defaultLow, for: .horizontal)
        addSubview(field)
        textField = field
        NSLayoutConstraint.activate([
            field.leadingAnchor.constraint(equalTo: leadingAnchor, constant: 2),
            field.trailingAnchor.constraint(equalTo: trailingAnchor, constant: -2),
            field.centerYAnchor.constraint(equalTo: centerYAnchor),
        ])
    }

    required init?(coder: NSCoder) { fatalError("not used") }
}

/// Tags drawn as capsules, like `TagPills`, without a view per tag.
private final class TagPillsCell: NSView {
    var pills: [String] = [] {
        didSet { if pills != oldValue { needsDisplay = true } }
    }

    private static let font = NSFont.systemFont(ofSize: NSFont.systemFontSize)

    init(identifier: NSUserInterfaceItemIdentifier) {
        super.init(frame: .zero)
        self.identifier = identifier
    }

    required init?(coder: NSCoder) { fatalError("not used") }

    override func draw(_ dirtyRect: NSRect) {
        let attributes: [NSAttributedString.Key: Any] = [.font: Self.font, .foregroundColor: NSColor.secondaryLabelColor]
        var x: CGFloat = 2
        for pill in pills {
            let text = NSAttributedString(string: pill, attributes: attributes)
            let size = text.size()
            let rect = NSRect(x: x, y: (bounds.height - size.height - 2) / 2, width: size.width + 14, height: size.height + 2)
            guard rect.maxX <= bounds.width else { break }
            NSColor.quaternaryLabelColor.setFill()
            NSBezierPath(roundedRect: rect, xRadius: rect.height / 2, yRadius: rect.height / 2).fill()
            text.draw(at: NSPoint(x: rect.minX + 7, y: rect.minY + 1))
            x = rect.maxX + 4
        }
    }
}

/// Card text on one line with inline Markdown rendered, remembered per source text:
/// cells are refilled as rows scroll into view, and parsing is the slow part.
@MainActor
private enum InlineMarkdown {
    private static var cache: [String: NSAttributedString] = [:]

    static func render(_ source: String, color: NSColor) -> NSAttributedString {
        let key = "\(color.hash)\u{0}\(source)"
        if let cached = cache[key] { return cached }
        let line = source.split(whereSeparator: \.isNewline).joined(separator: " ")
        let options = AttributedString.MarkdownParsingOptions(interpretedSyntax: .inlineOnlyPreservingWhitespace)
        let parsed = (try? AttributedString(markdown: line, options: options)) ?? AttributedString(line)

        let base = NSFont.systemFont(ofSize: NSFont.systemFontSize)
        let paragraph = NSMutableParagraphStyle()
        paragraph.lineBreakMode = .byTruncatingTail
        let text = NSMutableAttributedString()
        for run in parsed.runs {
            let intent = run.inlinePresentationIntent ?? []
            var font = intent.contains(.code) ? NSFont.monospacedSystemFont(ofSize: base.pointSize, weight: .regular) : base
            var traits: NSFontDescriptor.SymbolicTraits = []
            if intent.contains(.stronglyEmphasized) { traits.insert(.bold) }
            if intent.contains(.emphasized) { traits.insert(.italic) }
            if !traits.isEmpty {
                font = NSFont(descriptor: font.fontDescriptor.withSymbolicTraits(traits), size: 0) ?? font
            }
            var attributes: [NSAttributedString.Key: Any] = [.font: font, .foregroundColor: color, .paragraphStyle: paragraph]
            if intent.contains(.strikethrough) {
                attributes[.strikethroughStyle] = NSUnderlineStyle.single.rawValue
            }
            text.append(NSAttributedString(string: String(parsed[run.range].characters), attributes: attributes))
        }
        // Bounded by the text of the cards browsed; start over rather than grow without end.
        if cache.count > 20_000 { cache.removeAll() }
        cache[key] = text
        return text
    }
}
