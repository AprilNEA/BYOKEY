import SwiftUI

struct DashboardStatusBar: View {
    @Environment(ProcessManager.self) private var pm
    @Environment(AppEnvironment.self) private var appEnv
    @Environment(DataService.self) private var dataService
    @State private var endpointCopied = false

    var body: some View {
        HStack(spacing: 0) {
            statusItem("SERVER") {
                HStack(spacing: 8) {
                    Toggle("", isOn: Binding(
                        get: { pm.isRunning },
                        set: { $0 ? pm.start() : pm.stop() }
                    ))
                    .toggleStyle(.switch)
                    .labelsHidden()
                    .controlSize(.small)

                    HStack(spacing: 5) {
                        Circle()
                            .fill(pm.isReachable ? .green : (pm.isRunning ? .orange : .red))
                            .frame(width: 7, height: 7)
                        Text(
                            pm.isReachable ? "Running" : (pm.isRunning ? "Starting…" : "Stopped")
                        )
                        .fontWeight(.semibold)
                    }
                }
            }

            Spacer()

            if pm.isReachable {
                statusItem("ENDPOINT") {
                    HStack(spacing: 4) {
                        Text(appEnv.baseURL.absoluteString)
                            .fontDesign(.monospaced)

                        Button {
                            NSPasteboard.general.clearContents()
                            NSPasteboard.general.setString(
                                appEnv.baseURL.absoluteString, forType: .string)
                            endpointCopied = true
                            Task {
                                try? await Task.sleep(for: .seconds(1.5))
                                endpointCopied = false
                            }
                        } label: {
                            Image(systemName: endpointCopied ? "checkmark" : "doc.on.doc")
                                .foregroundStyle(endpointCopied ? .green : .secondary)
                                .contentTransition(.symbolEffect(.replace))
                        }
                        .buttonStyle(.borderless)
                    }
                }

                Spacer()

                statusItem("PROVIDERS") {
                    let active = dataService.providers.filter { $0.enabled && $0.authStatus == .valid }.count
                    let total = dataService.providers.count
                    Text("\(active)/\(total) active")
                        .fontWeight(.semibold)
                }
            }
        }
        .padding(16)
        .background(.white.opacity(0.85), in: .rect(cornerRadius: 14))
        .shadow(color: .black.opacity(0.04), radius: 8, x: 0, y: 2)
    }

    private func statusItem<C: View>(
        _ label: String, @ViewBuilder content: () -> C
    ) -> some View {
        VStack(alignment: .leading, spacing: 4) {
            Text(label)
                .font(.system(size: 10, weight: .bold))
                .foregroundStyle(Color.accentColor.opacity(0.7))
                .kerning(0.8)
            content()
                .font(.subheadline)
        }
    }
}
