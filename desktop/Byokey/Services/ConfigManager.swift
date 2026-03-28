import AppKit
import Foundation

/// Reads and writes `~/.config/byokey/settings.json`.
///
/// Only touches the fields exposed in the UI; unknown keys are preserved
/// across load/save cycles so hand-edited config is never destroyed.
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

    // MARK: - State

    private(set) var configFileExists = false
    private(set) var needsRestart = false
    private var rawConfig: [String: Any] = [:]
    private var isLoading = false
    private var saveTask: Task<Void, Never>?

    var configURL: URL {
        FileManager.default.homeDirectoryForCurrentUser
            .appendingPathComponent(".config/byokey/settings.json")
    }

    // MARK: - Load

    func load() {
        isLoading = true
        defer { isLoading = false }

        let url = configURL
        configFileExists = FileManager.default.fileExists(atPath: url.path)

        guard configFileExists,
              let data = try? Data(contentsOf: url),
              let json = try? JSONSerialization.jsonObject(with: data) as? [String: Any]
        else { return }

        rawConfig = json
        port = (json["port"] as? NSNumber)?.intValue ?? AppEnvironment.defaultPort
        host = json["host"] as? String ?? "127.0.0.1"
        proxyUrl = json["proxy_url"] as? String ?? ""

        if let log = json["log"] as? [String: Any] {
            logLevel = log["level"] as? String ?? "info"
        }

        if let streaming = json["streaming"] as? [String: Any] {
            keepaliveSeconds = (streaming["keepalive_seconds"] as? NSNumber)?.intValue ?? 15
            bootstrapRetries = (streaming["bootstrap_retries"] as? NSNumber)?.intValue ?? 1
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
        rawConfig["port"] = port
        rawConfig["host"] = host

        if proxyUrl.isEmpty {
            rawConfig.removeValue(forKey: "proxy_url")
        } else {
            rawConfig["proxy_url"] = proxyUrl
        }

        var log = rawConfig["log"] as? [String: Any] ?? [:]
        log["level"] = logLevel
        rawConfig["log"] = log

        var streaming = rawConfig["streaming"] as? [String: Any] ?? [:]
        streaming["keepalive_seconds"] = keepaliveSeconds
        streaming["bootstrap_retries"] = bootstrapRetries
        rawConfig["streaming"] = streaming

        // Create directory if needed.
        let dir = configURL.deletingLastPathComponent()
        try? FileManager.default.createDirectory(at: dir, withIntermediateDirectories: true)

        guard let data = try? JSONSerialization.data(
            withJSONObject: rawConfig,
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
