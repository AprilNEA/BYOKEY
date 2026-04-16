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

// MARK: - View

struct AmpView: View {
    @Environment(ProcessManager.self) private var pm
    @Environment(AppEnvironment.self) private var appEnv
    @Environment(DataService.self) private var dataService
    @State private var isInjecting = false
    @State private var isTogglingAds = false
    @State private var resultMessage: ResultMessage?
    @State private var injectionStatus: InjectionStatus = .unknown

    /// Which provider routes each model family. nil = native (default).
    @State private var routing: [String: String] = [:]  // family.id → provider id

    private var proxyURL: String {
        "\(appEnv.baseURL.absoluteString)/amp"
    }

    var body: some View {
        DetailPage("Amp") {
            VStack(spacing: 20) {
                // ── Model Routing ────────────────────────────
                sectionHeader("MODEL ROUTING", subtitle: "Choose which Provider handles each model family")

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
                }

                Spacer(minLength: 0)
            }
        }
        .onAppear {
            checkInjectionStatus()
            loadRouting()
        }
    }

    // MARK: - Section Header

    private func sectionHeader(_ title: String, subtitle: String?) -> some View {
        VStack(alignment: .leading, spacing: 2) {
            Text(title)
                .font(.system(size: 10, weight: .bold))
                .foregroundStyle(Color.accentColor.opacity(0.8))
                .kerning(0.8)
            if let subtitle {
                Text(subtitle)
                    .font(.caption)
                    .foregroundStyle(.tertiary)
            }
        }
        .frame(maxWidth: .infinity, alignment: .leading)
    }

    // MARK: - Model Routing Card

    private func modelRoutingCard(_ family: ModelFamily) -> some View {
        // routing values: nil = Amp Official (passthrough), "claude"/"copilot"/etc = specific provider
        let selectedId = routing[family.id]
        let isAmpOfficial = selectedId == nil
        let selectedProvider = selectedId.flatMap { id in
            dataService.providers.first { $0.id == id }
        }
        let isActive = isAmpOfficial || selectedProvider?.authStatus == .valid

        // Display name for the current selection
        let displayName: String = if isAmpOfficial {
            "Amp Official"
        } else {
            selectedProvider?.displayName ?? selectedId ?? family.name
        }

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

            // Provider picker
            Menu {
                // Amp Official (passthrough — no interception)
                Button {
                    setRouting(family: family.id, provider: nil)
                } label: {
                    HStack {
                        Text("Amp Official")
                        if isAmpOfficial { Image(systemName: "checkmark") }
                    }
                }

                Divider()

                // All providers (including the native one for this family)
                ForEach(dataService.providers.filter { $0.id != "amp" }, id: \.id) { provider in
                    Button {
                        setRouting(family: family.id, provider: provider.id)
                    } label: {
                        HStack {
                            Circle()
                                .fill(provider.authStatus == .valid ? .green : .gray)
                                .frame(width: 6, height: 6)
                            Text(provider.displayName)
                            if provider.id == selectedId {
                                Image(systemName: "checkmark")
                            }
                        }
                    }
                }
            } label: {
                HStack(spacing: 6) {
                    Circle()
                        .fill(isActive ? .green : .gray.opacity(0.3))
                        .frame(width: 6, height: 6)

                    Text(displayName)
                        .font(.system(size: 11))
                        .foregroundStyle(isAmpOfficial ? .secondary : .primary)
                        .lineLimit(1)

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
        .background(.white.opacity(0.85), in: .rect(cornerRadius: 14))
        .shadow(color: .black.opacity(0.04), radius: 8, y: 2)
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

    private func setRouting(family: String, provider: String?) {
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

        resultMessage = .init(
            text: "Routing updated. Restart the server to apply.",
            isError: false
        )
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
        .background(.white.opacity(0.85), in: .rect(cornerRadius: 12))
        .shadow(color: .black.opacity(0.03), radius: 6, y: 2)
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
