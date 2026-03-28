import Charts
import SwiftUI

struct GeneralView: View {
    @Environment(ProcessManager.self) private var pm
    @Environment(AppEnvironment.self) private var appEnv
    @Environment(DataService.self) private var dataService
    @State private var endpointCopied = false
    @State private var activityTab: ActivityTab = .providers

    private var usage: UsageSnapshot? { dataService.usage }
    private var history: UsageHistoryResponse? { dataService.history }
    private var providers: [Components.Schemas.ProviderStatus] { dataService.providers }
    private var rateLimits: RateLimitsResponse? { dataService.rateLimits }

    var body: some View {
        ScrollView {
            VStack(spacing: 16) {
                statusBar

                if pm.isReachable {
                    HStack(spacing: 12) {
                        requestsCard
                        tokenCard(
                            title: "INPUT TOKENS",
                            value: usage?.input_tokens ?? 0,
                            color: .indigo,
                            points: tokenTimeSeries(\.input_tokens)
                        )
                        tokenCard(
                            title: "OUTPUT TOKENS",
                            value: usage?.output_tokens ?? 0,
                            color: .cyan,
                            points: tokenTimeSeries(\.output_tokens)
                        )
                    }

                    historyCard
                    activityCard

                    if let rateLimits,
                       rateLimits.providers.contains(where: {
                           $0.accounts.contains(where: { !$0.snapshot.headers.isEmpty })
                       })
                    {
                        rateLimitsCard(rateLimits)
                    }
                }

                if let error = pm.errorMessage {
                    Label(error, systemImage: "exclamationmark.triangle.fill")
                        .foregroundStyle(.red)
                        .font(.caption)
                        .frame(maxWidth: .infinity, alignment: .leading)
                }

                logCard
            }
            .padding(20)
        }
        .navigationTitle("Dashboard")
    }

    // MARK: - Status Bar

    private var statusBar: some View {
        HStack(spacing: 0) {
            statusItem("SERVER") {
                HStack(spacing: 8) {
                    Toggle("", isOn: Binding(
                        get: { pm.isRunning },
                        set: { $0 ? pm.start() : pm.stop() }
                    ))
                    .toggleStyle(.switch)
                    .labelsHidden()
                    .controlSize(.small)

                    HStack(spacing: 5) {
                        Circle()
                            .fill(pm.isReachable ? .green : (pm.isRunning ? .orange : .red))
                            .frame(width: 7, height: 7)
                        Text(
                            pm.isReachable ? "Running" : (pm.isRunning ? "Starting…" : "Stopped")
                        )
                        .fontWeight(.semibold)
                    }
                }
            }

            Spacer()

            if pm.isReachable {
                statusItem("ENDPOINT") {
                    HStack(spacing: 4) {
                        Text(appEnv.baseURL.absoluteString)
                            .fontDesign(.monospaced)

                        Button {
                            NSPasteboard.general.clearContents()
                            NSPasteboard.general.setString(
                                appEnv.baseURL.absoluteString, forType: .string)
                            endpointCopied = true
                            Task {
                                try? await Task.sleep(for: .seconds(1.5))
                                endpointCopied = false
                            }
                        } label: {
                            Image(systemName: endpointCopied ? "checkmark" : "doc.on.doc")
                                .foregroundStyle(endpointCopied ? .green : .secondary)
                                .contentTransition(.symbolEffect(.replace))
                        }
                        .buttonStyle(.borderless)
                    }
                }

                Spacer()

                statusItem("PROVIDERS") {
                    let active = providers.filter { $0.enabled && $0.auth_status == .valid }.count
                    let total = providers.count
                    Text("\(active)/\(total) active")
                        .fontWeight(.semibold)
                }
            }
        }
    }

    private func statusItem<C: View>(
        _ label: String, @ViewBuilder content: () -> C
    ) -> some View {
        VStack(alignment: .leading, spacing: 4) {
            Text(label)
                .font(.system(size: 10, weight: .semibold))
                .foregroundStyle(.tertiary)
            content()
                .font(.subheadline)
        }
    }

    // MARK: - Requests Card

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

    // MARK: - Token Cards

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

    // MARK: - History Card

    private var historyCard: some View {
        Card("REQUEST HISTORY") {
            if history != nil, !aggregated.isEmpty {
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

    // MARK: - Activity Card (tabbed: Providers / Top Models)

    private var activityCard: some View {
        Card("ACTIVITY") {
            Picker("", selection: $activityTab) {
                ForEach(ActivityTab.allCases, id: \.self) { tab in
                    Text(tab.rawValue).tag(tab)
                }
            }
            .pickerStyle(.segmented)
            .labelsHidden()

            switch activityTab {
            case .providers:
                providersList
            case .models:
                topModelsList
            }
        }
    }

    private var providersList: some View {
        VStack(spacing: 6) {
            if providers.isEmpty {
                Text("No providers")
                    .font(.caption)
                    .foregroundStyle(.tertiary)
                    .frame(maxWidth: .infinity, alignment: .center)
                    .padding(.vertical, 8)
            } else {
                ForEach(providers, id: \.id) { p in
                    HStack(spacing: 8) {
                        Circle()
                            .fill(providerColor(p))
                            .frame(width: 7, height: 7)
                        Text(p.display_name)
                            .lineLimit(1)
                        Spacer()
                        Text("\(p.models_count) models")
                            .foregroundStyle(.tertiary)
                            .monospacedDigit()
                    }
                    .font(.caption)
                    .opacity(p.enabled ? 1 : 0.5)
                }
            }
        }
    }

    private var topModelsList: some View {
        Group {
            let sorted = (usage?.models ?? [:])
                .sorted { $0.value.requests > $1.value.requests }
                .prefix(8)

            if sorted.isEmpty {
                Text("No model usage yet")
                    .font(.caption)
                    .foregroundStyle(.tertiary)
                    .frame(maxWidth: .infinity, alignment: .center)
                    .padding(.vertical, 8)
            } else {
                let maxReqs = sorted.first?.value.requests ?? 1

                VStack(spacing: 6) {
                    ForEach(Array(sorted), id: \.key) { model, stats in
                        VStack(spacing: 3) {
                            HStack {
                                Text(model)
                                    .lineLimit(1)
                                    .truncationMode(.middle)
                                Spacer()
                                Text("\(stats.requests) req")
                                    .foregroundStyle(.secondary)
                                    .monospacedDigit()
                            }
                            .font(.caption)

                            GeometryReader { geo in
                                RoundedRectangle(cornerRadius: 2)
                                    .fill(.blue.gradient.opacity(0.3))
                                    .frame(
                                        width: geo.size.width
                                            * CGFloat(stats.requests)
                                            / CGFloat(max(maxReqs, 1))
                                    )
                            }
                            .frame(height: 4)
                        }
                    }
                }
            }
        }
    }

    // MARK: - Rate Limits Card

    private func rateLimitsCard(_ data: RateLimitsResponse) -> some View {
        Card("RATE LIMITS") {
            VStack(spacing: 8) {
                ForEach(data.providers, id: \.id) { provider in
                    ForEach(provider.accounts, id: \.account_id) { account in
                        if !account.snapshot.headers.isEmpty {
                            rateLimitRow(
                                name: provider.display_name,
                                multiAccount: provider.accounts.count > 1,
                                accountId: account.account_id,
                                headers: account.snapshot.headers,
                                capturedAt: account.snapshot.captured_at
                            )
                        }
                    }
                }
            }
        }
    }

    private func rateLimitRow(
        name: String, multiAccount: Bool, accountId: String,
        headers: [String: String], capturedAt: UInt64
    ) -> some View {
        let remaining = findHeader(headers, "remaining")
        let limit = findHeader(headers, "limit")

        return VStack(alignment: .leading, spacing: 4) {
            HStack {
                Text(name)
                    .fontWeight(.medium)
                if multiAccount {
                    Text("(\(accountId))")
                        .foregroundStyle(.tertiary)
                }
                Spacer()
                Text(timeAgo(capturedAt))
                    .foregroundStyle(.tertiary)
            }
            .font(.caption)

            if let remaining, let limit,
               let r = Double(remaining), let l = Double(limit), l > 0
            {
                HStack(spacing: 8) {
                    GeometryReader { geo in
                        ZStack(alignment: .leading) {
                            RoundedRectangle(cornerRadius: 3)
                                .fill(.quaternary)
                            RoundedRectangle(cornerRadius: 3)
                                .fill(ratioColor(r / l).gradient)
                                .frame(width: geo.size.width * r / l)
                        }
                    }
                    .frame(height: 6)

                    Text("\(remaining)/\(limit)")
                        .font(.caption2)
                        .foregroundStyle(.secondary)
                        .monospacedDigit()
                }
            }
        }
    }

    // MARK: - Log Card

    private var logCard: some View {
        Card("LOG") {
            ScrollViewReader { proxy in
                ScrollView(.vertical, showsIndicators: false) {
                    if pm.logs.isEmpty {
                        Text("Waiting for log entries…")
                            .font(.system(size: 11, design: .monospaced))
                            .foregroundStyle(.tertiary)
                            .frame(maxWidth: .infinity, alignment: .leading)
                    } else {
                        VStack(alignment: .leading, spacing: 0) {
                            ForEach(Array(pm.logs.enumerated()), id: \.offset) { i, line in
                                Text(AnsiParser.parse(line))
                                    .font(.system(size: 11, design: .monospaced))
                                    .lineLimit(1)
                                    .truncationMode(.tail)
                                    .textSelection(.enabled)
                                    .frame(maxWidth: .infinity, alignment: .leading)
                                    .id(i)
                            }
                        }
                    }
                }
                .frame(height: 64)
                .onChange(of: pm.logs.count) {
                    proxy.scrollTo(pm.logs.count - 1, anchor: .bottom)
                }
            }

            Divider()

            HStack(spacing: 12) {
                Text("\(pm.logs.count) lines")
                    .foregroundStyle(.tertiary)
                    .monospacedDigit()
                Spacer()
                Button("Clear", systemImage: "trash") {
                    pm.clearLogs()
                }
                .buttonStyle(.borderless)
                .labelStyle(.iconOnly)
            }
            .font(.caption2)
        }
    }

    // MARK: - Computed

    private func tokenTimeSeries(
        _ keyPath: KeyPath<UsageBucket, UInt64>
    ) -> [(date: Date, value: UInt64)] {
        guard let history else { return [] }
        return Dictionary(grouping: history.buckets, by: \.period_start)
            .map { ts, buckets in
                (date: Date(timeIntervalSince1970: TimeInterval(ts)),
                 value: buckets.reduce(0) { $0 + $1[keyPath: keyPath] })
            }
            .sorted { $0.date < $1.date }
    }

    private var aggregated: [(date: Date, requests: UInt64)] {
        guard let history else { return [] }
        return Dictionary(grouping: history.buckets, by: \.period_start)
            .map { ts, buckets in
                (date: Date(timeIntervalSince1970: TimeInterval(ts)),
                 requests: buckets.reduce(0) { $0 + $1.request_count })
            }
            .sorted { $0.date < $1.date }
    }

    // MARK: - Helpers

    private func providerColor(_ p: Components.Schemas.ProviderStatus) -> Color {
        switch p.auth_status {
        case .valid: .green
        case .expired: .orange
        case .not_configured: .gray
        }
    }

    private func findHeader(_ headers: [String: String], _ keyword: String) -> String? {
        headers.first(where: {
            $0.key.localizedCaseInsensitiveContains(keyword)
                && $0.key.localizedCaseInsensitiveContains("request")
        })?.value
    }

    private func ratioColor(_ ratio: Double) -> Color {
        if ratio > 0.5 { .green }
        else if ratio > 0.2 { .orange }
        else { .red }
    }

    private func timeAgo(_ ts: UInt64) -> String {
        let elapsed = Int64(Date().timeIntervalSince1970) - Int64(ts)
        if elapsed < 60 { return "just now" }
        if elapsed < 3600 { return "\(elapsed / 60)m ago" }
        return "\(elapsed / 3600)h ago"
    }
}

// MARK: - Reusable Components

private struct Card<Content: View>: View {
    let title: String
    @ViewBuilder var content: Content

    init(_ title: String, @ViewBuilder content: () -> Content) {
        self.title = title
        self.content = content()
    }

    var body: some View {
        VStack(alignment: .leading, spacing: 10) {
            Text(title)
                .font(.system(size: 10, weight: .semibold))
                .foregroundStyle(.secondary)
                .kerning(0.5)

            content
        }
        .frame(maxWidth: .infinity, alignment: .leading)
        .padding(16)
        .background(.background, in: .rect(cornerRadius: 10))
    }
}

private struct HeroNumber: View {
    let value: UInt64

    var body: some View {
        Text("\(value)")
            .font(.system(size: 34, weight: .bold, design: .rounded))
            .monospacedDigit()
    }
}

private enum ActivityTab: String, CaseIterable {
    case providers = "Providers"
    case models = "Models"
}

// MARK: - Formatting

private func formatTokenParts(_ count: UInt64) -> (String, String) {
    switch count {
    case 0..<1_000:
        return ("\(count)", "")
    case 1_000..<1_000_000:
        return (String(format: "%.1f", Double(count) / 1_000), "K")
    default:
        return (String(format: "%.2f", Double(count) / 1_000_000), "M")
    }
}

private func formatTokens(_ count: UInt64) -> String {
    let (num, unit) = formatTokenParts(count)
    return "\(num)\(unit)"
}

#Preview {
    GeneralView()
        .environment(AppEnvironment.shared)
        .environment(ProcessManager())
        .environment(DataService())
        .frame(width: 640, height: 600)
}
