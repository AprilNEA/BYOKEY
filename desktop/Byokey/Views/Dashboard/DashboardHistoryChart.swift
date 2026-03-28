import Charts
import SwiftUI

struct DashboardHistoryChart: View {
    @Environment(DataService.self) private var dataService

    private var aggregated: [(date: Date, requests: UInt64)] {
        guard let history = dataService.history else { return [] }
        return Dictionary(grouping: history.buckets, by: \.period_start)
            .map { ts, buckets in
                (date: Date(timeIntervalSince1970: TimeInterval(ts)),
                 requests: buckets.reduce(0) { $0 + $1.request_count })
            }
            .sorted { $0.date < $1.date }
    }

    var body: some View {
        Card("REQUEST HISTORY") {
            if dataService.history != nil, !aggregated.isEmpty {
                Chart(aggregated, id: \.date) { bucket in
                    BarMark(
                        x: .value("Time", bucket.date),
                        y: .value("Requests", bucket.requests)
                    )
                    .foregroundStyle(.blue.gradient)
                    .cornerRadius(2)
                }
                .chartXAxis {
                    AxisMarks(values: .automatic(desiredCount: 8)) { _ in
                        AxisGridLine()
                        AxisValueLabel(format: .dateTime.hour())
                    }
                }
                .chartYAxis {
                    AxisMarks(position: .trailing) { _ in
                        AxisGridLine()
                        AxisValueLabel()
                    }
                }
                .frame(height: 120)
            } else {
                Text("No request data yet")
                    .foregroundStyle(.tertiary)
                    .frame(maxWidth: .infinity, alignment: .center)
                    .frame(height: 80)
            }
        }
    }
}
