import AmgiCore
import AppKit

/// Something the command palette can do: run an action, or go to a deck, tag or view.
struct PaletteCommand {
    let title: String
    /// What kind of thing it is, shown at the right: "Deck", "Tag", "Study"…
    let kind: String
    /// An SF Symbol, or an emoji when the tag or view has one.
    let icon: Icon
    /// The keys that do the same, shown as a hint.
    let shortcut: String?
    let action: @MainActor () -> Void

    enum Icon {
        case symbol(String)
        case emoji(String)
    }

    init(_ title: String, kind: String, icon: Icon, shortcut: String? = nil, action: @escaping @MainActor () -> Void) {
        self.title = title
        self.kind = kind
        self.icon = icon
        self.shortcut = shortcut
        self.action = action
    }

    init(_ title: String, kind: String, symbol: String, shortcut: String? = nil, action: @escaping @MainActor () -> Void) {
        self.init(title, kind: kind, icon: .symbol(symbol), shortcut: shortcut, action: action)
    }

    /// How well `query` matches, higher being better; nil if it doesn't. Every word of
    /// the query has to be found in the title or kind.
    func score(_ query: String) -> Int? {
        let title = Self.fold(title)
        let kind = Self.fold(kind)
        var total = 0
        for word in Self.fold(query).split(whereSeparator: \.isWhitespace) {
            let word = String(word)
            if let score = Self.score(word, in: title) {
                total += score
            } else if let score = Self.score(word, in: kind) {
                total += score / 4
            } else {
                return nil
            }
        }
        // Shorter titles first among equal matches.
        return total * 100 - title.count
    }

    private static func fold(_ text: String) -> String {
        text.folding(options: [.caseInsensitive, .diacriticInsensitive], locale: nil)
    }

    /// A prefix beats a word's start, which beats any substring, which beats letters in order.
    private static func score(_ word: String, in text: String) -> Int? {
        if text.hasPrefix(word) { return 100 }
        if let range = text.range(of: word) {
            let before = text[text.index(before: range.lowerBound)]
            return before.isLetter || before.isNumber ? 50 : 80
        }
        // Letters in order, such as "ss" for Show Sidebar; runs and word starts count more.
        var score = 0
        var index = text.startIndex
        var previous: String.Index?
        for letter in word {
            guard let found = text[index...].firstIndex(of: letter) else { return nil }
            if let previous, text.index(after: previous) == found { score += 3 }
            if found == text.startIndex || !text[text.index(before: found)].isLetter { score += 4 }
            previous = found
            index = text.index(after: found)
        }
        return min(score, 40)
    }
}

/// The command palette, opened with ⌘K: type to find any command, deck, tag or saved
/// view and press Return to run or open it. A floating sheet of Liquid Glass over the
/// window it was opened from.
@MainActor
final class CommandPalette: NSObject, NSWindowDelegate, NSTextFieldDelegate, NSTableViewDataSource, NSTableViewDelegate {
    private static let width: CGFloat = 580
    private static let fieldHeight: CGFloat = 52
    private static let rowHeight: CGFloat = 34
    private static let visibleRows = 9
    private static let listPadding: CGFloat = 6

    private var panel: PalettePanel?
    private let field = NSTextField()
    private let table = NSTableView()
    private let scroll = NSScrollView()
    private let separator = NSBox()
    private let empty = NSTextField(labelWithString: "No Results")

    private var commands: [PaletteCommand] = []
    private var results: [PaletteCommand] = []

    var isShown: Bool { panel?.isVisible == true }

    func show(_ commands: [PaletteCommand]) {
        self.commands = commands
        let panel = self.panel ?? makePanel()
        self.panel = panel
        field.stringValue = ""
        filter()
        attach(panel, to: NSApp.keyWindow ?? NSApp.mainWindow)
        panel.makeKeyAndOrderFront(nil)
        panel.makeFirstResponder(field)
    }

    func close() {
        guard let panel, panel.isVisible else { return }
        let parent = panel.parent
        parent?.removeChildWindow(panel)
        panel.orderOut(nil)
        parent?.makeKey()
    }

    // MARK: Layout

    private func makePanel() -> PalettePanel {
        let panel = PalettePanel(
            contentRect: NSRect(x: 0, y: 0, width: Self.width, height: Self.fieldHeight),
            styleMask: [.borderless, .fullSizeContentView],
            backing: .buffered,
            defer: true
        )
        panel.isOpaque = false
        panel.backgroundColor = .clear
        panel.hasShadow = true
        panel.isReleasedWhenClosed = false
        panel.hidesOnDeactivate = true
        panel.animationBehavior = .utilityWindow
        panel.delegate = self
        panel.onCancel = { [weak self] in self?.close() }
        panel.setAccessibilitySubrole(.dialog)
        panel.setAccessibilityLabel("Command Palette")

        let glass = NSGlassEffectView()
        glass.cornerRadius = 22
        let content = NSView()
        glass.contentView = content
        panel.contentView = glass

        let magnifier = NSImageView(image: NSImage(systemSymbolName: "magnifyingglass", accessibilityDescription: nil)!)
        magnifier.symbolConfiguration = .init(pointSize: 17, weight: .medium)
        magnifier.contentTintColor = .secondaryLabelColor

        field.isBordered = false
        field.drawsBackground = false
        field.focusRingType = .none
        field.font = .systemFont(ofSize: 20)
        field.placeholderString = "Search commands, decks, tags and views"
        field.cell?.isScrollable = true
        field.cell?.lineBreakMode = .byTruncatingTail
        field.delegate = self
        field.setAccessibilityLabel("Search commands")

        separator.boxType = .separator

        let column = NSTableColumn(identifier: .init("command"))
        table.addTableColumn(column)
        table.headerView = nil
        table.style = .plain
        table.backgroundColor = .clear
        table.intercellSpacing = .zero
        table.rowHeight = Self.rowHeight
        table.selectionHighlightStyle = .regular
        // The field keeps focus; the list follows its arrow keys.
        table.refusesFirstResponder = true
        table.dataSource = self
        table.delegate = self
        table.target = self
        table.action = #selector(clicked)

        scroll.documentView = table
        scroll.drawsBackground = false
        scroll.hasVerticalScroller = true
        scroll.autohidesScrollers = true
        scroll.contentInsets = NSEdgeInsets(top: Self.listPadding, left: 0, bottom: Self.listPadding, right: 0)
        scroll.automaticallyAdjustsContentInsets = false

        empty.textColor = .secondaryLabelColor
        empty.alignment = .center

        for view in [magnifier, field, separator, scroll, empty] as [NSView] {
            view.translatesAutoresizingMaskIntoConstraints = false
            content.addSubview(view)
        }
        NSLayoutConstraint.activate([
            magnifier.leadingAnchor.constraint(equalTo: content.leadingAnchor, constant: 18),
            magnifier.centerYAnchor.constraint(equalTo: content.topAnchor, constant: Self.fieldHeight / 2),
            field.leadingAnchor.constraint(equalTo: magnifier.trailingAnchor, constant: 10),
            field.trailingAnchor.constraint(equalTo: content.trailingAnchor, constant: -18),
            field.centerYAnchor.constraint(equalTo: magnifier.centerYAnchor),
            separator.topAnchor.constraint(equalTo: content.topAnchor, constant: Self.fieldHeight),
            separator.leadingAnchor.constraint(equalTo: content.leadingAnchor),
            separator.trailingAnchor.constraint(equalTo: content.trailingAnchor),
            scroll.topAnchor.constraint(equalTo: separator.bottomAnchor),
            scroll.leadingAnchor.constraint(equalTo: content.leadingAnchor),
            scroll.trailingAnchor.constraint(equalTo: content.trailingAnchor),
            scroll.bottomAnchor.constraint(equalTo: content.bottomAnchor),
            empty.centerXAnchor.constraint(equalTo: scroll.centerXAnchor),
            empty.centerYAnchor.constraint(equalTo: scroll.centerYAnchor),
        ])
        return panel
    }

    /// Centered over the window, a fifth of the way down, and moving with it. Without a
    /// window, near the top of the screen with the pointer, like Spotlight.
    private func attach(_ panel: PalettePanel, to window: NSWindow?) {
        let width = Self.width
        let height = panel.frame.height
        if let window, window !== panel, window.isVisible {
            let frame = window.frame
            let top = frame.maxY - min(120, frame.height * 0.2)
            panel.setFrame(NSRect(x: frame.midX - width / 2, y: top - height, width: width, height: height), display: false)
            window.addChildWindow(panel, ordered: .above)
        } else {
            let mouse = NSEvent.mouseLocation
            guard let screen = NSScreen.screens.first(where: { $0.frame.contains(mouse) }) ?? NSScreen.main else { return }
            let area = screen.visibleFrame
            let top = area.maxY - area.height * 0.2
            panel.setFrame(NSRect(x: area.midX - width / 2, y: top - height, width: width, height: height), display: false)
        }
    }

    /// Fits the panel to the results, keeping its top edge where it is.
    private func resize() {
        guard let panel else { return }
        let rows = min(results.count, Self.visibleRows)
        let list: CGFloat
        if results.isEmpty {
            list = field.stringValue.isEmpty ? 0 : 44
        } else {
            list = CGFloat(rows) * Self.rowHeight + Self.listPadding * 2 + 1
        }
        separator.isHidden = list == 0
        empty.isHidden = !results.isEmpty || list == 0
        var frame = panel.frame
        let height = Self.fieldHeight + list
        frame.origin.y += frame.height - height
        frame.size.height = height
        panel.setFrame(frame, display: true)
        panel.invalidateShadow()
    }

    // MARK: Results

    private func filter() {
        let query = field.stringValue.trimmingCharacters(in: .whitespaces)
        if query.isEmpty {
            results = commands
        } else {
            results = commands.enumerated()
                .compactMap { index, command in command.score(query).map { (command, $0, index) } }
                .sorted { $0.1 != $1.1 ? $0.1 > $1.1 : $0.2 < $1.2 }
                .map(\.0)
        }
        table.reloadData()
        resize()
        select(results.isEmpty ? -1 : 0)
    }

    private func select(_ row: Int) {
        guard row >= 0, row < results.count else {
            table.deselectAll(nil)
            return
        }
        table.selectRowIndexes([row], byExtendingSelection: false)
        table.scrollRowToVisible(row)
    }

    private func move(by offset: Int) {
        guard !results.isEmpty else { return }
        let row = table.selectedRow < 0 ? 0 : (table.selectedRow + offset + results.count) % results.count
        select(row)
    }

    private func run(_ row: Int) {
        guard row >= 0, row < results.count else { return }
        let action = results[row].action
        close()
        // After the palette is gone, so focus is back in the window and a modal panel
        // such as Open Folder doesn't start inside this event.
        DispatchQueue.main.async { action() }
    }

    @objc private func clicked() {
        run(table.clickedRow)
    }

    // MARK: Field

    func controlTextDidChange(_ notification: Notification) {
        filter()
    }

    func control(_ control: NSControl, textView: NSTextView, doCommandBy selector: Selector) -> Bool {
        switch selector {
        case #selector(NSResponder.moveDown(_:)): move(by: 1)
        case #selector(NSResponder.moveUp(_:)): move(by: -1)
        case #selector(NSResponder.insertNewline(_:)): run(table.selectedRow)
        case #selector(NSResponder.cancelOperation(_:)): close()
        default: return false
        }
        return true
    }

    // MARK: Table

    func numberOfRows(in tableView: NSTableView) -> Int {
        results.count
    }

    func tableView(_ tableView: NSTableView, viewFor tableColumn: NSTableColumn?, row: Int) -> NSView? {
        let cell = tableView.makeView(withIdentifier: PaletteCell.identifier, owner: nil) as? PaletteCell ?? PaletteCell()
        cell.show(results[row])
        return cell
    }

    func tableView(_ tableView: NSTableView, rowViewForRow row: Int) -> NSTableRowView? {
        PaletteRowView()
    }

    // MARK: Window

    /// Clicking elsewhere or switching apps puts the palette away.
    func windowDidResignKey(_ notification: Notification) {
        close()
    }
}

/// A borderless panel that takes typing, and closes on Escape.
private final class PalettePanel: NSPanel {
    var onCancel: () -> Void = {}

    override var canBecomeKey: Bool { true }
    override var canBecomeMain: Bool { false }

    override func cancelOperation(_ sender: Any?) {
        onCancel()
    }
}

/// The selected row: a rounded rectangle in the accent color, inset from the glass's edges.
private final class PaletteRowView: NSTableRowView {
    // Selected rows read as active though the field, not the list, has focus.
    override var isEmphasized: Bool {
        get { true }
        set {}
    }

    override func drawSelection(in dirtyRect: NSRect) {
        NSColor.selectedContentBackgroundColor.setFill()
        NSBezierPath(roundedRect: bounds.insetBy(dx: 8, dy: 1), xRadius: 8, yRadius: 8).fill()
    }
}

/// Icon, title, then the kind and shortcut in secondary text at the right.
private final class PaletteCell: NSTableCellView {
    static let identifier = NSUserInterfaceItemIdentifier("PaletteCell")

    private let icon = NSImageView()
    private let emoji = NSTextField(labelWithString: "")
    private let title = NSTextField(labelWithString: "")
    private let kind = NSTextField(labelWithString: "")
    private let shortcut = NSTextField(labelWithString: "")

    init() {
        super.init(frame: .zero)
        identifier = Self.identifier
        icon.symbolConfiguration = .init(pointSize: 14, weight: .regular)
        emoji.font = .systemFont(ofSize: 14)
        emoji.alignment = .center
        title.font = .systemFont(ofSize: 14)
        title.lineBreakMode = .byTruncatingTail
        title.setContentCompressionResistancePriority(.defaultLow, for: .horizontal)
        kind.font = .systemFont(ofSize: 12)
        shortcut.font = .monospacedSystemFont(ofSize: 12, weight: .regular)
        for label in [kind, shortcut] {
            label.setContentCompressionResistancePriority(.required, for: .horizontal)
            label.setContentHuggingPriority(.required, for: .horizontal)
        }
        for view in [icon, emoji, title, kind, shortcut] as [NSView] {
            view.translatesAutoresizingMaskIntoConstraints = false
            addSubview(view)
        }
        NSLayoutConstraint.activate([
            icon.leadingAnchor.constraint(equalTo: leadingAnchor, constant: 22),
            icon.centerYAnchor.constraint(equalTo: centerYAnchor),
            icon.widthAnchor.constraint(equalToConstant: 20),
            emoji.centerXAnchor.constraint(equalTo: icon.centerXAnchor),
            emoji.centerYAnchor.constraint(equalTo: centerYAnchor),
            title.leadingAnchor.constraint(equalTo: icon.trailingAnchor, constant: 10),
            title.centerYAnchor.constraint(equalTo: centerYAnchor),
            kind.leadingAnchor.constraint(greaterThanOrEqualTo: title.trailingAnchor, constant: 12),
            kind.centerYAnchor.constraint(equalTo: centerYAnchor),
            shortcut.leadingAnchor.constraint(equalTo: kind.trailingAnchor, constant: 10),
            shortcut.trailingAnchor.constraint(equalTo: trailingAnchor, constant: -22),
            shortcut.centerYAnchor.constraint(equalTo: centerYAnchor),
        ])
        textField = title
    }

    required init?(coder: NSCoder) { fatalError("not used") }

    func show(_ command: PaletteCommand) {
        switch command.icon {
        case .symbol(let name):
            icon.image = NSImage(systemSymbolName: name, accessibilityDescription: nil)
            icon.isHidden = false
            emoji.isHidden = true
        case .emoji(let text):
            emoji.stringValue = text
            icon.isHidden = true
            emoji.isHidden = false
        }
        title.stringValue = command.title
        kind.stringValue = command.kind
        shortcut.stringValue = command.shortcut ?? ""
        shortcut.isHidden = command.shortcut == nil
        setAccessibilityLabel([command.title, command.kind, command.shortcut].compactMap { $0 }.joined(separator: ", "))
        restyle()
    }

    /// White on the selected row, as in a menu.
    override var backgroundStyle: NSView.BackgroundStyle {
        didSet { restyle() }
    }

    private func restyle() {
        let selected = backgroundStyle == .emphasized
        title.textColor = selected ? .alternateSelectedControlTextColor : .labelColor
        let secondary: NSColor = selected ? .alternateSelectedControlTextColor.withAlphaComponent(0.75) : .secondaryLabelColor
        kind.textColor = secondary
        shortcut.textColor = secondary
        icon.contentTintColor = selected ? .alternateSelectedControlTextColor : .secondaryLabelColor
    }
}

// MARK: Commands

extension AppModel {
    /// What the palette offers now: what can be done where you are first, then going
    /// elsewhere, then every deck, tag and saved view.
    func paletteCommands() -> [PaletteCommand] {
        var commands: [PaletteCommand] = []
        let open = folder != nil

        if open, selection == .review, let card = current {
            if revealed {
                let keys = answerKeys.ordered
                for rating in Rating.allCases {
                    commands.append(PaletteCommand(rating.label, kind: "Study", symbol: rating.symbol, shortcut: keys[rating.index].uppercased()) { [weak self] in
                        self?.answer(rating, via: .key)
                    })
                }
            } else {
                commands.append(PaletteCommand("Show Answer", kind: "Study", symbol: "eye", shortcut: "Space") { [weak self] in
                    self?.reveal(via: .key)
                })
            }
            commands.append(PaletteCommand("Suspend Card", kind: "Study", symbol: "pause.circle", shortcut: "⌘S") { [weak self] in
                self?.suspendCurrent()
            })
            commands.append(PaletteCommand("Card Info", kind: "Study", symbol: "info.circle", shortcut: "⌘I") { [weak self] in
                self?.showCardInfo(id: card.id)
            })
        }
        if open, selection == .review, canUndoAnswer {
            commands.append(PaletteCommand("Undo Answer", kind: "Study", symbol: "arrow.uturn.backward", shortcut: "⌘Z") { [weak self] in
                self?.undoAnswer()
            })
        }
        if open, isBrowsing {
            if let card = selectedBrowseCard {
                commands.append(PaletteCommand("Edit Card", kind: "Browse", symbol: "square.and.pencil", shortcut: "↩") { [weak self] in
                    self?.revealInEditor(path: card.path)
                })
                commands.append(PaletteCommand("Card Info", kind: "Browse", symbol: "info.circle", shortcut: "⌘I") { [weak self] in
                    self?.showCardInfo(id: card.id)
                })
            }
            if !currentQuery.isEmpty {
                commands.append(PaletteCommand("Save Search as View…", kind: "Browse", symbol: "line.3.horizontal.decrease.circle") { [weak self] in
                    self?.saveCurrentSearch()
                })
            }
        }

        if open {
            commands.append(PaletteCommand("Add Card…", kind: "Command", symbol: "plus.rectangle.portrait", shortcut: "⌘N") { [weak self] in
                self?.toggleQuickAdd()
            })
            commands.append(PaletteCommand("Dashboard", kind: "Go To", symbol: "chart.bar.xaxis", shortcut: "⌘1") { [weak self] in
                self?.selection = .dashboard
            })
            commands.append(PaletteCommand("Review", kind: "Go To", symbol: "rectangle.stack", shortcut: "⌘2") { [weak self] in
                self?.selection = .review
            })
            commands.append(PaletteCommand("Decks", kind: "Go To", symbol: "rectangle.portrait.on.rectangle.portrait", shortcut: "⌘3") { [weak self] in
                self?.selection = .cards
            })
            if !tags.isEmpty {
                commands.append(PaletteCommand("Tags", kind: "Go To", symbol: "number") { [weak self] in
                    self?.selection = .tags
                })
            }
            if !savedViews.isEmpty {
                commands.append(PaletteCommand("Saved Views", kind: "Go To", symbol: "line.3.horizontal.decrease.circle") { [weak self] in
                    self?.selection = .views
                })
            }
            commands.append(PaletteCommand("Show or Hide Sidebar", kind: "View", symbol: "sidebar.left", shortcut: "⌃⌘S") {
                NSApp.sendAction(#selector(NSSplitViewController.toggleSidebar(_:)), to: nil, from: nil)
            })
            commands.append(PaletteCommand("Reload Cards", kind: "Command", symbol: "arrow.clockwise", shortcut: "⌘R") { [weak self] in
                self?.reload()
            })
        }
        commands.append(PaletteCommand("Open Folder…", kind: "Command", symbol: "folder", shortcut: "⌘O") { [weak self] in
            self?.chooseFolder()
        })
        if open, !importing {
            commands.append(PaletteCommand("Import Anki Deck…", kind: "Command", symbol: "square.and.arrow.down", shortcut: "⇧⌘I") { [weak self] in
                self?.chooseAnkiPackage()
            })
            commands.append(PaletteCommand("Close Folder", kind: "Command", symbol: "xmark.circle") { [weak self] in
                self?.closeFolder()
            })
        }
        commands.append(PaletteCommand("Settings…", kind: "Command", symbol: "gearshape", shortcut: "⌘,") { [weak self] in
            self?.openSettingsWindow()
        })
        commands.append(PaletteCommand("Keyboard Shortcuts", kind: "Help", symbol: "keyboard", shortcut: "⌘/") { [weak self] in
            self?.openShortcutsWindow()
        })

        guard open else { return commands }
        for deck in deckNames {
            let title = deck.title == deckTitle(deck.name) ? deck.name : "\(deck.title) (\(deck.name))"
            commands.append(PaletteCommand(title, kind: "Deck", symbol: "rectangle.portrait.on.rectangle.portrait") { [weak self] in
                self?.selection = .deck(deck.name)
            })
        }
        for tag in tags {
            let icon: PaletteCommand.Icon = icon(forTag: tag.name).map { .emoji($0) } ?? .symbol("number")
            commands.append(PaletteCommand("#\(tag.name)", kind: "Tag", icon: icon) { [weak self] in
                self?.selection = .tag(tag.name)
            })
        }
        for view in savedViews {
            let icon: PaletteCommand.Icon = view.icon.map { .emoji($0) } ?? .symbol("line.3.horizontal.decrease.circle")
            commands.append(PaletteCommand(view.name, kind: "Saved View", icon: icon) { [weak self] in
                self?.selection = .view(view.id)
            })
        }
        return commands
    }
}

private extension Rating {
    var symbol: String {
        switch self {
        case .again: "arrow.counterclockwise"
        case .hard: "tortoise"
        case .good: "checkmark"
        case .easy: "hare"
        }
    }
}
