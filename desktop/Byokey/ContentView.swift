import SwiftUI
import ServiceManagement

struct ContentView: View {
    @State private var daemon = DaemonManager()

    var body: some View {
        VStack(spacing: 16) {
            HStack {
                Circle()
                    .fill(statusColor)
                    .frame(width: 10, height: 10)
                Text("Daemon: \(statusText)")
            }
            .font(.headline)

            HStack(spacing: 12) {
                Button("Enable") {
                    do {
                        try daemon.register()
                    } catch {
                        print("Failed to register: \(error)")
                    }
                }
                .disabled(daemon.status == .enabled)

                Button("Disable") {
                    Task {
                        do {
                            try await daemon.unregister()
                        } catch {
                            print("Failed to unregister: \(error)")
                        }
                    }
                }
                .disabled(daemon.status != .enabled)
            }
        }
        .padding(40)
        .onAppear {
            daemon.refresh()
        }
    }

    private var statusColor: Color {
        switch daemon.status {
        case .enabled: .green
        case .notRegistered: .gray
        case .notFound: .red
        case .requiresApproval: .orange
        @unknown default: .gray
        }
    }

    private var statusText: String {
        switch daemon.status {
        case .enabled: "Running"
        case .notRegistered: "Not Registered"
        case .notFound: "Not Found"
        case .requiresApproval: "Requires Approval"
        @unknown default: "Unknown"
        }
    }
}

#Preview {
    ContentView()
}
