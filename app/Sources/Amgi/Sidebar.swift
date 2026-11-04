import AmgiCore
import AppKit
import SwiftUI

/// The sidebar: room for the traffic lights at the top, then the list of places to go.
/// The list is an AppKit table, drawn plainly on the window's background.
struct Sidebar: View {
    @Environment(AppModel.self) private var model
    @State private var showingWarnings = false
    @State private var expandedDecks: Set<String> = []
    @AppStorage("cardsExpanded") private var cardsExpanded = true
    @AppStorage("decksExpanded") private var decksExpanded = true
    @AppStorage("tagsExpanded") private var tagsExpanded = true
    @AppStorage("viewsExpanded") private var viewsExpanded = true

    var body: some View {
        // The list starts where the title bar ends, just under the traffic lights, and
        // rows scrolled up fade out there. Its first row is a blank one, so at rest none
        // sit in the fade.
        ZStack(alignment: .top) {
            SidebarTable(model: model, rows: rows, selection: model.selection, onToggle: toggle)
                .mask {
                    VStack(spacing: 0) {
                        LinearGradient(colors: [.clear, .black], startPoint: .top, endPoint: .bottom)
                            .frame(height: sidebarFade)
                        Color.black
                    }
                }
                .padding(.top, titlebarDepth)
            HStack {
                Spacer()
                if !model.warnings.isEmpty {
                    Button {
                        showingWarnings.toggle()
                    } label: {
                        Image(systemName: "exclamationmark.triangle")
                            .font(.system(size: 14))
                            .foregroundStyle(.themeSecondaryText)
                            .frame(width: 28, height: 28)
                            .contentShape(.rect)
                    }
                    .buttonStyle(.plain)
                    .help("\(model.warnings.count) problems found while reading cards")
                    .popover(isPresented: $showingWarnings, arrowEdge: .bottom) { WarningList() }
                }
            }
            .padding(.trailing, 8)
            .frame(height: titlebarHeight)
            .windowDragArea()
        }
    }

    private var rows: [SidebarRow] {
        var rows = [
            SidebarRow.top,
            SidebarRow(id: "dashboard", title: "Dashboard", icon: .symbol("chart.bar.xaxis"), selection: .dashboard),
            SidebarRow(header: "Review", id: "h:review"),
            SidebarRow(
                id: "review",
                title: model.folder?.lastPathComponent ?? "All Cards",
                icon: .symbol("rectangle.stack"),
                badge: Int(model.totalDueAndLearning),
                selection: .review
            ),
        ]
        let hasCards = !model.deckNames.isEmpty || !model.savedViews.isEmpty || !model.tags.isEmpty
        guard hasCards else { return rows }
        rows.append(SidebarRow(header: "Cards", id: "h:cards", expanded: cardsExpanded))
        guard cardsExpanded else { return rows }
        if !model.deckNames.isEmpty {
            rows.append(SidebarRow(
                id: "decks", title: "Decks", icon: .symbol("rectangle.portrait.on.rectangle.portrait"),
                selection: .cards, expanded: decksExpanded
            ))
            if decksExpanded {
                func add(_ nodes: [DeckNode], depth: Int) {
                    for node in nodes {
                        let open = expandedDecks.contains(node.id)
                        rows.append(SidebarRow(
                            id: "deck:\(node.id)",
                            title: node.title,
                            icon: .symbol(node.children.isEmpty ? "rectangle.portrait.on.rectangle.portrait" : "folder"),
                            depth: depth,
                            selection: .deck(node.id),
                            expanded: node.children.isEmpty ? nil : open
                        ))
                        if open { add(node.children, depth: depth + 1) }
                    }
                }
                add(DeckNode.tree(decks: model.deckNames), depth: 1)
            }
        }
        if !model.savedViews.isEmpty {
            rows.append(SidebarRow(
                id: "views", title: "Saved Views", icon: .symbol("line.3.horizontal.decrease.circle"),
                selection: .views, expanded: viewsExpanded
            ))
            if viewsExpanded {
                for view in model.savedViews {
                    rows.append(SidebarRow(
                        id: "view:\(view.id)",
                        title: view.name,
                        icon: view.icon.map(SidebarRow.Icon.emoji) ?? .symbol("line.3.horizontal.decrease.circle"),
                        depth: 1,
                        selection: .view(view.id),
                        toolTip: view.query,
                        target: .view(view.id)
                    ))
                }
            }
        }
        if !model.tags.isEmpty {
            rows.append(SidebarRow(
                id: "tags", title: "Tags", icon: .symbol("number"), selection: .tags, expanded: tagsExpanded
            ))
            if tagsExpanded {
                for tag in model.tags {
                    rows.append(SidebarRow(
                        id: "tag:\(tag.name)",
                        title: tag.name,
                        icon: model.icon(forTag: tag.name).map(SidebarRow.Icon.emoji) ?? .symbol("number"),
                        depth: 1,
                        selection: .tag(tag.name),
                        target: .tag(tag.name)
                    ))
                }
            }
        }
        return rows
    }

    private func toggle(_ id: String) {
        switch id {
        case "h:cards": cardsExpanded.toggle()
        case "decks": decksExpanded.toggle()
        case "views": viewsExpanded.toggle()
        case "tags": tagsExpanded.toggle()
        default:
            guard id.hasPrefix("deck:") else { return }
            let deck = String(id.dropFirst("deck:".count))
            if expandedDecks.contains(deck) { expandedDecks.remove(deck) } else { expandedDecks.insert(deck) }
        }
    }
}

/// How far down the title bar reaches: just past the traffic lights.
let titlebarDepth: CGFloat = 36

/// The room between the title bar and the first row at rest.
private let sidebarTopRoom = titlebarHeight - titlebarDepth

/// How tall the fade is where sidebar rows scroll out of view, below the title bar.
private let sidebarFade: CGFloat = 10

/// How far down the window the traffic lights' row reaches. The sidebar and the detail
/// panel's header line up with it.
let titlebarHeight: CGFloat = 52

/// A line in the sidebar: a section header, or a place to go.
struct SidebarRow: Equatable {
    enum Icon: Equatable {
        case symbol(String)
        case emoji(String)
    }

    /// What the row's right-click menu acts on.
    enum Target: Equatable {
        case tag(String)
        case view(Int64)
    }

    /// Stays the same as the row's content changes, so the table can tell rows apart.
    var id: String
    var isHeader = false
    var title: String
    var icon: Icon?
    var badge: Int?
    var depth = 0
    var selection: SidebarItem?
    /// Whether the row's children show; nil when it has none.
    var expanded: Bool?
    var toolTip: String?
    var target: Target?

    init(
        id: String, title: String, icon: Icon, badge: Int? = nil, depth: Int = 0, selection: SidebarItem,
        expanded: Bool? = nil, toolTip: String? = nil, target: Target? = nil
    ) {
        self.id = id
        self.title = title
        self.icon = icon
        self.badge = badge
        self.depth = depth
        self.selection = selection
        self.expanded = expanded
        self.toolTip = toolTip
        self.target = target
    }

    /// The blank first row, which leaves room at the top.
    static let top = SidebarRow(header: "", id: "top")

    init(header: String, id: String, expanded: Bool? = nil) {
        self.id = id
        isHeader = true
        title = header
        self.expanded = expanded
    }
}

/// A deck in the sidebar: a deck file, a folder, or both (`lang.md` and `lang/`).
private struct DeckNode: Identifiable {
    let id: String
    let title: String
    let children: [DeckNode]

    /// The top-level decks.
    static func tree(decks: [DeckName]) -> [DeckNode] {
        // `decks` lists parents before children, so grouping keeps file order.
        let byParent = Dictionary(grouping: decks) { deck in
            deck.name.split(separator: "/").dropLast().joined(separator: "/")
        }
        func children(of parent: String) -> [DeckNode] {
            (byParent[parent] ?? []).map { deck in
                DeckNode(id: deck.name, title: deck.title, children: children(of: deck.name))
            }
        }
        return children(of: "")
    }
}

private struct SidebarTable: NSViewRepresentable {
    let model: AppModel
    let rows: [SidebarRow]
    let selection: SidebarItem
    let onToggle: (String) -> Void

    func makeCoordinator() -> SidebarController { SidebarController(model: model) }

    func makeNSView(context: Context) -> NSScrollView { context.coordinator.scrollView }

    func updateNSView(_ view: NSScrollView, context: Context) {
        context.coordinator.onToggle = onToggle
        context.coordinator.update(rows: rows, selection: selection)
    }
}

/// Runs the sidebar's table: fills it from the rows, keeps its selection in step with
/// the model's, and opens the right-click menus and icon pickers.
@MainActor
final class SidebarController: NSObject, NSTableViewDataSource, NSTableViewDelegate, NSMenuDelegate {
    let scrollView = NSScrollView()
    private let table = SidebarTableView()
    private weak var model: AppModel?
    private var rows: [SidebarRow] = []
    private var selection: SidebarItem?
    /// Set while the table's selection is changed from the model, so it isn't sent back.
    private var syncing = false
    private var iconPopover: NSPopover?
    var onToggle: (String) -> Void = { _ in }

    init(model: AppModel) {
        self.model = model
        super.init()
        let column = NSTableColumn(identifier: .init("row"))
        column.resizingMask = .autoresizingMask
        table.addTableColumn(column)
        table.headerView = nil
        table.style = .plain
        table.backgroundColor = .clear
        table.intercellSpacing = NSSize(width: 0, height: 2)
        table.columnAutoresizingStyle = .uniformColumnAutoresizingStyle
        table.focusRingType = .none
        table.allowsEmptySelection = true
        table.dataSource = self
        table.delegate = self
        table.target = self
        table.action = #selector(clicked)
        table.onArrow = { [weak self] open in self?.arrow(open: open) ?? false }
        let menu = NSMenu()
        menu.delegate = self
        table.menu = menu

        scrollView.documentView = table
        scrollView.drawsBackground = false
        scrollView.hasVerticalScroller = true
        scrollView.autohidesScrollers = true
        scrollView.scrollerStyle = .overlay
        // Room at the top comes from a blank first row: an inset here would have AppKit
        // draw a hard edge where it ends.
        scrollView.contentInsets = NSEdgeInsets(top: 0, left: 0, bottom: 10, right: 0)
        scrollView.automaticallyAdjustsContentInsets = false
    }

    func update(rows newRows: [SidebarRow], selection newSelection: SidebarItem) {
        let old = rows
        rows = newRows
        selection = newSelection
        if old.map(\.id) == newRows.map(\.id) {
            // Same rows: refresh the ones whose content changed, such as a count.
            for index in newRows.indices where newRows[index] != old[index] {
                configure(row: index)
            }
        } else if let change = Self.change(from: old.map(\.id), to: newRows.map(\.id)) {
            // A group opened or closed: slide its rows in or out.
            table.beginUpdates()
            if change.inserted {
                table.insertRows(at: IndexSet(change.range), withAnimation: .slideDown)
            } else {
                table.removeRows(at: IndexSet(change.range), withAnimation: .slideUp)
            }
            table.endUpdates()
            for index in newRows.indices { configure(row: index) }
        } else {
            table.reloadData()
        }
        syncSelection()
    }

    /// The rows added or removed in one run, when that's the only difference.
    private static func change(from old: [String], to new: [String]) -> (inserted: Bool, range: Range<Int>)? {
        let (short, long, inserted) = old.count < new.count ? (old, new, true) : (new, old, false)
        guard short.count < long.count else { return nil }
        let prefix = zip(short, long).prefix { $0 == $1 }.count
        let count = long.count - short.count
        guard Array(long[(prefix + count)...]) == Array(short[prefix...]) else { return nil }
        return (inserted, prefix..<(prefix + count))
    }

    private func configure(row index: Int) {
        guard let cell = table.view(atColumn: 0, row: index, makeIfNecessary: false) as? SidebarCell else { return }
        cell.show(rows[index], onToggle: onToggle)
    }

    private func syncSelection() {
        let index = rows.firstIndex { $0.selection == selection }
        guard table.selectedRow != index ?? -1 else { return }
        syncing = true
        if let index {
            table.selectRowIndexes(IndexSet(integer: index), byExtendingSelection: false)
            table.scrollRowToVisible(index)
        } else {
            table.deselectAll(nil)
        }
        syncing = false
    }

    // MARK: Table

    func numberOfRows(in tableView: NSTableView) -> Int { rows.count }

    func tableView(_ tableView: NSTableView, heightOfRow row: Int) -> CGFloat {
        // The blank first row fills the room above the first row; each row also takes the spacing
        // below it.
        if rows[row].id == SidebarRow.top.id { return sidebarTopRoom - tableView.intercellSpacing.height }
        // Headers sit apart from the group above them.
        return rows[row].isHeader ? 34 : 28
    }

    func tableView(_ tableView: NSTableView, rowViewForRow row: Int) -> NSTableRowView? {
        tableView.makeView(withIdentifier: SidebarRowView.identifier, owner: nil) as? SidebarRowView ?? SidebarRowView()
    }

    func tableView(_ tableView: NSTableView, viewFor tableColumn: NSTableColumn?, row: Int) -> NSView? {
        let cell = tableView.makeView(withIdentifier: SidebarCell.identifier, owner: nil) as? SidebarCell ?? SidebarCell()
        cell.show(rows[row], onToggle: onToggle)
        return cell
    }

    func tableView(_ tableView: NSTableView, shouldSelectRow row: Int) -> Bool {
        rows[row].selection != nil
    }

    func tableViewSelectionDidChange(_ notification: Notification) {
        guard !syncing, table.selectedRow >= 0, let item = rows[table.selectedRow].selection else { return }
        selection = item
        model?.selection = item
    }

    /// Clicking a section header opens or closes it.
    @objc private func clicked() {
        let row = table.clickedRow
        guard rows.indices.contains(row), rows[row].isHeader, rows[row].expanded != nil else { return }
        onToggle(rows[row].id)
    }

    /// ← closes the selected group and → opens it, as in Finder's sidebar.
    private func arrow(open: Bool) -> Bool {
        let row = table.selectedRow
        guard rows.indices.contains(row), let expanded = rows[row].expanded, expanded != open else { return false }
        onToggle(rows[row].id)
        return true
    }

    // MARK: Right-click menu

    func menuNeedsUpdate(_ menu: NSMenu) {
        menu.removeAllItems()
        let row = table.clickedRow
        guard rows.indices.contains(row), let target = rows[row].target, let model else { return }
        switch target {
        case .tag(let tag):
            menu.addItem(ActionMenuItem("Choose Icon…") { [weak self] in
                self?.chooseIcon(row: row, current: model.icon(forTag: tag)) { model.setIcon($0, forTag: tag) }
            })
            if model.icon(forTag: tag) != nil {
                menu.addItem(ActionMenuItem("Remove Icon") { model.setIcon(nil, forTag: tag) })
            }
        case .view(let id):
            guard let view = model.savedView(id) else { return }
            menu.addItem(ActionMenuItem("Edit View…") { model.editSavedView(view) })
            menu.addItem(ActionMenuItem("Choose Icon…") { [weak self] in
                self?.chooseIcon(row: row, current: view.icon) { model.setIcon($0, for: view) }
            })
            if view.icon != nil {
                menu.addItem(ActionMenuItem("Remove Icon") { model.setIcon(nil, for: view) })
            }
            menu.addItem(.separator())
            menu.addItem(ActionMenuItem("Delete View") { model.deleteSavedView(view) })
        }
    }

    private func chooseIcon(row: Int, current: String?, choose: @escaping (String?) -> Void) {
        guard rows.indices.contains(row) else { return }
        let popover = NSPopover()
        popover.behavior = .transient
        popover.contentViewController = NSHostingController(rootView: EmojiPicker(current: current) { [weak popover] icon in
            choose(icon)
            popover?.close()
        })
        popover.show(relativeTo: table.rect(ofRow: row), of: table, preferredEdge: .maxX)
        iconPopover = popover
    }
}

/// Hands ← and → to the controller, which opens and closes groups with them.
private final class SidebarTableView: NSTableView {
    var onArrow: (_ open: Bool) -> Bool = { _ in false }

    override func keyDown(with event: NSEvent) {
        switch event.specialKey {
        case .leftArrow? where onArrow(false): return
        case .rightArrow? where onArrow(true): return
        default: super.keyDown(with: event)
        }
    }
}

/// Draws the selection as a soft rounded fill, and tells its cell when the pointer is over it.
private final class SidebarRowView: NSTableRowView {
    static let identifier = NSUserInterfaceItemIdentifier("SidebarRowView")
    static let inset: CGFloat = 10

    init() {
        super.init(frame: .zero)
        identifier = Self.identifier
    }

    required init?(coder: NSCoder) { fatalError("init(coder:) is not used") }

    private var cell: SidebarCell? { view(atColumn: 0) as? SidebarCell }

    override func drawSelection(in dirtyRect: NSRect) {
        let rect = bounds.insetBy(dx: Self.inset, dy: 0)
        (isEmphasized ? Theme.selection : Theme.fill).setFill()
        NSBezierPath(roundedRect: rect, xRadius: 8, yRadius: 8).fill()
    }

    // Keeps the text its own colors on a selected row, rather than turning white.
    override var interiorBackgroundStyle: NSView.BackgroundStyle { .normal }

    override func updateTrackingAreas() {
        super.updateTrackingAreas()
        trackingAreas.forEach(removeTrackingArea)
        addTrackingArea(NSTrackingArea(rect: .zero, options: [.mouseEnteredAndExited, .activeAlways, .inVisibleRect], owner: self))
    }

    override func mouseEntered(with event: NSEvent) { cell?.hovering = true }
    override func mouseExited(with event: NSEvent) { cell?.hovering = false }

    override func prepareForReuse() {
        super.prepareForReuse()
        cell?.hovering = false
    }
}

/// One row's content: indent, icon, title, then a count and a chevron at the trailing edge.
private final class SidebarCell: NSTableCellView {
    static let identifier = NSUserInterfaceItemIdentifier("SidebarCell")

    private let symbol = NSImageView()
    private let emoji = NSTextField(labelWithString: "")
    private let title = NSTextField(labelWithString: "")
    private let badge = NSTextField(labelWithString: "")
    private let chevron = NSButton()
    private var leading: NSLayoutConstraint!
    private var row: SidebarRow?
    private var onToggle: (String) -> Void = { _ in }

    /// The chevron shows under the pointer, and always on a closed group.
    var hovering = false {
        didSet { updateChevron() }
    }

    init() {
        super.init(frame: .zero)
        identifier = Self.identifier

        symbol.symbolConfiguration = .init(pointSize: 13, weight: .regular)
        symbol.contentTintColor = Theme.secondaryText
        emoji.font = .systemFont(ofSize: 13)
        emoji.alignment = .center
        title.lineBreakMode = .byTruncatingTail
        title.setContentCompressionResistancePriority(.defaultLow, for: .horizontal)
        badge.font = .monospacedDigitSystemFont(ofSize: 12, weight: .regular)
        badge.textColor = Theme.secondaryText
        badge.setContentCompressionResistancePriority(.required, for: .horizontal)
        chevron.isBordered = false
        chevron.imagePosition = .imageOnly
        chevron.contentTintColor = Theme.tertiaryText
        chevron.target = self
        chevron.action = #selector(toggle)
        textField = title

        let icon = NSView()
        for view in [symbol, emoji] {
            view.translatesAutoresizingMaskIntoConstraints = false
            icon.addSubview(view)
            NSLayoutConstraint.activate([
                view.centerXAnchor.constraint(equalTo: icon.centerXAnchor),
                view.centerYAnchor.constraint(equalTo: icon.centerYAnchor),
            ])
        }
        icon.identifier = .init("icon")
        let stack = NSStackView(views: [icon, title, NSView(), badge, chevron])
        stack.orientation = .horizontal
        stack.spacing = 8
        stack.translatesAutoresizingMaskIntoConstraints = false
        addSubview(stack)
        leading = stack.leadingAnchor.constraint(equalTo: leadingAnchor)
        NSLayoutConstraint.activate([
            leading,
            stack.trailingAnchor.constraint(equalTo: trailingAnchor, constant: -(SidebarRowView.inset + 8)),
            stack.centerYAnchor.constraint(equalTo: centerYAnchor),
            icon.widthAnchor.constraint(equalToConstant: 18),
            icon.heightAnchor.constraint(equalToConstant: 18),
            chevron.widthAnchor.constraint(equalToConstant: 12),
        ])
    }

    required init?(coder: NSCoder) { fatalError("init(coder:) is not used") }

    func show(_ row: SidebarRow, onToggle: @escaping (String) -> Void) {
        self.row = row
        self.onToggle = onToggle
        let iconView = symbol.superview!
        if row.isHeader {
            leading.constant = SidebarRowView.inset + 8
            title.font = .systemFont(ofSize: 11, weight: .semibold)
            title.textColor = Theme.secondaryText
            iconView.isHidden = true
        } else {
            leading.constant = SidebarRowView.inset + 8 + CGFloat(row.depth) * 16
            title.font = .systemFont(ofSize: 13)
            title.textColor = Theme.text
            iconView.isHidden = row.icon == nil
        }
        title.stringValue = row.title
        switch row.icon {
        case .symbol(let name)?:
            symbol.image = NSImage(systemSymbolName: name, accessibilityDescription: nil)
            symbol.isHidden = false
            emoji.isHidden = true
        case .emoji(let text)?:
            emoji.stringValue = text
            emoji.isHidden = false
            symbol.isHidden = true
        case nil:
            symbol.isHidden = true
            emoji.isHidden = true
        }
        badge.stringValue = row.badge.map(String.init) ?? ""
        badge.isHidden = row.badge == nil || row.badge == 0
        toolTip = row.toolTip
        updateChevron()
    }

    private func updateChevron() {
        guard let expanded = row?.expanded else {
            chevron.isHidden = true
            return
        }
        chevron.isHidden = false
        let name = expanded ? "chevron.down" : "chevron.right"
        chevron.image = NSImage(systemSymbolName: name, accessibilityDescription: expanded ? "Collapse" : "Expand")?
            .withSymbolConfiguration(.init(pointSize: 10, weight: .semibold))
        chevron.alphaValue = hovering || !expanded ? 1 : 0
    }

    @objc private func toggle() {
        if let row { onToggle(row.id) }
    }
}

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

extension View {
    /// Moves the window when dragged, and zooms or minimizes it on a double-click as the
    /// title bar would, for the bars drawn where the title bar is. Buttons on top still
    /// take their own clicks.
    func windowDragArea() -> some View {
        background {
            Color.clear
                .contentShape(.rect)
                .gesture(WindowDragGesture())
                .simultaneousGesture(TapGesture(count: 2).onEnded {
                    guard let window = NSApp.keyWindow else { return }
                    switch UserDefaults.standard.string(forKey: "AppleActionOnDoubleClick") {
                    case "Minimize": window.performMiniaturize(nil)
                    case "None": break
                    default: window.performZoom(nil)
                    }
                })
                .allowsWindowActivationEvents(true)
        }
    }
}
