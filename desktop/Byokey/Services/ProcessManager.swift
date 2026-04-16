import Foundation
import SwiftUI

@Observable
final class ProcessManager {
    private(set) var isRunning = false
    private(set) var isReachable = false
    private(set) var logs: [String] = []
    private(set) var errorMessage: String?
    var showError = false

    private var process: Process?
    private var healthTask: Task<Void, Never>?
    private var shouldAutoRestart = true
    private(set) var currentPort = AppEnvironment.defaultPort

    /// Bundled binary in release; cargo target directory in development.
    static var binaryURL: URL {
        let bundled = Bundle.main.bundleURL
            .appendingPathComponent("Contents/Resources/byokey")
        if FileManager.default.fileExists(atPath: bundled.path) {
            return bundled
        }

        #if DEBUG
        if let dir = Bundle.main.infoDictionary?["RustWorkspaceDir"] as? String {
            for candidate in [
                "target/aarch64-apple-darwin/debug/byokey",
                "target/debug/byokey",
            ] {
                let url = URL(filePath: dir).appendingPathComponent(candidate)
                if FileManager.default.fileExists(atPath: url.path) {
                    return url
                }
            }
        }
        #endif

        return URL(filePath: "/usr/local/bin/byokey")
    }

    /// Persistent log directory inside `~/Library/Logs/Byokey/`.
    static var logDirectory: URL {
        FileManager.default.urls(for: .libraryDirectory, in: .userDomainMask)[0]
            .appendingPathComponent("Logs/Byokey")
    }

    // MARK: - Lifecycle

    func start(port: Int = AppEnvironment.defaultPort) {
        guard process == nil else { return }
        currentPort = port
        shouldAutoRestart = true
        errorMessage = nil

        // Ensure log directory exists.
        let logDir = Self.logDirectory
        try? FileManager.default.createDirectory(at: logDir, withIntermediateDirectories: true)
        let logFile = logDir.appendingPathComponent("server.log")

        let proc = Process()
        proc.executableURL = Self.binaryURL
        proc.arguments = ["serve", "--port", "\(port)", "--log-file", logFile.path]

        let pipe = Pipe()
        proc.standardOutput = pipe
        proc.standardError = pipe
        proc.standardInput = FileHandle.nullDevice

        pipe.fileHandleForReading.readabilityHandler = { [weak self] handle in
            let data = handle.availableData
            guard !data.isEmpty,
                  let str = String(data: data, encoding: .utf8)
            else { return }

            let newLines = str
                .components(separatedBy: .newlines)
                .filter { !$0.isEmpty }

            guard !newLines.isEmpty else { return }
            DispatchQueue.main.async { [weak self] in
                guard let self else { return }
                self.logs.append(contentsOf: newLines)
                if self.logs.count > 1000 {
                    self.logs = Array(self.logs.suffix(500))
                }
            }
        }

        proc.terminationHandler = { [weak self] p in
            DispatchQueue.main.async { [weak self] in
                guard let self else { return }
                self.isRunning = false
                self.isReachable = false
                self.process = nil

                if p.terminationStatus != 0 {
                    let tail = self.logs.suffix(5).map { AnsiParser.strip($0) }.joined(separator: "\n")
                    self.errorMessage = tail.isEmpty ? "Process exited with code \(p.terminationStatus)" : tail
                    self.showError = true
                }

                if self.shouldAutoRestart, p.terminationReason == .uncaughtSignal {
                    self.logs.append("[byokey] process crashed, restarting in 2s…")
                    DispatchQueue.main.asyncAfter(deadline: .now() + 2) { [weak self] in
                        self?.start(port: port)
                    }
                }
            }
        }

        do {
            try proc.run()
            process = proc
            isRunning = true
            startHealthMonitoring()
        } catch {
            errorMessage = "Failed to start: \(error.localizedDescription)"
            showError = true
        }
    }

    func stop() {
        shouldAutoRestart = false
        stopHealthMonitoring()
        guard let proc = process, proc.isRunning else {
            isRunning = false
            isReachable = false
            return
        }
        proc.terminate()
        process = nil
        isRunning = false
        isReachable = false
    }

    func restart(port: Int? = nil) {
        let p = port ?? currentPort
        stop()
        DispatchQueue.main.asyncAfter(deadline: .now() + 0.5) { [weak self] in
            self?.start(port: p)
        }
    }

    // MARK: - Health

    private func startHealthMonitoring() {
        healthTask?.cancel()
        healthTask = Task { [weak self] in
            while !Task.isCancelled {
                await self?.checkReachability()
                try? await Task.sleep(for: .seconds(3))
            }
        }
    }

    private func stopHealthMonitoring() {
        healthTask?.cancel()
    }

    @discardableResult
    func checkReachability() async -> Bool {
        // Probes /v1/models — a cheap REST endpoint that's always available
        // on the main proxy port. The old /v0/management/status path moved
        // to ConnectRPC; see SWIFT_MIGRATION.md.
        let url = URL(string: "http://127.0.0.1:\(currentPort)/v1/models")!
        var request = URLRequest(url: url, timeoutInterval: 2)
        request.httpMethod = "GET"
        do {
            let (_, response) = try await URLSession.shared.data(for: request)
            let reachable = (response as? HTTPURLResponse)?.statusCode == 200
            isReachable = reachable
            return reachable
        } catch {
            isReachable = false
            return false
        }
    }

    // MARK: - Logs

    func clearLogs() {
        logs.removeAll()
    }

    deinit {
        shouldAutoRestart = false
        process?.terminate()
        healthTask?.cancel()
    }
}
