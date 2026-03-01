import Foundation
import ServiceManagement

@Observable
final class DaemonManager {
    private(set) var registrationStatus: SMAppService.Status = .notRegistered
    private(set) var isReachable = false
    private(set) var isTransitioning = false
    private(set) var errorMessage: String?

    private var monitorTask: Task<Void, Never>?

    private var service: SMAppService {
        SMAppService.agent(plistName: "io.byokey.desktop.daemon.plist")
    }

    var statusSummary: StatusSummary {
        if isTransitioning { return .transitioning }
        if isReachable { return .running }
        if registrationStatus == .enabled { return .registered }
        return .stopped
    }

    enum StatusSummary {
        case stopped
        case transitioning
        case registered // registered but not yet reachable
        case running
    }

    func refresh() {
        registrationStatus = service.status
    }

    /// Start periodic reachability monitoring.
    func startMonitoring() {
        monitorTask?.cancel()
        monitorTask = Task { [weak self] in
            while !Task.isCancelled {
                await self?.checkReachability()
                try? await Task.sleep(for: .seconds(3))
            }
        }
    }

    func stopMonitoring() {
        monitorTask?.cancel()
    }

    func enable() async {
        errorMessage = nil
        isTransitioning = true

        do {
            try service.register()
        } catch {
            errorMessage = error.localizedDescription
            isTransitioning = false
            refresh()
            return
        }

        refresh()

        // Wait up to 10s for daemon to become reachable.
        for _ in 0..<20 {
            try? await Task.sleep(for: .milliseconds(500))
            await checkReachability()
            if isReachable { break }
        }

        if !isReachable {
            errorMessage = "Daemon registered but not responding. Check Console.app for launch errors."
        }
        isTransitioning = false
    }

    func disable() async {
        errorMessage = nil
        isTransitioning = true

        do {
            try await service.unregister()
        } catch {
            errorMessage = error.localizedDescription
        }

        refresh()

        // Wait up to 3s for daemon to stop.
        for _ in 0..<6 {
            try? await Task.sleep(for: .milliseconds(500))
            await checkReachability()
            if !isReachable { break }
        }

        isTransitioning = false
    }

    @discardableResult
    func checkReachability() async -> Bool {
        let url = URL(string: "http://127.0.0.1:8018/v0/management/status")!
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
}
