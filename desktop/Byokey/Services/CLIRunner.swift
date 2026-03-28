import Foundation

enum CLIRunner {
    static var binaryURL: URL { ProcessManager.binaryURL }

    /// Run a CLI command and return (exitCode, output).
    @discardableResult
    private static func run(_ arguments: [String]) async throws -> (Int32, String) {
        let process = Process()
        process.executableURL = binaryURL
        process.arguments = arguments

        let pipe = Pipe()
        process.standardOutput = pipe
        process.standardError = pipe
        process.standardInput = FileHandle.nullDevice

        return try await withCheckedThrowingContinuation { continuation in
            var outputData = Data()
            let lock = NSLock()

            pipe.fileHandleForReading.readabilityHandler = { handle in
                let chunk = handle.availableData
                guard !chunk.isEmpty else { return }
                lock.lock()
                outputData.append(chunk)
                lock.unlock()
            }

            process.terminationHandler = { proc in
                pipe.fileHandleForReading.readabilityHandler = nil
                let remaining = pipe.fileHandleForReading.readDataToEndOfFile()
                lock.lock()
                outputData.append(remaining)
                let finalOutput = String(data: outputData, encoding: .utf8) ?? ""
                lock.unlock()
                continuation.resume(returning: (proc.terminationStatus, finalOutput))
            }

            do {
                try process.run()
            } catch {
                pipe.fileHandleForReading.readabilityHandler = nil
                continuation.resume(throwing: error)
            }
        }
    }

    // MARK: - Login

    static func login(provider: String, account: String? = nil) async throws {
        var args = ["login", provider]
        if let account { args += ["--account", account] }
        let (status, output) = try await run(args)
        if status != 0 { throw CLIError.commandFailed("login", output) }
    }

    // MARK: - Amp

    static func ampInject(url: String? = nil) async throws -> String {
        var args = ["amp", "inject"]
        if let url { args += ["--url", url] }
        let (status, output) = try await run(args)
        if status != 0 { throw CLIError.commandFailed("amp inject", output) }
        return output
    }

    static func ampAdsDisable(all: Bool = true) async throws -> String {
        var args = ["amp", "ads", "disable"]
        if all { args.append("--all") }
        let (status, output) = try await run(args)
        if status != 0 { throw CLIError.commandFailed("ads disable", output) }
        return output
    }

    static func ampAdsEnable() async throws -> String {
        let (status, output) = try await run(["amp", "ads", "enable"])
        if status != 0 { throw CLIError.commandFailed("ads enable", output) }
        return output
    }

    // MARK: - Error

    enum CLIError: LocalizedError {
        case commandFailed(String, String)

        var errorDescription: String? {
            switch self {
            case .commandFailed(let cmd, let output):
                "\(cmd) failed: \(output)"
            }
        }
    }
}
