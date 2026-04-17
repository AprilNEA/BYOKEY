import SwiftUI

// MARK: - Model Family

/// A model family that Amp uses. The user configures which Provider routes it.
private struct ModelFamily: Identifiable {
    let id: String        // config key: "claude", "codex", "gemini"
    let name: String      // "Claude", "GPT", "Gemini"
    let examples: String  // example model names
    let icon: String      // asset catalog image name
    let color: Color
}

private let modelFamilies: [ModelFamily] = [
    .init(id: "claude", name: "Claude", examples: "sonnet · opus · haiku", icon: "provider-claude", color: .orange),
    .init(id: "codex", name: "GPT", examples: "gpt-4o · o3 · codex", icon: "provider-codex", color: .green),
    .init(id: "gemini", name: "Gemini", examples: "2.5-pro · flash", icon: "provider-gemini", color: .blue),
]

// MARK: - Routing Choice

/// A single entry the user can pick from the per-family dropdown.
private enum RoutingChoice: Hashable {
    case ampOfficial                                   // passthrough, no interception
    case provider(id: String)                          // route via provider, use its active account
    case account(providerId: String, accountId: String)// route via provider, pin specific account
}

// MARK: - View

struct AmpView: View {
    @Environment(ProcessManager.self) private var pm
    @Environment(AppEnvironment.self) private var appEnv
    @Environment(DataService.self) private var dataService
    @State private var isInjecting = false
    @State private var isTogglingAds = false
    @State private var resultMessage: ResultMessage?
    @State private var injectionStatus: InjectionStatus = .unknown

    /// Which provider routes each model family. nil = Amp Official (default).
    @State private var routing: [String: String] = [:]  // family.id → provider id

    private var proxyURL: String {
        "\(appEnv.baseURL.absoluteString)/amp"
    }

    var body: some View {
        DetailPage("Amp") {
            if pm.isReachable {
            VStack(spacing: 20) {
                // ── Model Routing ────────────────────────────
                sectionHeader("MODEL ROUTING", subtitle: "Pick which provider account handles each model family. Changes apply live.")

                HStack(alignment: .top, spacing: 12) {
                    ForEach(modelFamilies) { family in
                        modelRoutingCard(family)
                    }
                }

                Divider().padding(.vertical, 4)

                // ── Quick Actions ────────────────────────────
                sectionHeader("SETUP", subtitle: nil)

                HStack(spacing: 12) {
                    quickActionCard(
                        title: "Proxy Injection",
                        subtitle: injectionSubtitle,
                        statusColor: injectionStatusColor,
                        icon: "arrow.triangle.branch",
                        actionLabel: "Inject",
                        isLoading: isInjecting
                    ) {
                        Task { await inject() }
                    }

                    quickActionCard(
                        title: "Ads Control",
                        subtitle: "Patch Amp CLI & extensions",
                        statusColor: nil,
                        icon: "eye.slash",
                        actionLabel: "Disable Ads",
                        isLoading: isTogglingAds
                    ) {
                        Task { await toggleAds(disable: true) }
                    }
                }

                if let result = resultMessage {
                    resultBanner(result)
                        .transition(.opacity.combined(with: .move(edge: .top)))
                }

                Spacer(minLength: 0)
            }
            } else if pm.isRunning {
                ServerStartingView()
            } else {
                Spacer()
                ContentUnavailableView(
                    "Server Not Running",
                    systemImage: "bolt.fill",
                    description: Text("Enable the proxy server to configure Amp.")
                )
                Spacer()
            }
        }
        .onAppear {
            checkInjectionStatus()
            loadRouting()
        }
    }

    // MARK: - Section Header

    private func sectionHeader(_ title: String, subtitle: String?) -> some View {
        SectionLabel(title, subtitle: subtitle)
            .frame(maxWidth: .infinity, alignment: .leading)
    }

    // MARK: - Model Routing Card

    private func modelRoutingCard(_ family: ModelFamily) -> some View {
        let current = currentChoice(family: family)
        let (label, accent) = displayLabel(for: current, family: family)

        return VStack(alignment: .leading, spacing: 12) {
            // Model family header
            HStack(spacing: 10) {
                Image(family.icon)
                    .resizable()
                    .scaledToFit()
                    .frame(width: 18, height: 18)
                    .frame(width: 30, height: 30)
                    .background(family.color.opacity(0.1), in: .rect(cornerRadius: 8))

                VStack(alignment: .leading, spacing: 1) {
                    Text(family.name)
                        .font(.system(size: 13, weight: .semibold))
                    Text(family.examples)
                        .font(.system(size: 9))
                        .foregroundStyle(.tertiary)
                        .lineLimit(1)
                }
            }

            // Provider + account picker
            Menu {
                // Amp Official (passthrough — no interception)
                Button {
                    Task { await apply(choice: .ampOfficial, family: family.id) }
                } label: {
                    HStack {
                        Text("Amp Official")
                        if case .ampOfficial = current { Image(systemName: "checkmark") }
                    }
                }

                // One submenu per provider
                ForEach(dataService.providers.filter { $0.id != "amp" }, id: \.id) { provider in
                    let accounts = dataService.providerAccounts
                        .first(where: { $0.id == provider.id })?
                        .accounts ?? []

                    if accounts.isEmpty {
                        // No accounts — show as disabled-ish button that still routes to active default
                        Button {
                            Task { await apply(choice: .provider(id: provider.id), family: family.id) }
                        } label: {
                            providerMenuLabel(provider: provider, current: current)
                        }
                    } else {
                        Menu {
                            // "Default / Active" option
                            Button {
                                Task { await apply(choice: .provider(id: provider.id), family: family.id) }
                            } label: {
                                HStack {
                                    Text("Active account (default)")
                                    if case .provider(let pid) = current, pid == provider.id {
                                        Image(systemName: "checkmark")
                                    }
                                }
                            }
                            Divider()
                            ForEach(accounts, id: \.accountID) { account in
                                Button {
                                    Task {
                                        await apply(
                                            choice: .account(providerId: provider.id, accountId: account.accountID),
                                            family: family.id
                                        )
                                    }
                                } label: {
                                    HStack {
                                        accountStatusDot(account)
                                        Text(accountDisplayName(account))
                                        if case .account(let pid, let aid) = current,
                                           pid == provider.id, aid == account.accountID
                                        {
                                            Image(systemName: "checkmark")
                                        }
                                    }
                                }
                            }
                        } label: {
                            providerMenuLabel(provider: provider, current: current)
                        }
                    }
                }
            } label: {
                HStack(spacing: 6) {
                    Circle()
                        .fill(accent)
                        .frame(width: 6, height: 6)

                    Text(label)
                        .font(.system(size: 11))
                        .lineLimit(1)
                        .foregroundStyle(accent == .secondary ? .secondary : .primary)

                    Spacer()

                    Image(systemName: "chevron.up.chevron.down")
                        .font(.system(size: 8))
                        .foregroundStyle(.tertiary)
                }
                .padding(.horizontal, 10)
                .padding(.vertical, 6)
                .background(Color.secondary.opacity(0.06), in: .rect(cornerRadius: 8))
            }
            .menuStyle(.borderlessButton)
        }
        .frame(maxWidth: .infinity, alignment: .leading)
        .padding(14)
        .cardSurface()
    }

    @ViewBuilder
    private func providerMenuLabel(provider: Byokey_Status_ProviderStatus, current: RoutingChoice) -> some View {
        HStack {
            Circle()
                .fill(provider.authStatus == .valid ? .green : .gray)
                .frame(width: 6, height: 6)
            Text(provider.displayName)
            if case .provider(let pid) = current, pid == provider.id {
                Image(systemName: "checkmark")
            }
            if case .account(let pid, _) = current, pid == provider.id {
                Image(systemName: "checkmark")
            }
        }
    }

    private func accountStatusDot(_ account: Byokey_Accounts_AccountDetail) -> some View {
        let color: Color = switch account.tokenState {
        case .valid: .green
        case .expired: .orange
        default: .red
        }
        return Circle().fill(color).frame(width: 5, height: 5)
    }

    private func accountDisplayName(_ account: Byokey_Accounts_AccountDetail) -> String {
        if account.hasLabel, !account.label.isEmpty {
            return account.label
        }
        return account.accountID
    }

    // MARK: - Current Choice & Display

    private func currentChoice(family: ModelFamily) -> RoutingChoice {
        guard let providerId = routing[family.id] else { return .ampOfficial }
        // Return the user's *pinned* choice as stored in settings.json.
        // The routing dict stores only a provider ID (no pinned account ID), so
        // this is always .provider — never .account — unless we later add per-
        // account pinning to the config format.  Do NOT fall through to the live
        // active account: that would make the picker reflect the router's runtime
        // decision instead of the user's stored preference.
        if let pinnedAccountId = pinnedAccountId(for: providerId) {
            return .account(providerId: providerId, accountId: pinnedAccountId)
        }
        return .provider(id: providerId)
    }

    /// Returns the explicitly-pinned account ID for a provider, if one was
    /// saved to settings.json.  Returns nil when the user chose "active account
    /// (default)" — i.e. only the provider is pinned, not a specific account.
    private func pinnedAccountId(for providerId: String) -> String? {
        guard let data = try? Data(contentsOf: configURL),
              let json = try? JSONSerialization.jsonObject(with: data) as? [String: Any],
              let providers = json["providers"] as? [String: Any]
        else { return nil }

        for family in modelFamilies {
            if let conf = providers[family.id] as? [String: Any],
               let backend = conf["backend"] as? String,
               backend == providerId,
               let accountId = conf["account_id"] as? String
            {
                return accountId
            }
        }
        return nil
    }

    private func displayLabel(for choice: RoutingChoice, family: ModelFamily) -> (String, Color) {
        switch choice {
        case .ampOfficial:
            return ("Amp Official", .secondary)
        case .provider(let id):
            let provider = dataService.providers.first(where: { $0.id == id })
            let name = provider?.displayName ?? id
            return ("\(name) (active)", provider?.authStatus == .valid ? .green : .gray)
        case .account(let pid, let aid):
            let provider = dataService.providers.first(where: { $0.id == pid })
            let name = provider?.displayName ?? pid
            let account = dataService.providerAccounts
                .first(where: { $0.id == pid })?
                .accounts.first(where: { $0.accountID == aid })
            let accLabel = account.map { accountDisplayName($0) } ?? aid
            return ("\(name) / \(accLabel)", provider?.authStatus == .valid ? .green : .gray)
        }
    }

    // MARK: - Routing Config

    private var configURL: URL {
        FileManager.default.homeDirectoryForCurrentUser
            .appendingPathComponent(".config/byokey/settings.json")
    }

    private func loadRouting() {
        guard let data = try? Data(contentsOf: configURL),
              let json = try? JSONSerialization.jsonObject(with: data) as? [String: Any],
              let providers = json["providers"] as? [String: Any]
        else { return }

        for family in modelFamilies {
            if let conf = providers[family.id] as? [String: Any],
               let backend = conf["backend"] as? String
            {
                routing[family.id] = backend
            }
        }
    }

    private func apply(choice: RoutingChoice, family: String) async {
        switch choice {
        case .ampOfficial:
            writeBackend(family: family, provider: nil, accountId: nil)
            resultMessage = .init(text: "Routing cleared. Requests will pass through to Amp.", isError: false)
        case .provider(let providerId):
            // Pin provider but clear any previously pinned account so the
            // router uses its own active-account selection.
            writeBackend(family: family, provider: providerId, accountId: nil)
            resultMessage = .init(text: "Routing updated. Using provider's active account.", isError: false)
        case .account(let providerId, let accountId):
            // Pin both the provider and the specific account.
            writeBackend(family: family, provider: providerId, accountId: accountId)
            do {
                try await dataService.activateAccount(provider: providerId, accountId: accountId)
                resultMessage = .init(text: "Routing updated. Account pinned.", isError: false)
            } catch {
                resultMessage = .init(
                    text: "Routing saved but activation failed: \(error.localizedDescription)",
                    isError: true
                )
            }
        }
    }

    private func writeBackend(family: String, provider: String?, accountId: String?) {
        routing[family] = provider

        var raw: [String: Any] = [:]
        if let data = try? Data(contentsOf: configURL),
           let json = try? JSONSerialization.jsonObject(with: data) as? [String: Any]
        {
            raw = json
        }

        var providers = raw["providers"] as? [String: Any] ?? [:]
        var conf = providers[family] as? [String: Any] ?? [:]

        if let provider {
            conf["backend"] = provider
        } else {
            conf.removeValue(forKey: "backend")
            conf.removeValue(forKey: "account_id")
        }

        if let accountId {
            conf["account_id"] = accountId
        } else {
            conf.removeValue(forKey: "account_id")
        }

        providers[family] = conf
        raw["providers"] = providers

        let dir = configURL.deletingLastPathComponent()
        try? FileManager.default.createDirectory(at: dir, withIntermediateDirectories: true)
        if let data = try? JSONSerialization.data(
            withJSONObject: raw,
            options: [.prettyPrinted, .sortedKeys]
        ) {
            try? data.write(to: configURL, options: .atomic)
        }
    }

    // MARK: - Quick Action Cards

    private func quickActionCard(
        title: String,
        subtitle: String,
        statusColor: Color?,
        icon: String,
        actionLabel: String,
        isLoading: Bool,
        action: @escaping () -> Void
    ) -> some View {
        VStack(alignment: .leading, spacing: 10) {
            HStack(spacing: 8) {
                Image(systemName: icon)
                    .font(.system(size: 14))
                    .foregroundStyle(Color.accentColor)
                Text(title)
                    .font(.system(size: 13, weight: .semibold))
            }

            HStack(spacing: 5) {
                if let statusColor {
                    Circle().fill(statusColor).frame(width: 6, height: 6)
                }
                Text(subtitle)
                    .font(.caption)
                    .foregroundStyle(.secondary)
                    .lineLimit(2)
            }

            HStack {
                Spacer()
                Button(action: action) {
                    HStack(spacing: 4) {
                        if isLoading {
                            ProgressView().controlSize(.mini)
                        }
                        Text(actionLabel)
                            .font(.caption)
                    }
                }
                .buttonStyle(.borderedProminent)
                .controlSize(.small)
                .disabled(isLoading)

                if title == "Ads Control" {
                    Button {
                        Task { await toggleAds(disable: false) }
                    } label: {
                        Text("Restore")
                            .font(.caption)
                    }
                    .controlSize(.small)
                    .disabled(isTogglingAds)
                }
            }
        }
        .frame(maxWidth: .infinity, alignment: .leading)
        .padding(14)
        .cardSurface(cornerRadius: 12)
    }

    private func resultBanner(_ result: ResultMessage) -> some View {
        HStack(spacing: 8) {
            Image(systemName: result.isError ? "exclamationmark.triangle.fill" : "checkmark.circle.fill")
                .foregroundStyle(result.isError ? .red : .green)
            Text(result.text)
                .font(.caption)
                .textSelection(.enabled)
                .lineLimit(3)
            Spacer()
            Button {
                resultMessage = nil
            } label: {
                Image(systemName: "xmark")
                    .font(.caption2)
                    .foregroundStyle(.secondary)
            }
            .buttonStyle(.plain)
        }
        .padding(12)
        .background(
            (result.isError ? Color.red : Color.green).opacity(0.06),
            in: .rect(cornerRadius: 10)
        )
    }

    // MARK: - Injection Status

    private var injectionSubtitle: String {
        switch injectionStatus {
        case .unknown: proxyURL
        case .injected: "Already injected ✓"
        case .differentURL(let url): "Different: \(url)"
        case .notConfigured: "Not yet configured"
        case .noFile: "Settings file not found"
        }
    }

    private var injectionStatusColor: Color {
        switch injectionStatus {
        case .unknown: .gray
        case .injected: .green
        case .differentURL: .orange
        case .notConfigured, .noFile: .gray
        }
    }

    private func checkInjectionStatus() {
        let settingsURL = FileManager.default.homeDirectoryForCurrentUser
            .appendingPathComponent(".config/amp/settings.json")

        guard FileManager.default.fileExists(atPath: settingsURL.path),
              let data = try? Data(contentsOf: settingsURL),
              let json = try? JSONSerialization.jsonObject(with: data) as? [String: Any]
        else {
            injectionStatus = .noFile
            return
        }

        guard let ampURL = json["amp.url"] as? String else {
            injectionStatus = .notConfigured
            return
        }

        injectionStatus = ampURL == proxyURL ? .injected : .differentURL(ampURL)
    }

    // MARK: - Actions

    private func inject() async {
        isInjecting = true
        defer { isInjecting = false }
        do {
            let output = try await CLIRunner.ampInject()
            resultMessage = .init(
                text: output.trimmingCharacters(in: .whitespacesAndNewlines), isError: false)
            checkInjectionStatus()
        } catch {
            resultMessage = .init(text: error.localizedDescription, isError: true)
        }
    }

    private func toggleAds(disable: Bool) async {
        isTogglingAds = true
        defer { isTogglingAds = false }
        do {
            let output =
                disable
                ? try await CLIRunner.ampAdsDisable()
                : try await CLIRunner.ampAdsEnable()
            resultMessage = .init(
                text: output.trimmingCharacters(in: .whitespacesAndNewlines), isError: false)
        } catch {
            resultMessage = .init(text: error.localizedDescription, isError: true)
        }
    }
}

// MARK: - Supporting Types

private enum InjectionStatus {
    case unknown, injected, differentURL(String), notConfigured, noFile
}

private struct ResultMessage {
    let text: String
    let isError: Bool
}

#Preview {
    AmpView()
        .environment(AppEnvironment.shared)
        .environment(ProcessManager())
        .environment(DataService())
}
