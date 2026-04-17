import Charts
import SwiftUI

struct OverviewView: View {
    @Environment(ProcessManager.self) private var pm
    @Environment(AppEnvironment.self) private var appEnv
    @Environment(DataService.self) private var dataService
    @State private var copiedEndpoint = false
    @State private var range: UsageRange = .day

    var body: some View {
        DetailPage("Overview") {
            ScrollView {
                VStack(alignment: .leading, spacing: Spacing.lg) {
                    // Server control (always shown)
                    serverSection

                    // Token usage chart (always shown — tries to load regardless of server state)
                    usageSection

                    // At-a-glance stats (only when server live and has data)
                    if pm.isReachable {
                        summarySection
                    }
                }
            }
        }
        .task(id: range) { await reloadHistory() }
    }

    // MARK: - Server Control

    private var serverSection: some View {
        HStack(alignment: .top, spacing: Spacing.md) {
            serverControlCard
            if pm.isReachable {
                endpointCard
            }
        }
        .fixedSize(horizontal: false, vertical: true)
    }

    private var serverControlCard: some View {
        overviewCard {
            HStack {
                Text("Proxy Server")
                    .fontWeight(.semibold)
                Spacer()
                Toggle("", isOn: Binding(
                    get: { pm.isRunning },
                    set: { $0 ? pm.start() : pm.stop() }
                ))
                .toggleStyle(.switch)
                .labelsHidden()
                .controlSize(.small)
            }

            Text("Expose an OpenAI-compatible API endpoint for AI tools.")
                .font(.caption)
                .foregroundStyle(.secondary)

            Spacer(minLength: 0)

            HStack(spacing: 5) {
                StatusDot(pm.isReachable ? .active : (pm.isRunning ? .warning : .error))
                Text(pm.isReachable ? "Server is running" : (pm.isRunning ? "Starting…" : "Server is stopped"))
                    .font(.caption)
                    .foregroundStyle(.secondary)
            }
        }
    }

    private var endpointCard: some View {
        overviewCard {
            HStack {
                Text("Endpoint")
                    .fontWeight(.semibold)
                Spacer()
                Button {
                    NSPasteboard.general.clearContents()
                    NSPasteboard.general.setString(appEnv.baseURL.absoluteString, forType: .string)
                    copiedEndpoint = true
                    Task {
                        try? await Task.sleep(for: .seconds(1.5))
                        copiedEndpoint = false
                    }
                } label: {
                    Image(systemName: copiedEndpoint ? "checkmark" : "doc.on.doc")
                        .font(.caption)
                        .foregroundStyle(copiedEndpoint ? .green : .secondary)
                        .contentTransition(.symbolEffect(.replace))
                }
                .buttonStyle(.plain)
                .help("Copy endpoint URL")
            }

            Text(appEnv.baseURL.absoluteString)
                .font(.system(size: 13, design: .monospaced))
                .foregroundStyle(.secondary)
                .textSelection(.enabled)

            Spacer(minLength: 0)

            HStack(spacing: Spacing.lg) {
                detailItem("Port", value: String(appEnv.port))
                detailItem("Host", value: appEnv.baseURL.host ?? "localhost")
            }
        }
    }

    // MARK: - Usage Section

    private var usageSection: some View {
        VStack(alignment: .leading, spacing: Spacing.sm) {
            HStack(alignment: .firstTextBaseline) {
                SectionLabel("TOKEN USAGE", subtitle: "last \(range.label)")
                Spacer()
                Picker("", selection: $range) {
                    ForEach(UsageRange.allCases) { r in
                        Text(r.shortLabel).tag(r)
                    }
                }
                .labelsHidden()
                .pickerStyle(.segmented)
                .frame(width: 180)
            }

            overviewCard {
                let (totals, points) = aggregateHistory()

                // Headline stats (three columns)
                HStack(spacing: Spacing.xl) {
                    headlineStat(
                        label: "Input",
                        value: formatTokens(totals.input),
                        color: .blue
                    )
                    headlineStat(
                        label: "Output",
                        value: formatTokens(totals.output),
                        color: .purple
                    )
                    headlineStat(
                        label: "Requests",
                        value: "\(totals.requests)",
                        color: .green
                    )
                    Spacer()
                }

                Divider().opacity(0.4)

                if points.isEmpty {
                    usageEmptyState
                } else {
                    UsageHistoryChart(points: points, range: range)
                        .frame(height: 160)
                }
            }
        }
    }

    private var usageEmptyState: some View {
        VStack(spacing: 6) {
            Image(systemName: pm.isReachable ? "chart.bar.doc.horizontal" : "wifi.slash")
                .font(.system(size: 22))
                .foregroundStyle(.tertiary)
            Text(pm.isReachable ? "No usage in this range" : "Waiting for server to record usage")
                .font(.caption)
                .foregroundStyle(.tertiary)
        }
        .frame(maxWidth: .infinity)
        .frame(height: 140)
    }

    private func headlineStat(label: String, value: String, color: Color) -> some View {
        VStack(alignment: .leading, spacing: 2) {
            Text(label)
                .font(.system(size: 10, weight: .medium))
                .foregroundStyle(.tertiary)
                .textCase(.uppercase)
                .kerning(0.5)
            Text(value)
                .font(.system(size: 22, weight: .semibold, design: .rounded))
                .monospacedDigit()
                .foregroundStyle(color)
        }
    }

    // MARK: - Summary (provider/model/session)

    private var summarySection: some View {
        VStack(alignment: .leading, spacing: Spacing.sm) {
            SectionLabel("AT A GLANCE")
                .padding(.top, Spacing.sm)

            HStack(alignment: .top, spacing: Spacing.md) {
                // Provider summary
                overviewCard {
                    Text("Providers")
                        .fontWeight(.semibold)

                    let active = dataService.providers.filter { $0.authStatus == .valid }
                    let total = dataService.providers.count

                    HStack(alignment: .firstTextBaseline, spacing: 2) {
                        Text("\(active.count)")
                            .font(.system(size: 28, weight: .bold, design: .rounded))
                            .monospacedDigit()
                        Text("/ \(total)")
                            .font(.system(size: 14, weight: .medium))
                            .foregroundStyle(.secondary)
                    }

                    if !active.isEmpty {
                        HStack(spacing: Spacing.sm) {
                            ForEach(active, id: \.id) { provider in
                                if let iconName = providerIconName(for: provider.id) {
                                    Image(iconName)
                                        .resizable()
                                        .scaledToFit()
                                        .frame(width: 18, height: 18)
                                        .help(provider.displayName)
                                }
                            }
                        }
                    } else {
                        Text("No authenticated providers")
                            .font(.caption)
                            .foregroundStyle(.tertiary)
                    }
                }

                // Session usage
                overviewCard {
                    Text("Session")
                        .fontWeight(.semibold)

                    if let usage = dataService.usage, usage.totalRequests > 0 {
                        HStack(alignment: .firstTextBaseline, spacing: 2) {
                            Text("\(usage.totalRequests)")
                                .font(.system(size: 28, weight: .bold, design: .rounded))
                                .monospacedDigit()
                            Text("requests")
                                .font(.system(size: 14, weight: .medium))
                                .foregroundStyle(.secondary)
                        }

                        HStack(spacing: Spacing.lg) {
                            detailItem("Input", value: formatTokens(UInt64(usage.inputTokens)))
                            detailItem("Output", value: formatTokens(UInt64(usage.outputTokens)))
                            if usage.totalRequests > 0 {
                                let rate = Double(usage.successRequests) / Double(usage.totalRequests) * 100
                                detailItem("Success", value: String(format: "%.0f%%", rate))
                            }
                        }
                    } else {
                        Text("0")
                            .font(.system(size: 28, weight: .bold, design: .rounded))
                            .monospacedDigit()
                            .foregroundStyle(.tertiary)
                        Text("No requests this session")
                            .font(.caption)
                            .foregroundStyle(.tertiary)
                    }
                }

                // Models summary
                overviewCard {
                    Text("Models")
                        .fontWeight(.semibold)

                    HStack(alignment: .firstTextBaseline, spacing: 2) {
                        Text("\(dataService.models.count)")
                            .font(.system(size: 28, weight: .bold, design: .rounded))
                            .monospacedDigit()
                        Text("available")
                            .font(.system(size: 14, weight: .medium))
                            .foregroundStyle(.secondary)
                    }

                    let providerCount = Set(dataService.models.map(\.owned_by)).count
                    Text("from \(providerCount) provider\(providerCount == 1 ? "" : "s")")
                        .font(.caption)
                        .foregroundStyle(.tertiary)
                }
            }
            .fixedSize(horizontal: false, vertical: true)
        }
    }

    // MARK: - Helpers

    private func aggregateHistory() -> (totals: UsageTotals, points: [UsagePoint]) {
        guard let history = dataService.history else {
            return (UsageTotals(), [])
        }
        var totals = UsageTotals()
        var byBucket: [Int64: UsagePoint] = [:]
        for b in history.buckets {
            totals.input += b.inputTokens
            totals.output += b.outputTokens
            totals.requests += b.requestCount
            var point = byBucket[b.periodStart] ?? UsagePoint(
                date: Date(timeIntervalSince1970: TimeInterval(b.periodStart)),
                input: 0,
                output: 0,
                requests: 0
            )
            point.input += b.inputTokens
            point.output += b.outputTokens
            point.requests += b.requestCount
            byBucket[b.periodStart] = point
        }
        return (totals, byBucket.values.sorted { $0.date < $1.date })
    }

    private func reloadHistory() async {
        let now = Int64(Date().timeIntervalSince1970)
        await dataService.loadHistory(from: now - range.seconds, to: now)
    }

    private func overviewCard<C: View>(@ViewBuilder content: () -> C) -> some View {
        VStack(alignment: .leading, spacing: Spacing.sm) {
            content()
        }
        .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .topLeading)
        .padding(Spacing.lg)
        .cardSurface()
    }

    private func detailItem(_ label: String, value: String) -> some View {
        VStack(spacing: 2) {
            Text(value)
                .font(.system(size: 12, weight: .medium, design: .rounded))
                .monospacedDigit()
            Text(label)
                .font(.system(size: 9, weight: .medium))
                .foregroundStyle(.tertiary)
                .textCase(.uppercase)
        }
    }
}

// MARK: - Types

private struct UsageTotals {
    var input: UInt64 = 0
    var output: UInt64 = 0
    var requests: UInt64 = 0
}

private struct UsagePoint: Identifiable {
    let date: Date
    var input: UInt64
    var output: UInt64
    var requests: UInt64
    var id: Date { date }
}

enum UsageRange: String, CaseIterable, Identifiable {
    case day = "24h"
    case week = "7d"
    case month = "30d"

    var id: Self { self }
    var label: String { self == .day ? "24 hours" : (self == .week ? "7 days" : "30 days") }
    var shortLabel: String { rawValue }
    var seconds: Int64 {
        switch self {
        case .day: 86_400
        case .week: 86_400 * 7
        case .month: 86_400 * 30
        }
    }
}

// MARK: - Chart

private struct UsageHistoryChart: View {
    let points: [UsagePoint]
    let range: UsageRange

    var body: some View {
        Chart {
            ForEach(points) { p in
                BarMark(
                    x: .value("Time", p.date),
                    y: .value("Tokens", p.input)
                )
                .foregroundStyle(by: .value("Type", "Input"))
                .position(by: .value("Type", "Input"))

                BarMark(
                    x: .value("Time", p.date),
                    y: .value("Tokens", p.output)
                )
                .foregroundStyle(by: .value("Type", "Output"))
                .position(by: .value("Type", "Output"))
            }
        }
        .chartForegroundStyleScale([
            "Input": Color.blue.gradient,
            "Output": Color.purple.gradient,
        ])
        .chartXAxis {
            switch range {
            case .day:
                AxisMarks(values: .automatic(desiredCount: 6)) { _ in
                    AxisGridLine()
                    AxisValueLabel(format: .dateTime.hour())
                }
            case .week:
                AxisMarks(values: .stride(by: .day, count: 1)) { _ in
                    AxisGridLine()
                    AxisValueLabel(format: .dateTime.day(.defaultDigits).month(.abbreviated))
                }
            case .month:
                AxisMarks(values: .stride(by: .day, count: 7)) { _ in
                    AxisGridLine()
                    AxisValueLabel(format: .dateTime.day(.defaultDigits).month(.abbreviated))
                }
            }
        }
        .chartYAxis {
            AxisMarks(position: .trailing) { value in
                AxisGridLine()
                AxisValueLabel {
                    if let v = value.as(Int.self) {
                        Text(formatTokens(UInt64(v)))
                            .font(.system(size: 9))
                    }
                }
            }
        }
        .chartLegend(position: .top, alignment: .trailing, spacing: 12)
    }
}

#Preview {
    OverviewView()
        .environment(AppEnvironment.shared)
        .environment(ProcessManager())
        .environment(DataService())
        .frame(width: 900, height: 700)
}
