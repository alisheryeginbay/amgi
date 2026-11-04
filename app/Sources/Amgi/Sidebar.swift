import AmgiCore
import AppKit
import SwiftUI

/// The sidebar: room for the traffic lights at the top, then the list of places to go.
/// The list is an AppKit table, drawn plainly on the window's background.
struct Sidebar: View {
    @Environment(AppModel.self) private var model
    @State private var showingWarnings = false

    var body: some View {
        // The list starts where the title bar ends, just under the traffic lights, and
        // rows scrolled up fade out there. Its first row is a blank one, so at rest none
        // sit in the fade.
        ZStack(alignment: .top) {
            SidebarTable(model: model, rows: rows, selection: model.selection)
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
        [
            SidebarRow.top,
            SidebarRow(id: "dashboard", title: "Dashboard", symbol: "chart.bar.xaxis", selection: .dashboard),
            SidebarRow(header: "Review", id: "h:review"),
            SidebarRow(
                id: "review",
                title: model.folder?.lastPathComponent ?? "All Cards",
                symbol: "rectangle.stack",
                badge: Int(model.totalDueAndLearning),
                selection: .review
            ),
        ]
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
    /// Stays the same as the row's content changes, so the table can tell rows apart.
    var id: String
    var isHeader = false
    var title: String
    /// An SF Symbol.
    var symbol: String?
    var badge: Int?
    var selection: SidebarItem?

    init(id: String, title: String, symbol: String, badge: Int? = nil, selection: SidebarItem) {
        self.id = id
        self.title = title
        self.symbol = symbol
        self.badge = badge
        self.selection = selection
    }

    /// The blank first row, which leaves room at the top.
    static let top = SidebarRow(header: "", id: "top")

    init(header: String, id: String) {
        self.id = id
        isHeader = true
        title = header
    }
}

private struct SidebarTable: NSViewRepresentable {
    let model: AppModel
    let rows: [SidebarRow]
    let selection: SidebarItem

    func makeCoordinator() -> SidebarController { SidebarController(model: model) }

    func makeNSView(context: Context) -> NSScrollView { context.coordinator.scrollView }

    func updateNSView(_ view: NSScrollView, context: Context) {
        context.coordinator.update(rows: rows, selection: selection)
    }
}

/// Runs the sidebar's table: fills it from the rows and keeps its selection in step with
/// the model's.
@MainActor
final class SidebarController: NSObject, NSTableViewDataSource, NSTableViewDelegate {
    let scrollView = NSScrollView()
    private let table = NSTableView()
    private weak var model: AppModel?
    private var rows: [SidebarRow] = []
    private var selection: SidebarItem?
    /// Set while the table's selection is changed from the model, so it isn't sent back.
    private var syncing = false

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
        } else {
            table.reloadData()
        }
        syncSelection()
    }

    private func configure(row index: Int) {
        guard let cell = table.view(atColumn: 0, row: index, makeIfNecessary: false) as? SidebarCell else { return }
        cell.show(rows[index])
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
        cell.show(rows[row])
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
}

/// Draws the selection as a soft rounded fill.
private final class SidebarRowView: NSTableRowView {
    static let identifier = NSUserInterfaceItemIdentifier("SidebarRowView")
    static let inset: CGFloat = 10

    init() {
        super.init(frame: .zero)
        identifier = Self.identifier
    }

    required init?(coder: NSCoder) { fatalError("init(coder:) is not used") }

    override func drawSelection(in dirtyRect: NSRect) {
        let rect = bounds.insetBy(dx: Self.inset, dy: 0)
        (isEmphasized ? Theme.selection : Theme.fill).setFill()
        NSBezierPath(roundedRect: rect, xRadius: 8, yRadius: 8).fill()
    }

    // Keeps the text its own colors on a selected row, rather than turning white.
    override var interiorBackgroundStyle: NSView.BackgroundStyle { .normal }
}

/// One row's content: icon and title, then a count at the trailing edge.
private final class SidebarCell: NSTableCellView {
    static let identifier = NSUserInterfaceItemIdentifier("SidebarCell")

    private let symbol = NSImageView()
    private let title = NSTextField(labelWithString: "")
    private let badge = NSTextField(labelWithString: "")

    init() {
        super.init(frame: .zero)
        identifier = Self.identifier

        symbol.symbolConfiguration = .init(pointSize: 13, weight: .regular)
        symbol.contentTintColor = Theme.secondaryText
        title.lineBreakMode = .byTruncatingTail
        title.setContentCompressionResistancePriority(.defaultLow, for: .horizontal)
        badge.font = .monospacedDigitSystemFont(ofSize: 12, weight: .regular)
        badge.textColor = Theme.secondaryText
        badge.setContentCompressionResistancePriority(.required, for: .horizontal)
        textField = title

        let stack = NSStackView(views: [symbol, title, NSView(), badge])
        stack.orientation = .horizontal
        stack.spacing = 8
        stack.translatesAutoresizingMaskIntoConstraints = false
        addSubview(stack)
        NSLayoutConstraint.activate([
            stack.leadingAnchor.constraint(equalTo: leadingAnchor, constant: SidebarRowView.inset + 8),
            stack.trailingAnchor.constraint(equalTo: trailingAnchor, constant: -(SidebarRowView.inset + 8)),
            stack.centerYAnchor.constraint(equalTo: centerYAnchor),
            symbol.widthAnchor.constraint(equalToConstant: 18),
            symbol.heightAnchor.constraint(equalToConstant: 18),
        ])
    }

    required init?(coder: NSCoder) { fatalError("init(coder:) is not used") }

    func show(_ row: SidebarRow) {
        if row.isHeader {
            title.font = .systemFont(ofSize: 11, weight: .semibold)
            title.textColor = Theme.secondaryText
        } else {
            title.font = .systemFont(ofSize: 13)
            title.textColor = Theme.text
        }
        title.stringValue = row.title
        symbol.image = row.symbol.flatMap { NSImage(systemSymbolName: $0, accessibilityDescription: nil) }
        symbol.isHidden = row.symbol == nil
        badge.stringValue = row.badge.map(String.init) ?? ""
        badge.isHidden = row.badge == nil || row.badge == 0
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
