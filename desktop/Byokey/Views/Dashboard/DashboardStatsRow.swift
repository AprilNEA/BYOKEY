import Charts
import SwiftUI

struct DashboardStatsRow: View {
    @Environment(DataService.self) private var dataService

    private var usage: UsageSnapshot? { dataService.usage }

    var body: some View {
        HStack(alignment: .top, spacing: 12) {
            requestsCard
                .frame(maxHeight: .infinity, alignment: .top)
            tokenCard(
                title: "INPUT TOKENS",
                value: usage?.input_tokens ?? 0,
                color: .indigo,
                points: tokenTimeSeries(\.input_tokens)
            )
            .frame(maxHeight: .infinity, alignment: .top)
            tokenCard(
                title: "OUTPUT TOKENS",
                value: usage?.output_tokens ?? 0,
                color: .cyan,
                points: tokenTimeSeries(\.output_tokens)
            )
            .frame(maxHeight: .infinity, alignment: .top)
        }
        .fixedSize(horizontal: false, vertical: true)
    }

    private var requestsCard: some View {
        Card("REQUESTS") {
            HeroNumber(value: usage?.total_requests ?? 0)

            HStack(spacing: 16) {
                HStack(spacing: 4) {
                    Image(systemName: "checkmark")
                        .foregroundStyle(.green)
                    Text("\(usage?.success_requests ?? 0)")
                }
                Divider().frame(height: 14)
                HStack(spacing: 4) {
                    Image(systemName: "xmark")
                        .foregroundStyle(
                            (usage?.failure_requests ?? 0) > 0 ? .red : .secondary
                        )
                    Text("\(usage?.failure_requests ?? 0)")
                }
            }
            .font(.caption)
            .foregroundStyle(.secondary)
        }
    }

    private func tokenCard(
        title: String, value: UInt64, color: Color,
        points: [(date: Date, value: UInt64)]
    ) -> some View {
        Card(title) {
            HStack(alignment: .firstTextBaseline, spacing: 2) {
                let (num, unit) = formatTokenParts(value)
                Text(num)
                    .font(.system(size: 34, weight: .bold, design: .rounded))
                    .monospacedDigit()
                Text(unit)
                    .font(.system(size: 14, weight: .medium))
                    .foregroundStyle(.secondary)
            }

            if !points.isEmpty {
                Chart(points, id: \.date) { pt in
                    AreaMark(
                        x: .value("T", pt.date),
                        y: .value("V", pt.value)
                    )
                    .foregroundStyle(color.gradient.opacity(0.2))
                    .interpolationMethod(.catmullRom)

                    LineMark(
                        x: .value("T", pt.date),
                        y: .value("V", pt.value)
                    )
                    .foregroundStyle(color.gradient)
                    .interpolationMethod(.catmullRom)
                    .lineStyle(StrokeStyle(lineWidth: 1.5))
                }
                .chartXAxis(.hidden)
                .chartYAxis(.hidden)
                .frame(height: 36)

                if let peak = points.max(by: { $0.value < $1.value })?.value, peak > 0 {
                    Text("Peak \(formatTokens(peak))")
                        .font(.caption2)
                        .foregroundStyle(.tertiary)
                }
            } else {
                Text("—")
                    .font(.caption)
                    .foregroundStyle(.quaternary)
            }
        }
    }

    private func tokenTimeSeries(
        _ keyPath: KeyPath<UsageBucket, UInt64>
    ) -> [(date: Date, value: UInt64)] {
        guard let history = dataService.history else { return [] }
        return Dictionary(grouping: history.buckets, by: \.period_start)
            .map { ts, buckets in
                (date: Date(timeIntervalSince1970: TimeInterval(ts)),
                 value: buckets.reduce(0) { $0 + $1[keyPath: keyPath] })
            }
            .sorted { $0.date < $1.date }
    }
}
