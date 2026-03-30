import ServiceManagement
import SwiftUI

struct SettingsView: View {
    @Environment(ProcessManager.self) private var pm
    @Environment(AppEnvironment.self) private var appEnv
    @State private var config = ConfigManager()
    @Environment(UpdaterState.self) private var updaterState
    @State private var launchAtLogin = SMAppService.mainApp.status == .enabled

    var body: some View {
        DetailPage("Settings") {
                Form {
                    if config.needsRestart, pm.isRunning {
                        Section {
                            HStack {
                                Label("Settings changed. Restart to apply.", systemImage: "exclamationmark.triangle.fill")
                                    .foregroundStyle(.orange)
                                    .font(.callout)
                                Spacer()
                                Button("Restart Now") {
                                    config.clearRestartFlag()
                                    pm.restart(port: config.port)
                                }
                                .buttonStyle(.borderedProminent)
                                .controlSize(.small)
                            }
                        }
                    }

                    Section("General") {
                        Toggle("Launch at Login", isOn: $launchAtLogin)
                            .onChange(of: launchAtLogin) { _, newValue in
                                do {
                                    if newValue {
                                        try SMAppService.mainApp.register()
                                    } else {
                                        SMAppService.mainApp.unregister { _ in }
                                    }
                                } catch {
                                    launchAtLogin = SMAppService.mainApp.status == .enabled
                                }
                            }

                        Toggle("Automatically Check for Updates",
                               isOn: Binding(
                                   get: { updaterState.automaticallyChecksForUpdates },
                                   set: { updaterState.automaticallyChecksForUpdates = $0 }
                               ))

                        Button("Check for Updates…") {
                            updaterState.checkForUpdates()
                        }
                        .disabled(!updaterState.canCheckForUpdates)
                    }

                    Section("Server") {
                        TextField("Port", value: $config.port, format: .number.grouping(.never))
                            .monospacedDigit()
                        TextField("Host", text: $config.host)
                            .monospacedDigit()
                    }

                    Section("Network") {
                        TextField("Proxy URL", text: $config.proxyUrl, prompt: Text("socks5://host:port"))
                    }

                    Section("Streaming") {
                        TextField(
                            "SSE Keepalive (seconds)",
                            value: $config.keepaliveSeconds,
                            format: .number.grouping(.never)
                        )
                        .monospacedDigit()

                        TextField(
                            "Bootstrap Retries",
                            value: $config.bootstrapRetries,
                            format: .number.grouping(.never)
                        )
                        .monospacedDigit()
                    }

                    Section("Logging") {
                        Picker("Level", selection: $config.logLevel) {
                            Text("Error").tag("error")
                            Text("Warn").tag("warn")
                            Text("Info").tag("info")
                            Text("Debug").tag("debug")
                            Text("Trace").tag("trace")
                        }
                    }

                    Section {
                        LabeledContent("Path") {
                            Text(config.configURL.path)
                                .font(.caption)
                                .foregroundStyle(.secondary)
                                .textSelection(.enabled)
                                .lineLimit(1)
                                .truncationMode(.middle)
                        }

                        HStack {
                            Button("Reveal in Finder") {
                                config.revealInFinder()
                            }
                            Button("Open in Editor") {
                                config.openInEditor()
                            }
                        }
                    } header: {
                        Text("Config File")
                    }

                    Section {
                        Button("Restart Server") {
                            config.clearRestartFlag()
                            pm.restart(port: config.port)
                        }
                        .disabled(!pm.isRunning)
                    }
                }
                .formStyle(.grouped)
                .scrollContentBackground(.hidden)
            }
        .onAppear {
            config.load()
            appEnv.port = config.port
        }
        .onChange(of: config.port) { _, newPort in
            appEnv.port = newPort
        }
    }
}

#Preview {
    SettingsView()
        .environment(AppEnvironment.shared)
        .environment(ProcessManager())
        .environment(UpdaterState())
}
