import SwiftUI

@main
struct ByokeyApp: App {
    @State private var appEnv = AppEnvironment.shared
    @State private var processManager = ProcessManager()
    @State private var dataService = DataService()

    var body: some Scene {
        Window("BYOKEY", id: "main") {
            ContentView()
                .environment(appEnv)
                .environment(processManager)
                .environment(dataService)
                .onAppear {
                    let config = ConfigManager()
                    config.load()
                    appEnv.port = config.port
                    processManager.start(port: appEnv.port)
                }
                .onChange(of: processManager.isReachable) { _, newValue in
                    dataService.isServerReachable = newValue
                }
        }
        .defaultSize(width: 860, height: 640)

        MenuBarExtra {
            MenuBarMenu()
                .environment(appEnv)
                .environment(processManager)
        } label: {
            Image(systemName: "server.rack")
        }
    }
}

private struct MenuBarMenu: View {
    @Environment(ProcessManager.self) private var pm
    @Environment(\.openWindow) private var openWindow

    var body: some View {
        Button("Show Control Panel") {
            openWindow(id: "main")
            NSApplication.shared.activate(ignoringOtherApps: true)
        }
        .keyboardShortcut(",", modifiers: .command)

        Divider()

        Label(
            pm.isReachable ? "Running" : (pm.isRunning ? "Starting…" : "Stopped"),
            systemImage: pm.isReachable ? "circle.fill" : "circle"
        )

        Button("Reload") {
            pm.restart()
        }
        .keyboardShortcut("r", modifiers: .command)
        .disabled(!pm.isRunning)

        Divider()

        Button("Quit BYOKEY") {
            pm.stop()
            NSApplication.shared.terminate(nil)
        }
        .keyboardShortcut("q", modifiers: .command)
    }
}
