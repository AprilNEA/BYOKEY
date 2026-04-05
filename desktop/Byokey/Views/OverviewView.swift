import SwiftUI

struct OverviewView: View {
    @Environment(ProcessManager.self) private var pm
    @Environment(AppEnvironment.self) private var appEnv
    @Environment(DataService.self) private var dataService

    var body: some View {
        DetailPage("Overview") {
            // SERVER CONTROL
            sectionTitle("SERVER CONTROL")

            HStack(alignment: .top, spacing: 12) {
                // Server toggle card
                overviewCard {
                    HStack {
                        Text("Proxy Server")
                            .fontWeight(.semibold)
                        Spacer()
                        Toggle("", isOn: Binding(
                            get: { pm.isRunning },
                            set: { $0 ? pm.start() : pm.stop() }
                        ))
                        .toggleStyle(.switch)
                        .labelsHidden()
                        .controlSize(.small)
                    }

                    Text("Expose an OpenAI-compatible API endpoint for AI tools.")
                        .font(.caption)
                        .foregroundStyle(.secondary)

                    HStack(spacing: 5) {
                        Circle()
                            .fill(pm.isReachable ? .green : (pm.isRunning ? .orange : .red))
                            .frame(width: 8, height: 8)
                        Text(pm.isReachable ? "Server is running" : (pm.isRunning ? "Starting…" : "Server is stopped"))
                            .font(.caption)
                            .foregroundStyle(.secondary)
                    }
                }

                // Endpoint card
                if pm.isReachable {
                    overviewCard {
                        Text("Endpoint")
                            .fontWeight(.semibold)

                        Text("Clients connect to this URL to access proxied AI APIs.")
                            .font(.caption)
                            .foregroundStyle(.secondary)

                        HStack(spacing: 5) {
                            Circle()
                                .fill(.green)
                                .frame(width: 8, height: 8)
                            Text(appEnv.baseURL.absoluteString)
                                .font(.caption)
                                .fontDesign(.monospaced)
                                .foregroundStyle(.secondary)
                                .textSelection(.enabled)
                        }
                    }
                }
            }

            Spacer(minLength: 0)
        }
    }

    // MARK: - Helpers

    private func sectionTitle(_ text: String) -> some View {
        Text(text)
            .font(.system(size: 10, weight: .bold))
            .foregroundStyle(Color.accentColor.opacity(0.8))
            .kerning(0.8)
            .padding(.top, 16)
            .padding(.bottom, 8)
    }

    private func overviewCard<C: View>(@ViewBuilder content: () -> C) -> some View {
        VStack(alignment: .leading, spacing: 8) {
            content()
        }
        .frame(maxWidth: .infinity, alignment: .leading)
        .padding(16)
        .background(.white.opacity(0.85), in: .rect(cornerRadius: 14))
        .shadow(color: .black.opacity(0.04), radius: 8, x: 0, y: 2)
    }

}

#Preview {
    OverviewView()
        .environment(AppEnvironment.shared)
        .environment(ProcessManager())
        .environment(DataService())
        .frame(width: 700, height: 600)
}
