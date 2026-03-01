import ServiceManagement

@Observable
final class DaemonManager {
    private(set) var status: SMAppService.Status = .notRegistered

    private var service: SMAppService {
        SMAppService.daemon(plistName: "io.byokey.desktop.daemon.plist")
    }

    func refresh() {
        status = service.status
    }

    func register() throws {
        try service.register()
        refresh()
    }

    func unregister() async throws {
        try await service.unregister()
        refresh()
    }
}
