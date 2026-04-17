import SwiftUI

struct ContentView: View {
    @Environment(ProcessManager.self) private var pm
    @State private var selection: SidebarItem? = .activity
    @State private var showLog = false

    var body: some View {
        @Bindable var pm = pm

        AppShell(selection: $selection, showLog: $showLog) {
            detailContent
        }
        .frame(minWidth: 914, minHeight: 672)
        .alert("Server Error", isPresented: $pm.showError) {
            Button("Reload") { pm.restart() }
            Button("OK", role: .cancel) {}
        } message: {
            Text(pm.errorMessage ?? "Unknown error")
        }
    }

    @ViewBuilder
    private var detailContent: some View {
        switch selection {
        case .activity:  GeneralView()
        case .overview:  OverviewView()
        case .accounts:  AccountsView()
        case .models:    ModelsView()
        case .amp:       AmpView()
        case .threads:   ThreadsView()
        case .usage:     UsageView()
        case .settings:  SettingsView()
        case nil:        Text("Select a page")
        }
    }
}

#Preview {
    ContentView()
        .environment(AppEnvironment.shared)
        .environment(ProcessManager())
        .environment(DataService())
        .environment(UpdaterState())
}
