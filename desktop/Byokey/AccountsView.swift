import SwiftUI

struct AccountsView: View {
    var body: some View {
        ContentUnavailableView(
            "No Accounts",
            systemImage: "person.2",
            description: Text("Account management coming soon.")
        )
        .navigationTitle("Accounts")
    }
}

#Preview {
    AccountsView()
}
