import SwiftUI
import OpenAPIURLSession

struct GeneralView: View {
    @State private var daemon = DaemonManager()
    @State private var providers: [Components.Schemas.ProviderStatus] = []
    @State private var pollTask: Task<Void, Never>?

    var body: some View {
        Form {
            Section("Daemon") {
                    LabeledContent("Status") {
                        HStack(spacing: 6) {
                            switch daemon.statusSummary {
                            case .transitioning:
                                ProgressView()
                                    .controlSize(.small)
                                Text("Starting…")
                                    .foregroundStyle(.secondary)
                            case .running:
                                Circle().fill(.green).frame(width: 8, height: 8)
                                Text("Running")
                            case .registered:
                                Circle().fill(.orange).frame(width: 8, height: 8)
                                Text("Registered")
                                    .foregroundStyle(.secondary)
                            case .stopped:
                                Circle().fill(.red).frame(width: 8, height: 8)
                                Text("Stopped")
                                    .foregroundStyle(.secondary)
                            }
                        }
                    }

                    Toggle("Enabled", isOn: Binding(
                        get: { daemon.registrationStatus == .enabled },
                        set: { newValue in
                            Task {
                                if newValue {
                                    await daemon.enable()
                                } else {
                                    await daemon.disable()
                                }
                            }
                        }
                    ))
                    .disabled(daemon.isTransitioning)

                    if let error = daemon.errorMessage {
                        Label(error, systemImage: "exclamationmark.triangle.fill")
                            .foregroundStyle(.red)
                            .font(.caption)
                    }
                }

                Section("Providers") {
                    if daemon.isReachable {
                        if providers.isEmpty {
                            Text("No providers configured")
                                .foregroundStyle(.secondary)
                        } else {
                            ForEach(providers, id: \.id) { provider in
                                ProviderRow(provider: provider)
                            }
                        }
                    } else if daemon.isTransitioning {
                        HStack(spacing: 8) {
                            ProgressView().controlSize(.small)
                            Text("Connecting to daemon…")
                                .foregroundStyle(.secondary)
                        }
                        .frame(maxWidth: .infinity, alignment: .center)
                        .padding(.vertical, 8)
                    } else {
                        ContentUnavailableView(
                            "Daemon Not Running",
                            systemImage: "server.rack",
                            description: Text("Enable the daemon to view provider status.")
                        )
                    }
                }
            DaemonLogView()
        }
        .formStyle(.grouped)
        .navigationTitle("General")
        .onAppear {
            daemon.refresh()
            daemon.startMonitoring()
            startPolling()
        }
        .onDisappear {
            daemon.stopMonitoring()
            pollTask?.cancel()
        }
    }

    private func startPolling() {
        pollTask?.cancel()
        pollTask = Task {
            let client = Client(
                serverURL: URL(string: "http://127.0.0.1:8018")!,
                transport: URLSessionTransport()
            )
            while !Task.isCancelled {
                if daemon.isReachable {
                    do {
                        let response = try await client.status_handler()
                        let status = try response.ok.body.json
                        providers = status.providers
                    } catch {
                        providers = []
                    }
                } else {
                    providers = []
                }
                try? await Task.sleep(for: .seconds(3))
            }
        }
    }
}

private struct ProviderRow: View {
    let provider: Components.Schemas.ProviderStatus

    var body: some View {
        HStack {
            VStack(alignment: .leading, spacing: 2) {
                Text(provider.display_name)
                Text("\(provider.models_count) models")
                    .font(.caption)
                    .foregroundStyle(.tertiary)
            }

            Spacer()

            HStack(spacing: 6) {
                Text(authLabel)
                    .font(.caption)
                    .foregroundStyle(authColor)
                Circle()
                    .fill(authColor)
                    .frame(width: 8, height: 8)
            }
        }
        .opacity(provider.enabled ? 1 : 0.5)
    }

    private var authColor: Color {
        switch provider.auth_status {
        case .valid: .green
        case .expired: .orange
        case .not_configured: .gray
        }
    }

    private var authLabel: String {
        switch provider.auth_status {
        case .valid: "Active"
        case .expired: "Expired"
        case .not_configured: "Not Configured"
        }
    }
}

#Preview {
    GeneralView()
}
