import SwiftUI

private struct PendingRemoval: Identifiable {
    let providerId: String
    let providerName: String
    let accountId: String
    let accountLabel: String
    var id: String { "\(providerId)/\(accountId)" }
}

struct AccountsView: View {
    @Environment(ProcessManager.self) private var pm
    @Environment(DataService.self) private var dataService
    @State private var loginInProgress: String?
    @State private var errorMessage: String?
    @State private var hoveredProvider: String?
    @State private var pendingRemoval: PendingRemoval?

    var body: some View {
        DetailPage("Accounts") {
            if pm.isReachable {
                if dataService.providerAccounts.isEmpty {
                    if dataService.isLoading {
                        loadingState
                    } else {
                        ContentUnavailableView(
                            "No providers available",
                            systemImage: "person.crop.circle.badge.questionmark",
                            description: Text("The server returned no provider list. Check that the proxy is running and reachable.")
                        )
                    }
                } else {
                    ScrollView {
                        LazyVStack(spacing: 10) {
                            ForEach(dataService.providerAccounts, id: \.id) { provider in
                                providerCard(provider)
                            }
                        }
                    }
                }

                if let errorMessage {
                    HStack(spacing: 6) {
                        Image(systemName: "exclamationmark.triangle.fill")
                            .foregroundStyle(.red)
                        Text(errorMessage)
                    }
                    .font(.caption)
                    .foregroundStyle(.red)
                    .padding(.horizontal, 16)
                    .padding(.vertical, 10)
                    .frame(maxWidth: .infinity, alignment: .leading)
                    .background(.red.opacity(0.08), in: .rect(cornerRadius: 10))
                }
            } else if pm.isRunning {
                ServerStartingView()
            } else {
                Spacer()
                ContentUnavailableView(
                    "Server Not Running",
                    systemImage: "server.rack",
                    description: Text("Enable the proxy server to manage accounts.")
                )
                Spacer()
            }
        }
        .alert(
            "Remove account?",
            isPresented: Binding(
                get: { pendingRemoval != nil },
                set: { if !$0 { pendingRemoval = nil } }
            ),
            presenting: pendingRemoval
        ) { removal in
            Button("Remove", role: .destructive) {
                let captured = removal
                pendingRemoval = nil
                Task {
                    await removeAccount(
                        provider: captured.providerId,
                        accountId: captured.accountId
                    )
                }
            }
            Button("Cancel", role: .cancel) {
                pendingRemoval = nil
            }
        } message: { removal in
            Text("This will sign out \(removal.accountLabel) from \(removal.providerName). You'll need to re-authenticate to use it again.")
        }
    }

    // MARK: - Provider Card

    @ViewBuilder
    private func providerCard(_ provider: Byokey_Accounts_ProviderAccounts) -> some View {
        let isHovered = hoveredProvider == provider.id
        let stats = providerStats(for: provider.id)

        VStack(alignment: .leading, spacing: 0) {
            // Header
            HStack(spacing: 10) {
                if let iconName = providerIconName(for: provider.id) {
                    Image(iconName)
                        .resizable()
                        .scaledToFit()
                        .frame(width: 22, height: 22)
                }

                Text(provider.displayName)
                    .font(.system(size: 13, weight: .semibold))

                Spacer()

                Text("\(provider.accounts.count)")
                    .font(.system(size: 11, weight: .medium, design: .rounded))
                    .monospacedDigit()
                    .foregroundStyle(.secondary)
                    .padding(.horizontal, 7)
                    .padding(.vertical, 2)
                    .background(.secondary.opacity(0.1), in: Capsule())
            }
            .padding(.horizontal, 16)
            .padding(.vertical, 12)

            // Provider-level usage stats
            if let stats, stats.requests > 0 {
                Divider()
                    .padding(.horizontal, 16)

                HStack(spacing: 16) {
                    statItem(label: "Requests", value: "\(stats.requests)")
                    statItem(label: "Input", value: formatTokens(UInt64(stats.inputTokens)))
                    statItem(label: "Output", value: formatTokens(UInt64(stats.outputTokens)))
                    if stats.requests > 0 {
                        statItem(
                            label: "Success",
                            value: "\(Int(Double(stats.success) / Double(stats.requests) * 100))%"
                        )
                    }
                }
                .padding(.horizontal, 16)
                .padding(.vertical, 10)
            }

            if !provider.accounts.isEmpty {
                Divider()
                    .padding(.horizontal, 16)

                VStack(spacing: 0) {
                    ForEach(Array(provider.accounts.enumerated()), id: \.element.accountID) { index, account in
                        AccountRow(
                            account: account,
                            providerName: provider.displayName,
                            usage: accountStats(providerId: provider.id, accountId: account.accountID),
                            rateLimitHeaders: rateLimitHeaders(providerId: provider.id, accountId: account.accountID),
                            onActivate: {
                                Task { await activateAccount(provider: provider.id, accountId: account.accountID) }
                            },
                            onRemove: {
                                pendingRemoval = PendingRemoval(
                                    providerId: provider.id,
                                    providerName: provider.displayName,
                                    accountId: account.accountID,
                                    accountLabel: account.hasLabel && !account.label.isEmpty
                                        ? account.label
                                        : account.accountID
                                )
                            }
                        )

                        if index < provider.accounts.count - 1 {
                            Divider()
                                .padding(.leading, 44)
                                .padding(.trailing, 16)
                        }
                    }
                }
            }

            // Login button
            Divider()
                .padding(.horizontal, 16)

            Button {
                Task { await login(provider: provider.id) }
            } label: {
                HStack(spacing: 6) {
                    if loginInProgress == provider.id {
                        ProgressView()
                            .controlSize(.mini)
                    } else {
                        Image(systemName: "plus.circle.fill")
                            .foregroundStyle(Color.accentColor.opacity(0.7))
                    }
                    Text(provider.accounts.isEmpty ? "Login" : "Add Account")
                        .font(.system(size: 12, weight: .medium))
                        .foregroundStyle(Color.accentColor.opacity(0.9))
                }
                .frame(maxWidth: .infinity, alignment: .center)
                .padding(.vertical, 10)
            }
            .buttonStyle(.plain)
            .disabled(loginInProgress != nil)
        }
        .cardSurface(isHovered: isHovered)
        .animation(.easeOut(duration: 0.15), value: isHovered)
        .onHover { hovering in
            hoveredProvider = hovering ? provider.id : nil
        }
    }

    private func statItem(label: String, value: String) -> some View {
        VStack(spacing: 2) {
            Text(value)
                .font(.system(size: 13, weight: .semibold, design: .rounded))
                .monospacedDigit()
            Text(label)
                .font(.system(size: 9, weight: .medium))
                .foregroundStyle(.tertiary)
                .textCase(.uppercase)
        }
        .frame(maxWidth: .infinity)
    }

    private var loadingState: some View {
        VStack(spacing: 8) {
            ProgressView().controlSize(.regular)
            Text("Loading accounts…")
                .font(.caption)
                .foregroundStyle(.secondary)
        }
        .frame(maxWidth: .infinity)
        .padding(.vertical, 40)
    }

    // MARK: - Data Helpers

    private func providerStats(for providerId: String) -> ProviderAggregateStats? {
        // Prefer precise per-account aggregation when available.
        let rows = dataService.accountUsage.filter { $0.provider == providerId }
        if !rows.isEmpty {
            var agg = ProviderAggregateStats()
            for r in rows {
                agg.requests += r.requestCount
                agg.success += r.successCount
                agg.inputTokens += r.inputTokens
                agg.outputTokens += r.outputTokens
            }
            return agg.requests > 0 ? agg : nil
        }

        // Fallback: model-level stats mapped back through `models`.
        guard let modelStats = dataService.usage?.models else { return nil }
        let modelToProvider: [String: String] = Dictionary(
            dataService.models.map { ($0.id, $0.owned_by) },
            uniquingKeysWith: { first, _ in first }
        )

        var agg = ProviderAggregateStats()
        for (modelId, stats) in modelStats {
            if modelToProvider[modelId] == providerId {
                agg.requests += stats.requests
                agg.success += stats.success
                agg.inputTokens += stats.inputTokens
                agg.outputTokens += stats.outputTokens
            }
        }
        return agg.requests > 0 ? agg : nil
    }

    /// Per-account aggregate (summed across all models for this account).
    private func accountStats(providerId: String, accountId: String) -> ProviderAggregateStats? {
        let rows = dataService.accountUsage.filter {
            $0.provider == providerId && $0.accountID == accountId
        }
        guard !rows.isEmpty else { return nil }
        var agg = ProviderAggregateStats()
        for r in rows {
            agg.requests += r.requestCount
            agg.success += r.successCount
            agg.inputTokens += r.inputTokens
            agg.outputTokens += r.outputTokens
        }
        return agg.requests > 0 ? agg : nil
    }

    private func rateLimitHeaders(providerId: String, accountId: String) -> [String: String]? {
        guard let rateLimits = dataService.rateLimits else { return nil }
        guard let provider = rateLimits.providers.first(where: { $0.id == providerId }) else { return nil }
        guard let account = provider.accounts.first(where: { $0.accountID == accountId }) else { return nil }
        guard account.hasSnapshot else { return nil }
        let headers = account.snapshot.headers
        return headers.isEmpty ? nil : headers
    }

    // MARK: - Actions

    private func activateAccount(provider: String, accountId: String) async {
        do {
            try await dataService.activateAccount(provider: provider, accountId: accountId)
            errorMessage = nil
        } catch {
            errorMessage = "Failed to activate: \(error.localizedDescription)"
        }
    }

    private func removeAccount(provider: String, accountId: String) async {
        do {
            try await dataService.removeAccount(provider: provider, accountId: accountId)
            errorMessage = nil
        } catch {
            errorMessage = "Failed to remove: \(error.localizedDescription)"
        }
    }

    private func login(provider: String) async {
        loginInProgress = provider
        errorMessage = nil
        do {
            try await CLIRunner.login(provider: provider)
            try? await Task.sleep(for: .seconds(1))
            await dataService.reloadAccounts()
        } catch {
            errorMessage = "Login failed: \(error.localizedDescription)"
        }
        loginInProgress = nil
    }
}

// MARK: - Per-(provider, account) aggregate usage

struct ProviderAggregateStats: Equatable {
    var requests: UInt64 = 0
    var success: UInt64 = 0
    var inputTokens: UInt64 = 0
    var outputTokens: UInt64 = 0
}

// MARK: - Account Row

private struct AccountRow: View {
    let account: Byokey_Accounts_AccountDetail
    let providerName: String
    let usage: ProviderAggregateStats?
    let rateLimitHeaders: [String: String]?
    let onActivate: () -> Void
    let onRemove: () -> Void
    @State private var isHovered = false
    @State private var isExpanded = false

    private var displayName: String {
        if account.hasLabel, !account.label.isEmpty, account.label != providerName {
            return account.label
        }
        return account.accountID
    }

    var body: some View {
        VStack(spacing: 0) {
            // Main row
            HStack(spacing: 10) {
                // Active indicator
                Button(action: onActivate) {
                    ZStack {
                        Circle()
                            .strokeBorder(account.isActive ? Color.accentColor : .secondary.opacity(0.3), lineWidth: 1.5)
                            .frame(width: 16, height: 16)

                        if account.isActive {
                            Circle()
                                .fill(Color.accentColor)
                                .frame(width: 8, height: 8)
                        }
                    }
                }
                .buttonStyle(.plain)
                .disabled(account.isActive)
                .accessibilityLabel(account.isActive ? "Active account" : "Set as active account")

                // Name
                Text(displayName)
                    .font(.system(size: 12))
                    .lineLimit(1)

                Spacer()

                // Status badge
                HStack(spacing: 4) {
                    Circle()
                        .fill(stateColor)
                        .frame(width: 5, height: 5)
                    Text(stateLabel)
                        .font(.system(size: 10, weight: .medium))
                        .foregroundStyle(stateColor)
                }
                .padding(.horizontal, 8)
                .padding(.vertical, 3)
                .background(stateColor.opacity(0.1), in: Capsule())

                // Expiry
                if let remaining = remainingText {
                    Text(remaining)
                        .font(.system(size: 10, design: .rounded))
                        .monospacedDigit()
                        .foregroundStyle(.tertiary)
                }

                // Expand toggle (if rate limits available)
                if rateLimitHeaders != nil {
                    Button {
                        withAnimation(.easeInOut(duration: 0.15)) {
                            isExpanded.toggle()
                        }
                    } label: {
                        Image(systemName: "chevron.right")
                            .font(.system(size: 9, weight: .semibold))
                            .foregroundStyle(.tertiary)
                            .rotationEffect(.degrees(isExpanded ? 90 : 0))
                    }
                    .buttonStyle(.plain)
                }

                // Remove
                Button(role: .destructive, action: onRemove) {
                    Image(systemName: "trash")
                        .font(.system(size: 10))
                        .foregroundStyle(isHovered ? .red.opacity(0.7) : .secondary.opacity(0.4))
                }
                .buttonStyle(.plain)
            }
            .padding(.horizontal, 16)
            .padding(.vertical, 8)
            .contentShape(Rectangle())
            .onTapGesture {
                if rateLimitHeaders != nil {
                    withAnimation(.easeInOut(duration: 0.15)) {
                        isExpanded.toggle()
                    }
                }
            }
            .onHover { hovering in isHovered = hovering }

            // Compact per-account usage strip
            if let usage, usage.requests > 0 {
                HStack(spacing: 12) {
                    usageStat("Req", "\(usage.requests)")
                    usageStat("In", formatTokens(usage.inputTokens))
                    usageStat("Out", formatTokens(usage.outputTokens))
                    if usage.requests > 0 {
                        usageStat(
                            "OK",
                            "\(Int(Double(usage.success) / Double(usage.requests) * 100))%"
                        )
                    }
                    Spacer()
                }
                .padding(.horizontal, 44)
                .padding(.bottom, 8)
            }

            // Expanded rate limit detail
            if isExpanded, let headers = rateLimitHeaders {
                rateLimitDetail(headers)
                    .transition(.opacity.combined(with: .move(edge: .top)))
            }
        }
    }

    private func usageStat(_ label: String, _ value: String) -> some View {
        HStack(spacing: 3) {
            Text(label)
                .font(.system(size: 9, weight: .medium))
                .foregroundStyle(.tertiary)
                .textCase(.uppercase)
            Text(value)
                .font(.system(size: 10, weight: .medium, design: .rounded))
                .foregroundStyle(.secondary)
                .monospacedDigit()
        }
    }

    @ViewBuilder
    private func rateLimitDetail(_ headers: [String: String]) -> some View {
        let requestRemaining = findHeader(headers, "remaining", "request")
        let requestLimit = findHeader(headers, "limit", "request")
        let tokenRemaining = findHeader(headers, "remaining", "token")
        let tokenLimit = findHeader(headers, "limit", "token")

        VStack(spacing: 8) {
            if let remaining = requestRemaining, let limit = requestLimit,
               let r = Double(remaining), let l = Double(limit), l > 0
            {
                rateLimitBar(label: "Requests", remaining: r, limit: l, remainingText: remaining, limitText: limit)
            }

            if let remaining = tokenRemaining, let limit = tokenLimit,
               let r = Double(remaining), let l = Double(limit), l > 0
            {
                rateLimitBar(label: "Tokens", remaining: r, limit: l, remainingText: remaining, limitText: limit)
            }

            // Show any other rate limit headers as key-value pairs
            let knownKeys = Set(headers.keys.filter { key in
                let k = key.lowercased()
                return (k.contains("remaining") || k.contains("limit"))
                    && (k.contains("request") || k.contains("token"))
            })
            let otherHeaders = headers.filter { !knownKeys.contains($0.key) }
            if !otherHeaders.isEmpty {
                VStack(spacing: 3) {
                    ForEach(otherHeaders.sorted(by: { $0.key < $1.key }), id: \.key) { key, value in
                        HStack {
                            Text(key)
                                .foregroundStyle(.tertiary)
                            Spacer()
                            Text(value)
                                .foregroundStyle(.secondary)
                                .monospacedDigit()
                        }
                        .font(.system(size: 10))
                    }
                }
            }
        }
        .padding(.horizontal, 44)
        .padding(.bottom, 10)
    }

    private func rateLimitBar(label: String, remaining: Double, limit: Double, remainingText: String, limitText: String) -> some View {
        VStack(spacing: 3) {
            HStack {
                Text(label)
                    .font(.system(size: 10, weight: .medium))
                    .foregroundStyle(.secondary)
                Spacer()
                Text("\(remainingText) / \(limitText)")
                    .font(.system(size: 10))
                    .foregroundStyle(.tertiary)
                    .monospacedDigit()
            }

            GeometryReader { geo in
                ZStack(alignment: .leading) {
                    RoundedRectangle(cornerRadius: 3)
                        .fill(.quaternary)
                    RoundedRectangle(cornerRadius: 3)
                        .fill(ratioColor(remaining / limit).gradient)
                        .frame(width: geo.size.width * min(remaining / limit, 1.0))
                }
            }
            .frame(height: 5)
        }
    }

    private func findHeader(_ headers: [String: String], _ keyword1: String, _ keyword2: String) -> String? {
        headers.first(where: {
            let k = $0.key.lowercased()
            return k.contains(keyword1) && k.contains(keyword2)
        })?.value
    }

    private func ratioColor(_ ratio: Double) -> Color {
        if ratio > 0.5 { .green }
        else if ratio > 0.2 { .orange }
        else { .red }
    }

    private var stateColor: Color {
        switch account.tokenState {
        case .valid: .green
        case .expired: .orange
        default: .red
        }
    }

    private var stateLabel: String {
        switch account.tokenState {
        case .valid: "Active"
        case .expired: "Expired"
        default: "Invalid"
        }
    }

    private var remainingText: String? {
        guard account.hasExpiresAt else { return nil }
        let now = UInt64(Date().timeIntervalSince1970)
        guard account.expiresAt > now else { return nil }
        let remaining = account.expiresAt - now

        let days = remaining / 86400
        let hours = (remaining % 86400) / 3600

        if days > 0 {
            return "\(days)d"
        } else if hours > 0 {
            return "\(hours)h"
        } else {
            return "<1h"
        }
    }
}

#Preview {
    AccountsView()
        .environment(ProcessManager())
        .environment(DataService())
}
