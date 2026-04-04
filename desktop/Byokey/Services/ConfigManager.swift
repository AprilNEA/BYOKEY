import AppKit
import Foundation

/// Per-provider override (base_url / api_key).
struct ProviderOverride: Equatable {
    var baseUrl: String = ""
    var apiKey: String = ""

    var isEmpty: Bool { baseUrl.isEmpty && apiKey.isEmpty }
}

/// Known provider IDs matching the Rust `ProviderId` enum.
enum KnownProvider: String, CaseIterable, Identifiable {
    case claude, codex, gemini, copilot, kiro, antigravity, qwen, iflow, kimi
    var id: String { rawValue }

    var displayName: String {
        switch self {
        case .claude: "Claude"
        case .codex: "Codex / OpenAI"
        case .gemini: "Gemini"
        case .copilot: "GitHub Copilot"
        case .kiro: "Kiro"
        case .antigravity: "Antigravity"
        case .qwen: "Qwen"
        case .iflow: "iFlow"
        case .kimi: "Kimi"
        }
    }
}

/// Reads and writes `~/.config/byokey/settings.json`.
///
/// Uses a typed `Codable` struct for known fields. Unknown keys are preserved
/// across load/save cycles via a raw overlay so hand-edited config is never destroyed.
@Observable
final class ConfigManager {
    // MARK: - Server

    var port: Int = AppEnvironment.defaultPort { didSet { scheduleSave() } }
    var host: String = "127.0.0.1" { didSet { scheduleSave() } }

    // MARK: - Network

    var proxyUrl: String = "" { didSet { scheduleSave() } }

    // MARK: - Logging

    var logLevel: String = "info" { didSet { scheduleSave() } }

    // MARK: - Streaming

    var keepaliveSeconds: Int = 15 { didSet { scheduleSave() } }
    var bootstrapRetries: Int = 1 { didSet { scheduleSave() } }

    // MARK: - Provider Overrides

    /// Per-provider custom endpoint and API key overrides.
    var providerOverrides: [String: ProviderOverride] = [:] { didSet { scheduleSave() } }

    // MARK: - State

    private(set) var configFileExists = false
    private(set) var needsRestart = false
    private var rawOverlay: [String: Any] = [:]
    private var isLoading = false
    private var saveTask: Task<Void, Never>?

    var configURL: URL {
        FileManager.default.homeDirectoryForCurrentUser
            .appendingPathComponent(".config/byokey/settings.json")
    }

    // MARK: - Codable Schema

    private struct ConfigFile: Codable {
        var port: Int?
        var host: String?
        var proxy_url: String?
        var log: LogConfig?
        var streaming: StreamingConfig?

        struct LogConfig: Codable {
            var level: String?
        }

        struct StreamingConfig: Codable {
            var keepalive_seconds: Int?
            var bootstrap_retries: Int?
        }
    }

    // MARK: - Load

    func load() {
        isLoading = true
        defer { isLoading = false }

        let url = configURL
        configFileExists = FileManager.default.fileExists(atPath: url.path)
        guard configFileExists, let data = try? Data(contentsOf: url) else { return }

        // Preserve raw overlay for unknown keys
        rawOverlay = (try? JSONSerialization.jsonObject(with: data) as? [String: Any]) ?? [:]

        // Decode typed fields
        guard let config = try? JSONDecoder().decode(ConfigFile.self, from: data) else { return }

        port = config.port ?? AppEnvironment.defaultPort
        host = config.host ?? "127.0.0.1"
        proxyUrl = config.proxy_url ?? ""
        logLevel = config.log?.level ?? "info"
        keepaliveSeconds = config.streaming?.keepalive_seconds ?? 15
        bootstrapRetries = config.streaming?.bootstrap_retries ?? 1

        // Load provider overrides from raw overlay
        if let providers = rawOverlay["providers"] as? [String: Any] {
            var overrides: [String: ProviderOverride] = [:]
            for (key, value) in providers {
                guard let dict = value as? [String: Any] else { continue }
                var override_ = ProviderOverride()
                override_.baseUrl = dict["base_url"] as? String ?? ""
                override_.apiKey = dict["api_key"] as? String ?? ""
                if !override_.isEmpty {
                    overrides[key] = override_
                }
            }
            providerOverrides = overrides
        }
    }

    // MARK: - Save

    private func scheduleSave() {
        guard !isLoading else { return }
        needsRestart = true
        saveTask?.cancel()
        saveTask = Task { @MainActor in
            try? await Task.sleep(for: .milliseconds(500))
            guard !Task.isCancelled else { return }
            self.save()
        }
    }

    func save() {
        // Build typed config
        var config = ConfigFile()
        config.port = port
        config.host = host
        config.proxy_url = proxyUrl.isEmpty ? nil : proxyUrl
        config.log = .init(level: logLevel)
        config.streaming = .init(
            keepalive_seconds: keepaliveSeconds,
            bootstrap_retries: bootstrapRetries
        )

        // Encode typed → merge onto raw overlay (preserving unknown keys)
        if let typedData = try? JSONEncoder().encode(config),
           let typedDict = try? JSONSerialization.jsonObject(with: typedData) as? [String: Any]
        {
            for (key, value) in typedDict {
                rawOverlay[key] = value
            }
            // Remove proxy_url key entirely if empty
            if proxyUrl.isEmpty {
                rawOverlay.removeValue(forKey: "proxy_url")
            }
        }

        // Merge provider overrides into raw overlay (preserving other provider keys)
        var providers = rawOverlay["providers"] as? [String: Any] ?? [:]
        for (providerId, override_) in providerOverrides {
            var dict = providers[providerId] as? [String: Any] ?? [:]
            if override_.baseUrl.isEmpty {
                dict.removeValue(forKey: "base_url")
            } else {
                dict["base_url"] = override_.baseUrl
            }
            if override_.apiKey.isEmpty {
                dict.removeValue(forKey: "api_key")
            } else {
                dict["api_key"] = override_.apiKey
            }
            if dict.isEmpty {
                providers.removeValue(forKey: providerId)
            } else {
                providers[providerId] = dict
            }
        }
        if providers.isEmpty {
            rawOverlay.removeValue(forKey: "providers")
        } else {
            rawOverlay["providers"] = providers
        }

        let dir = configURL.deletingLastPathComponent()
        try? FileManager.default.createDirectory(at: dir, withIntermediateDirectories: true)

        guard let data = try? JSONSerialization.data(
            withJSONObject: rawOverlay,
            options: [.prettyPrinted, .sortedKeys]
        ) else { return }
        try? data.write(to: configURL, options: .atomic)
        configFileExists = true
    }

    func revealInFinder() {
        NSWorkspace.shared.selectFile(configURL.path, inFileViewerRootedAtPath: "")
    }

    func openInEditor() {
        NSWorkspace.shared.open(configURL)
    }

    func clearRestartFlag() {
        needsRestart = false
    }
}
