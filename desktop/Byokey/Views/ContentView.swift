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
        } detail: {
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
        .frame(minWidth: 480, minHeight: 320)
        .alert("Server Error", isPresented: $pm.showError) {
            Button("Reload") { pm.restart() }
            Button("OK", role: .cancel) {}
        } message: {
            Text(pm.errorMessage ?? "Unknown error")
        }
    }
}

#Preview {
    ContentView()
        .environment(AppEnvironment.shared)
        .environment(ProcessManager())
        .environment(DataService())
}
