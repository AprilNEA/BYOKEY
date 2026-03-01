import SwiftUI

enum SidebarItem: String, CaseIterable, Identifiable {
    case general = "General"
    case accounts = "Accounts"
    case settings = "Settings"

    var id: Self { self }

    var icon: String {
        switch self {
        case .general: "house"
        case .accounts: "person.2"
        case .settings: "gearshape"
        }
    }
}

struct ContentView: View {
    @State private var selection: SidebarItem? = .general

    var body: some View {
        NavigationSplitView {
            List(SidebarItem.allCases, selection: $selection) { item in
                Label(item.rawValue, systemImage: item.icon)
            }
            .navigationSplitViewColumnWidth(min: 160, ideal: 180, max: 220)
            .listStyle(.sidebar)
        } detail: {
            switch selection {
            case .general:  GeneralView()
            case .accounts: AccountsView()
            case .settings: SettingsView()
            case nil:       Text("Select a page")
            }
        }
    }
}

#Preview {
    ContentView()
}
