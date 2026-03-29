import SwiftUI

enum SidebarItem: String, Identifiable {
    case dashboard = "Dashboard"
    case accounts = "Accounts"
    case models = "Models"
    case amp = "Amp"
    case usage = "Usage"
    case settings = "Settings"

    var id: Self { self }
}

struct ContentView: View {
    @Environment(ProcessManager.self) private var pm
    @State private var selection: SidebarItem? = .dashboard
    @State private var showLog = false

    var body: some View {
        @Bindable var pm = pm

        NavigationSplitView {
            List(selection: $selection) {
                Label("Dashboard", systemImage: "square.grid.2x2")
                    .tag(SidebarItem.dashboard)

                Section("Provider") {
                    Label("Accounts", systemImage: "person.2")
                        .tag(SidebarItem.accounts)
                    Label("Models", systemImage: "cpu")
                        .tag(SidebarItem.models)
                }

                Section("Agent") {
                    Label("Amp", systemImage: "bolt.fill")
                        .tag(SidebarItem.amp)
                }

                Section("Proxy") {
                    Label("Usage", systemImage: "chart.bar")
                        .tag(SidebarItem.usage)
                    Label("Settings", systemImage: "gearshape")
                        .tag(SidebarItem.settings)
                }
            }
            .navigationSplitViewColumnWidth(min: 160, ideal: 180, max: 220)
            .listStyle(.sidebar)
            .safeAreaInset(edge: .bottom, spacing: 0) {
                logToggleButton
            }
        } detail: {
            VStack(spacing: 0) {
                Group {
                    switch selection {
                    case .dashboard: GeneralView()
                    case .accounts:  AccountsView()
                    case .models:    ModelsView()
                    case .amp:       AmpView()
                    case .usage:     UsageView()
                    case .settings:  SettingsView()
                    case nil:        Text("Select a page")
                    }
                }
                .frame(maxHeight: .infinity)

                if showLog {
                    Divider()
                    LogPanel()
                }
            }
        }
        .frame(minWidth: 720, minHeight: 560)
        .alert("Server Error", isPresented: $pm.showError) {
            Button("Reload") { pm.restart() }
            Button("OK", role: .cancel) {}
        } message: {
            Text(pm.errorMessage ?? "Unknown error")
        }
    }

    private var logToggleButton: some View {
        Button {
            withAnimation(.easeInOut(duration: 0.2)) {
                showLog.toggle()
            }
        } label: {
            HStack(spacing: 6) {
                Image(systemName: "terminal")
                Text("Log")
                Spacer()
                if !pm.logs.isEmpty {
                    Text("\(pm.logs.count)")
                        .font(.caption2)
                        .monospacedDigit()
                        .foregroundStyle(.tertiary)
                }
                Image(systemName: showLog ? "chevron.down" : "chevron.up")
                    .font(.caption2)
                    .foregroundStyle(.tertiary)
            }
            .font(.caption)
            .foregroundStyle(.secondary)
            .padding(.horizontal, 16)
            .padding(.vertical, 8)
        }
        .buttonStyle(.plain)
        .background(.bar)
    }
}

// MARK: - Log Panel

private struct LogPanel: View {
    @Environment(ProcessManager.self) private var pm

    var body: some View {
        VStack(spacing: 0) {
            ScrollViewReader { proxy in
                ScrollView([.vertical, .horizontal], showsIndicators: true) {
                    if pm.logs.isEmpty {
                        Text("Waiting for log entries…")
                            .font(.system(size: 11, design: .monospaced))
                            .foregroundStyle(.tertiary)
                            .frame(maxWidth: .infinity, alignment: .leading)
                            .padding(8)
                    } else {
                        VStack(alignment: .leading, spacing: 0) {
                            ForEach(Array(pm.logs.enumerated()), id: \.offset) { i, line in
                                Text(AnsiParser.parse(line))
                                    .font(.system(size: 11, design: .monospaced))
                                    .textSelection(.enabled)
                                    .frame(maxWidth: .infinity, alignment: .leading)
                                    .fixedSize(horizontal: true, vertical: false)
                                    .id(i)
                            }
                        }
                        .padding(8)
                    }
                }
                .frame(height: 140)
                .onChange(of: pm.logs.count) {
                    proxy.scrollTo(pm.logs.count - 1, anchor: .bottom)
                }
            }

            HStack(spacing: 12) {
                Text("\(pm.logs.count) lines")
                    .foregroundStyle(.tertiary)
                    .monospacedDigit()
                Spacer()
                Button("Clear", systemImage: "trash") {
                    pm.clearLogs()
                }
                .buttonStyle(.borderless)
                .labelStyle(.iconOnly)
            }
            .font(.caption2)
            .padding(.horizontal, 12)
            .padding(.vertical, 4)
            .background(.bar)
        }
    }
}

#Preview {
    ContentView()
        .environment(AppEnvironment.shared)
        .environment(ProcessManager())
        .environment(DataService())
}
