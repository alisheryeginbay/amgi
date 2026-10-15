import AmgiCore
import AppKit
import Charts
import SwiftUI

/// Collection-wide statistics: today's numbers, a year of activity, upcoming reviews.
struct DashboardView: View {
    @Environment(AppModel.self) private var model

    var body: some View {
        ScrollView {
            if let dashboard = model.dashboard {
                VStack(alignment: .leading, spacing: 20) {
                    HStack(spacing: 12) {
                        StatTile(value: model.totalDue.formatted(), label: "Due")
                        StatTile(value: model.today.newAvailable.formatted(), label: "New today")
                        StatTile(value: model.today.reviewed.formatted(), label: "Reviewed today")
                        StatTile(value: dashboard.streak == 1 ? "1 day" : "\(dashboard.streak) days", label: "Streak")
                    }
                    Panel("Activity") {
                        ActivityHeatmap(activity: dashboard.activity, start: dashboard.activityStart)
                    }
                    HStack(alignment: .top, spacing: 20) {
                        Panel("Due in the next two weeks") {
                            ForecastChart(forecast: dashboard.forecast)
                        }
                        Panel("Cards") {
                            CardBreakdown(dashboard: dashboard)
                        }
                        .frame(width: 220)
                    }
                }
                .padding(32)
                .frame(maxWidth: 860, alignment: .leading)
                .frame(maxWidth: .infinity)
            }
        }
        .navigationTitle("Dashboard")
        .navigationSubtitle(model.folder?.lastPathComponent ?? "")
    }
}

private struct StatTile: View {
    let value: String
    let label: String

    var body: some View {
        VStack(alignment: .leading, spacing: 2) {
            Text(value)
                .font(.title.weight(.semibold))
                .monospacedDigit()
            Text(label)
                .font(.callout)
                .foregroundStyle(.secondary)
        }
        .frame(maxWidth: .infinity, alignment: .leading)
        .padding(16)
        .background(.quinary, in: .rect(cornerRadius: 12))
    }
}

private struct Panel<Content: View>: View {
    let title: String
    @ViewBuilder let content: Content

    init(_ title: String, @ViewBuilder content: () -> Content) {
        self.title = title
        self.content = content()
    }

    var body: some View {
        VStack(alignment: .leading, spacing: 14) {
            Text(title)
                .font(.headline)
            content
        }
        .frame(maxWidth: .infinity, alignment: .leading)
        .padding(16)
        .background(.quinary, in: .rect(cornerRadius: 12))
    }
}

// MARK: - Activity heatmap

/// A year of reviews as a grid of days: one column per week, like GitHub's
/// contribution graph. Shade is the day's quartile among days with reviews.
struct ActivityHeatmap: View {
    let activity: [UInt32]
    /// Unix seconds at which the first day starts.
    let start: Int64

    private static let weeks = 53
    private static let cell: CGFloat = 11
    private static let gap: CGFloat = 3
    private static let monthSpacing: CGFloat = 4

    /// Days to request so the grid ends today and fills `weeks` columns.
    static func dayCount(now: Date = .now) -> UInt32 {
        // Study days roll over at 4am, like the scheduler.
        let today = now.addingTimeInterval(-4 * 3600)
        return UInt32((weeks - 1) * 7 + weekdayIndex(today) + 1)
    }

    /// Row of a date in its week column, counting from the locale's first weekday.
    private static func weekdayIndex(_ date: Date) -> Int {
        let calendar = Calendar.current
        return (calendar.component(.weekday, from: date) - calendar.firstWeekday + 7) % 7
    }

    private struct Day {
        let index: Int
        let date: Date
        let count: UInt32
    }

    /// Index of the day under the pointer.
    @State private var hovered: Int?
    private static let tooltipWidth: CGFloat = 220

    var body: some View {
        let days = activity.enumerated().map { i, count in
            Day(index: i, date: Date(timeIntervalSince1970: TimeInterval(start) + TimeInterval(i) * 86_400), count: count)
        }
        let weeks = columns(days)
        let levels = Levels(activity)

        VStack(alignment: .leading, spacing: 10) {
            HStack(alignment: .bottom, spacing: Self.gap) {
                // Outside the scroll view, so the labels stay put when the grid scrolls.
                weekdayLabels
                ScrollView(.horizontal) {
                    VStack(alignment: .leading, spacing: Self.monthSpacing) {
                        monthLabels(weeks)
                        HStack(alignment: .top, spacing: Self.gap) {
                            ForEach(weeks.indices, id: \.self) { column in
                                VStack(spacing: Self.gap) {
                                    ForEach(0..<7, id: \.self) { row in
                                        cell(weeks[column][row], levels: levels)
                                    }
                                }
                            }
                        }
                    }
                }
                .scrollIndicators(.never)
                .defaultScrollAnchor(.trailing)
                // Only as tall as the grid, so the weekday labels line up with its rows.
                .fixedSize(horizontal: false, vertical: true)
            }

            HStack {
                Text(summary)
                    .foregroundStyle(.secondary)
                Spacer()
                legend
            }
            .font(.caption)
        }
        // Drawn over the whole heatmap, so the scroll view around the grid can't clip it.
        .overlayPreferenceValue(HoveredCell.self) { anchor in
            GeometryReader { proxy in
                if let anchor, let hovered, days.indices.contains(hovered) {
                    let cell = proxy[anchor]
                    let half = Self.tooltipWidth / 2
                    // Keep the bubble inside the heatmap; the arrow still points at the cell.
                    let x = min(max(cell.midX, half), max(proxy.size.width - half, half))
                    DayTooltip(count: days[hovered].count, date: days[hovered].date, arrowOffset: cell.midX - x)
                        .frame(width: Self.tooltipWidth, height: 0, alignment: .bottom)
                        .position(x: x, y: cell.minY - 3)
                        .allowsHitTesting(false)
                }
            }
        }
        .accessibilityElement(children: .ignore)
        .accessibilityLabel("Activity")
        .accessibilityValue(summary)
    }

    /// Days grouped into week columns, padded with `nil` before the first day and after today.
    private func columns(_ days: [Day]) -> [[Day?]] {
        guard let first = days.first else { return [] }
        let padded: [Day?] = Array(repeating: nil, count: Self.weekdayIndex(first.date)) + days
        return stride(from: 0, to: padded.count, by: 7).map { i in
            let week = Array(padded[i..<min(i + 7, padded.count)])
            return week + Array(repeating: nil, count: 7 - week.count)
        }
    }

    private func cell(_ day: Day?, levels: Levels) -> some View {
        let shape = RoundedRectangle(cornerRadius: 2.5, style: .continuous)
        return Group {
            if let day {
                shape
                    .fill(HeatmapColor.fill(level: levels.level(day.count)))
                    .overlay {
                        if hovered == day.index {
                            shape.strokeBorder(.primary, lineWidth: 1.5)
                        }
                    }
                    .anchorPreference(key: HoveredCell.self, value: .bounds) { hovered == day.index ? $0 : nil }
                    .onHover { inside in
                        if inside {
                            hovered = day.index
                        } else if hovered == day.index {
                            hovered = nil
                        }
                    }
            } else {
                shape.fill(.clear)
            }
        }
        .frame(width: Self.cell, height: Self.cell)
    }

    /// Short month names above the first column of each month.
    private func monthLabels(_ weeks: [[Day?]]) -> some View {
        let calendar = Calendar.current
        var lastMonth: Int?
        var lastLabel = -3
        let labels: [String?] = weeks.enumerated().map { column, week in
            guard let day = week.compactMap({ $0 }).first else { return nil }
            let month = calendar.component(.month, from: day.date)
            defer { lastMonth = month }
            // Skip a label that would crowd the previous one, such as a partial first month.
            guard month != lastMonth, column - lastLabel >= 3 else { return nil }
            lastLabel = column
            return day.date.formatted(.dateTime.month(.abbreviated))
        }
        return HStack(spacing: Self.gap) {
            ForEach(labels.indices, id: \.self) { column in
                Text(labels[column] ?? "")
                    .fixedSize()
                    .frame(width: Self.cell, alignment: .leading)
            }
        }
        .font(.caption2)
        .foregroundStyle(.secondary)
    }

    private let weekdayLabelWidth: CGFloat = 26

    /// Every other weekday name, like GitHub's Mon / Wed / Fri.
    private var weekdayLabels: some View {
        let calendar = Calendar.current
        let symbols = calendar.shortWeekdaySymbols
        return VStack(alignment: .leading, spacing: Self.gap) {
            ForEach(0..<7, id: \.self) { row in
                Text(row % 2 == 1 ? symbols[(calendar.firstWeekday - 1 + row) % 7] : "")
                    .font(.caption2)
                    .foregroundStyle(.secondary)
                    .frame(width: weekdayLabelWidth, height: Self.cell, alignment: .leading)
            }
        }
    }

    private var legend: some View {
        HStack(spacing: Self.gap) {
            Text("Less")
                .padding(.trailing, 2)
            ForEach(0...HeatmapColor.levels, id: \.self) { level in
                RoundedRectangle(cornerRadius: 2.5, style: .continuous)
                    .fill(HeatmapColor.fill(level: level))
                    .frame(width: Self.cell, height: Self.cell)
            }
            Text("More")
                .padding(.leading, 2)
        }
        .foregroundStyle(.secondary)
    }

    private var summary: String {
        let total = activity.reduce(0, +)
        let studied = activity.filter { $0 > 0 }.count
        return "\(total.formatted()) reviews on \(studied.formatted()) days in the last year"
    }
}

/// Frame of the hovered heatmap cell.
private struct HoveredCell: PreferenceKey {
    static let defaultValue: Anchor<CGRect>? = nil

    static func reduce(value: inout Anchor<CGRect>?, nextValue: () -> Anchor<CGRect>?) {
        value = value ?? nextValue()
    }
}

/// Dark bubble with an arrow pointing down at a heatmap day.
private struct DayTooltip: View {
    let count: UInt32
    let date: Date
    /// Horizontal distance from the bubble's center to the cell.
    let arrowOffset: CGFloat

    private static let fill = Color(light: 0x2B2B2B, dark: 0x4A4A4A)

    var body: some View {
        let amount = Text(count == 0 ? "No" : count.formatted()).bold()
        let noun = count == 1 ? "card" : "cards"
        let day = date.formatted(.dateTime.weekday(.wide).month(.wide).day().year())
        VStack(spacing: 0) {
            Text("\(amount) \(noun) reviewed on \(day)")
                .font(.callout)
                .multilineTextAlignment(.center)
                .foregroundStyle(.white)
                .fixedSize(horizontal: false, vertical: true)
                .padding(.horizontal, 10)
                .padding(.vertical, 6)
                .background(Self.fill, in: .rect(cornerRadius: 6))
            Arrow()
                .fill(Self.fill)
                .frame(width: 12, height: 6)
                .offset(x: arrowOffset)
        }
        .shadow(color: .black.opacity(0.2), radius: 4, y: 2)
    }

    private struct Arrow: Shape {
        func path(in rect: CGRect) -> Path {
            Path { p in
                p.move(to: CGPoint(x: rect.minX, y: rect.minY))
                p.addLine(to: CGPoint(x: rect.maxX, y: rect.minY))
                p.addLine(to: CGPoint(x: rect.midX, y: rect.maxY))
                p.closeSubpath()
            }
        }
    }
}

/// Quartile boundaries of the days with reviews, so one huge day does not wash out the rest.
private struct Levels {
    private let upperBounds: [UInt32]

    init(_ counts: [UInt32]) {
        let active = counts.filter { $0 > 0 }.sorted()
        upperBounds = active.isEmpty ? [] : [0.25, 0.5, 0.75].map { active[Int(Double(active.count - 1) * $0)] }
    }

    /// 0 for no reviews, otherwise 1 through `HeatmapColor.levels`.
    func level(_ count: UInt32) -> Int {
        count == 0 ? 0 : 1 + upperBounds.filter { count > $0 }.count
    }
}

/// One-hue ordinal ramp, validated separately for each appearance: every step clears
/// 2:1 against the window background and adjacent steps differ in lightness.
private enum HeatmapColor {
    static let levels = 4
    private static let light: [UInt32] = [0x86B6EF, 0x3987E5, 0x1C5CAB, 0x0D366B]
    private static let dark: [UInt32] = [0x184F95, 0x256ABF, 0x5598E7, 0x9EC5F4]
    private static let ramp = zip(light, dark).map { Color(light: $0, dark: $1) }

    static func fill(level: Int) -> AnyShapeStyle {
        level == 0 ? AnyShapeStyle(.quaternary) : AnyShapeStyle(ramp[min(level, levels) - 1])
    }
}

// MARK: - Forecast

private struct ForecastChart: View {
    let forecast: [UInt32]
    @State private var selected: Int?

    private static let bar = Color(light: 0x2A78D6, dark: 0x3987E5)

    var body: some View {
        Chart {
            ForEach(Array(forecast.enumerated()), id: \.offset) { day, count in
                BarMark(x: .value("Day", day), y: .value("Cards", count), width: .ratio(0.7))
                    .foregroundStyle(Self.bar.opacity(selected == nil || selected == day ? 1 : 0.4))
                    .cornerRadius(4)
            }
            if let selected, forecast.indices.contains(selected) {
                RuleMark(x: .value("Day", selected))
                    .foregroundStyle(.clear)
                    .annotation(position: .top, overflowResolution: .init(x: .fit, y: .disabled)) {
                        VStack(alignment: .leading, spacing: 1) {
                            Text(dayName(selected))
                                .foregroundStyle(.secondary)
                            Text(forecast[selected] == 1 ? "1 card" : "\(forecast[selected]) cards")
                                .monospacedDigit()
                        }
                        .font(.caption)
                        .padding(.horizontal, 8)
                        .padding(.vertical, 4)
                        .background(.regularMaterial, in: .rect(cornerRadius: 6))
                    }
            }
        }
        .chartXScale(domain: -0.5...Double(forecast.count) - 0.5)
        .chartXAxis {
            AxisMarks(values: Array(stride(from: 0, to: forecast.count, by: 2))) { value in
                AxisValueLabel {
                    if let day = value.as(Int.self) {
                        Text(day == 0 ? "Today" : dayName(day, short: true))
                    }
                }
            }
        }
        .chartYAxis {
            AxisMarks(position: .leading) {
                AxisGridLine()
                AxisValueLabel()
            }
        }
        .chartXSelection(value: $selected)
        .frame(height: 180)
        .accessibilityLabel("Cards due per day")
    }

    private func dayName(_ offset: Int, short: Bool = false) -> String {
        guard offset > 0 else { return "Today" }
        let date = Calendar.current.date(byAdding: .day, value: offset, to: .now) ?? .now
        return short
            ? date.formatted(.dateTime.weekday(.narrow))
            : date.formatted(.dateTime.weekday(.wide).month(.abbreviated).day())
    }
}

// MARK: - Card counts

private struct CardBreakdown: View {
    let dashboard: Dashboard

    var body: some View {
        VStack(alignment: .leading, spacing: 10) {
            row("New", dashboard.newCards, help: "Never studied")
            row("Young", dashboard.youngCards, help: "Next review less than 21 days after the last")
            row("Mature", dashboard.matureCards, help: "Next review 21 days or more after the last")
            Divider()
            HStack {
                Text("Retention")
                Spacer()
                Text(dashboard.retention.map { $0.formatted(.percent.precision(.fractionLength(0))) } ?? "–")
                    .monospacedDigit()
            }
            .help("Reviews in the last 30 days not rated Again, counting cards last seen a day or more before")
        }
    }

    private func row(_ label: String, _ count: UInt32, help: String) -> some View {
        HStack {
            Text(label)
            Spacer()
            Text(count.formatted())
                .monospacedDigit()
                .foregroundStyle(.secondary)
        }
        .help(help)
    }
}

extension Color {
    /// A color with separate values for light and dark appearances, as `0xRRGGBB`.
    init(light: UInt32, dark: UInt32) {
        func rgb(_ hex: UInt32) -> NSColor {
            NSColor(
                srgbRed: CGFloat((hex >> 16) & 0xFF) / 255,
                green: CGFloat((hex >> 8) & 0xFF) / 255,
                blue: CGFloat(hex & 0xFF) / 255,
                alpha: 1
            )
        }
        self.init(nsColor: NSColor(name: nil) { appearance in
            appearance.bestMatch(from: [.aqua, .darkAqua]) == .darkAqua ? rgb(dark) : rgb(light)
        })
    }
}
