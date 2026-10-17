import AmgiCore
import Charts
import SwiftUI

/// The Card Info window: a card's schedule, memory and review history. It shows the
/// card last asked about, and keeps up as that card is studied or edited.
struct CardInfoView: View {
    @Environment(AppModel.self) private var model

    var body: some View {
        Group {
            if let info = model.infoCard {
                CardInfoForm(info: info, target: model.desiredRetention)
            } else {
                ContentUnavailableView("No Card", systemImage: "info.circle", description: Text("Choose Card Info for a card to see its history."))
            }
        }
        .frame(minWidth: 360, minHeight: 420)
        .toolbar {
            if let info = model.infoCard {
                Button {
                    model.revealInEditor(path: info.path)
                } label: {
                    Label("Edit Card", systemImage: "pencil")
                }
                .help("Open \(info.path), line \(info.line)")
            }
        }
    }
}

private struct CardInfoForm: View {
    let info: CardInfo
    /// The desired retention, at which a card comes due.
    let target: Double

    var body: some View {
        // Read again each minute, so "due in" and recall stay current while open.
        TimelineView(.everyMinute) { context in
            Form {
                Section {
                    VStack(alignment: .leading, spacing: 6) {
                        Text(inline(info.front))
                            .font(.title3)
                        Text(inline(info.back))
                            .foregroundStyle(.secondary)
                    }
                    .lineLimit(4)
                    .textSelection(.enabled)
                    LabeledContent("Deck", value: info.deck)
                    LabeledContent("File", value: "\(info.path):\(info.line)")
                    if !info.tags.isEmpty {
                        LabeledContent("Tags", value: info.tags.map { "#\($0)" }.joined(separator: " "))
                    }
                }

                Section("Schedule") {
                    LabeledContent("Status", value: status(now: context.date))
                    if let due = info.due.map(date) {
                        LabeledContent("Due", value: due.formatted(date: .abbreviated, time: .shortened))
                    }
                    if let due = info.due, let last = info.lastReview {
                        LabeledContent("Interval", value: formatInterval(seconds: due - last))
                    }
                    if let last = info.lastReview.map(date) {
                        LabeledContent("Last Review", value: last.formatted(.relative(presentation: .named)))
                    }
                }

                if let stability = info.stability, let difficulty = info.difficulty {
                    Section {
                        LabeledContent("Stability", value: formatInterval(seconds: Int64(stability * 86_400)))
                            .help("How long until the chance of recalling it falls to 90%")
                        LabeledContent("Difficulty", value: percent((difficulty - 1) / 9))
                            .help("How hard the card is to remember, from 0% to 100%")
                        if let recall = info.retrievability {
                            LabeledContent("Recall Now", value: percent(recall))
                                .help("The chance of recalling it if studied now")
                        }
                    } header: {
                        Text("Memory")
                    }
                }

                if !info.reviews.isEmpty {
                    Section {
                        ForgettingCurve(reviews: info.reviews, target: target, now: context.date)
                    }
                }

                Section("Reviews") {
                    LabeledContent("Reviews", value: "\(info.reps)")
                    LabeledContent("Lapses", value: "\(info.lapses)")
                    ForEach(Array(info.reviews.enumerated().reversed()), id: \.offset) { _, review in
                        ReviewRow(review: review)
                    }
                }
            }
            .formStyle(.grouped)
        }
    }

    private func status(now: Date) -> String {
        if info.suspended { return "Suspended" }
        guard let due = info.due.map(date) else { return "New" }
        return due <= now ? "Due now" : "Due \(due.formatted(.relative(presentation: .named)))"
    }
}

/// The chance of recalling the card over its whole history: each review sets it back to
/// 100%, from where it falls on the curve of the stability that review left, until the
/// next. Dashed from now on, through the target retention where the card comes due.
/// Hovering reads off a day.
private struct ForgettingCurve: View {
    /// Oldest first; not empty.
    let reviews: [CardReview]
    let target: Double
    let now: Date
    @State private var hovered: Date?

    private struct Point: Identifiable {
        let id: Int
        let date: Date
        let recall: Double
    }

    private var start: Date { date(reviews[0].reviewedAt) }

    /// Past now by a third of the history, at least to where the curve reaches the target.
    private var end: Date {
        let last = reviews[reviews.count - 1]
        let due = date(last.reviewedAt + last.intervalSecs)
        let lead = max(now.timeIntervalSince(start) / 3, 86_400)
        return max(now.addingTimeInterval(lead), due.addingTimeInterval(lead / 3))
    }

    /// Recall on `day`, from the last review at or before it.
    private func recall(at day: Date) -> Double? {
        guard let review = reviews.last(where: { date($0.reviewedAt) <= day }) else { return nil }
        return recallProbability(stability: review.stability, elapsedDays: day.timeIntervalSince(date(review.reviewedAt)) / 86_400)
    }

    /// Each review's stretch sampled, ending in the drop the next review resets.
    private func samples(from: Date, to: Date, stability: Double, since review: Date, into points: inout [Point]) {
        let steps = 40
        for i in 0...steps {
            let day = from.addingTimeInterval(to.timeIntervalSince(from) * Double(i) / Double(steps))
            let recall = recallProbability(stability: stability, elapsedDays: day.timeIntervalSince(review) / 86_400)
            points.append(Point(id: points.count, date: day, recall: recall))
        }
    }

    private var curves: (past: [Point], future: [Point]) {
        var past: [Point] = []
        for (i, review) in reviews.enumerated() {
            let from = date(review.reviewedAt)
            let to = i + 1 < reviews.count ? date(reviews[i + 1].reviewedAt) : now
            samples(from: from, to: max(from, to), stability: review.stability, since: from, into: &past)
        }
        var future: [Point] = []
        let last = reviews[reviews.count - 1]
        samples(from: now, to: end, stability: last.stability, since: date(last.reviewedAt), into: &future)
        return (past, future)
    }

    /// The lowest 10% step the curve reaches, so its fall shows.
    private func floor(_ points: [Point]) -> Double {
        let lowest = min(points.map(\.recall).min() ?? 1, target)
        return max(0, ((lowest - 0.02) * 10).rounded(.down) / 10)
    }

    private let grid = Color(nsColor: .separatorColor)
    private let ink = Color(nsColor: .secondaryLabelColor)

    var body: some View {
        let (past, future) = curves
        let floor = floor(past + future)
        VStack(alignment: .leading, spacing: 4) {
            Text("Forgetting Curve")
            Text("Chance of recall over time; each review resets it")
                .font(.caption)
                .foregroundStyle(.secondary)
            Chart {
                RuleMark(y: .value("Target", target))
                    .foregroundStyle(ink)
                    .lineStyle(StrokeStyle(lineWidth: 1, dash: [3, 3]))
                    .annotation(position: .bottom, alignment: .leading, spacing: 2) {
                        Text("Target \(percent(target))").font(.caption2).foregroundStyle(ink)
                    }
                ForEach(past) { point in
                    LineMark(x: .value("Date", point.date), y: .value("Recall", point.recall), series: .value("Part", "Past"))
                        .foregroundStyle(Color.accentColor)
                        .lineStyle(StrokeStyle(lineWidth: 2, lineCap: .round, lineJoin: .round))
                }
                ForEach(future) { point in
                    LineMark(x: .value("Date", point.date), y: .value("Recall", point.recall), series: .value("Part", "Future"))
                        .foregroundStyle(Color.accentColor)
                        .lineStyle(StrokeStyle(lineWidth: 2, lineCap: .round, dash: [4, 4]))
                }
                if let today = recall(at: now) {
                    PointMark(x: .value("Date", now), y: .value("Recall", today))
                        .symbolSize(100)
                        .foregroundStyle(Color(nsColor: .controlBackgroundColor))
                    PointMark(x: .value("Date", now), y: .value("Recall", today))
                        .symbolSize(50)
                        .foregroundStyle(Color.accentColor)
                }
                if let hovered, let value = recall(at: hovered) {
                    RuleMark(x: .value("Date", hovered))
                        .foregroundStyle(grid)
                        .annotation(position: .top, spacing: 0, overflowResolution: .init(x: .fit(to: .chart), y: .disabled)) {
                            VStack(alignment: .leading, spacing: 1) {
                                Text(percent(value)).font(.caption.weight(.semibold))
                                Text(hovered.formatted(date: .abbreviated, time: .omitted)).font(.caption2).foregroundStyle(ink)
                            }
                            .padding(.horizontal, 6)
                            .padding(.vertical, 3)
                            .background(.background, in: .rect(cornerRadius: 5))
                            .shadow(color: .black.opacity(0.12), radius: 3, y: 1)
                        }
                    PointMark(x: .value("Date", hovered), y: .value("Recall", value))
                        .symbolSize(40)
                        .foregroundStyle(Color.accentColor)
                }
            }
            .chartXScale(domain: start...end)
            .chartYScale(domain: floor...1)
            .chartYAxis {
                AxisMarks(position: .leading, values: stride(from: floor, through: 1.0001, by: floor < 0.5 ? 0.2 : 0.1).map { ($0 * 10).rounded() / 10 }) { value in
                    AxisGridLine().foregroundStyle(grid)
                    AxisValueLabel {
                        if let v = value.as(Double.self) { Text(percent(v)) }
                    }
                }
            }
            .chartXAxis {
                AxisMarks(values: .automatic(desiredCount: 4)) { _ in
                    AxisGridLine().foregroundStyle(grid)
                    AxisValueLabel()
                }
            }
            .chartXSelection(value: $hovered)
            .frame(height: 180)
            .padding(.top, 12)
        }
        .padding(.vertical, 4)
    }
}

/// One review, newest first: when, the answer given, and the interval it led to.
private struct ReviewRow: View {
    let review: CardReview

    var body: some View {
        HStack {
            Text(date(review.reviewedAt).formatted(date: .abbreviated, time: .shortened))
                .foregroundStyle(.secondary)
            Spacer()
            Text(review.rating.label)
                .foregroundStyle(review.rating.color)
            Text(formatInterval(seconds: review.intervalSecs))
                .monospacedDigit()
                .foregroundStyle(.secondary)
                .frame(minWidth: 44, alignment: .trailing)
        }
        .help(review.wasNew
            ? "First review; stability \(formatInterval(seconds: Int64(review.stability * 86_400)))"
            : "\(Int(review.elapsedDays)) days after the review before; stability \(formatInterval(seconds: Int64(review.stability * 86_400)))")
    }
}

private extension Rating {
    var color: Color {
        switch self {
        case .again: .red
        case .hard: .orange
        case .good: .green
        case .easy: .blue
        }
    }
}

private func date(_ seconds: Int64) -> Date {
    Date(timeIntervalSince1970: TimeInterval(seconds))
}

private func percent(_ value: Double) -> String {
    value.formatted(.percent.precision(.fractionLength(0)))
}

/// Card Markdown as styled text, inline only: images and sounds drop out.
private func inline(_ markdown: String) -> AttributedString {
    let options = AttributedString.MarkdownParsingOptions(interpretedSyntax: .inlineOnlyPreservingWhitespace)
    return (try? AttributedString(markdown: markdown, options: options)) ?? AttributedString(markdown)
}
