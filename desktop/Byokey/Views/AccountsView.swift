import SwiftUI

struct AccountsView: View {
    @Environment(ProcessManager.self) private var pm
    @Environment(DataService.self) private var dataService
    @State private var loginInProgress: String?
    @State private var errorMessage: String?

    var body: some View {
        DetailPage("Accounts") {
            if pm.isReachable {
                Form {
                    if dataService.providerAccounts.isEmpty, dataService.isLoading {
                        Section {
                            HStack(spacing: 8) {
                                ProgressView().controlSize(.small)
                                Text("Loading accounts…")
                                    .foregroundStyle(.secondary)
                            }
                            .frame(maxWidth: .infinity, alignment: .center)
                            .padding(.vertical, 8)
                        }
                    } else {
                        ForEach(dataService.providerAccounts, id: \.id) { provider in
                            Section {
                                if provider.accounts.isEmpty {
                                    Text("No accounts configured")
                                        .foregroundStyle(.secondary)
                                } else {
                                    ForEach(provider.accounts, id: \.account_id) { account in
                                        AccountRow(
                                            account: account,
                                            providerName: provider.display_name,
                                            onActivate: {
                                                Task { await activateAccount(provider: provider.id, accountId: account.account_id) }
                                            },
                                            onRemove: {
                                                Task { await removeAccount(provider: provider.id, accountId: account.account_id) }
                                            }
                                        )
                                    }
                                }
                            } header: {
                                HStack(spacing: 6) {
                                    if let iconName = providerIconName(for: provider.id) {
                                        Image(iconName)
                                            .resizable()
                                            .scaledToFit()
                                            .frame(width: 14, height: 14)
                                    }
                                    Text(provider.display_name)
                                }
                            } footer: {
                                HStack {
                                    Spacer()
                                    Button {
                                        Task { await login(provider: provider.id) }
                                    } label: {
                                        if loginInProgress == provider.id {
                                            ProgressView()
                                                .controlSize(.small)
                                        } else {
                                            Label(
                                                provider.accounts.isEmpty ? "Login…" : "Login New Account…",
                                                systemImage: "plus"
                                            )
                                        }
                                    }
                                    .disabled(loginInProgress != nil)
                                }
                            }
                        }
                    }

                    if let errorMessage {
                        Section {
                            Label(errorMessage, systemImage: "exclamationmark.triangle.fill")
                                .foregroundStyle(.red)
                                .font(.caption)
                        }
                    }
                }
                .formStyle(.grouped)
                .scrollContentBackground(.hidden)
            } else if pm.isRunning {
                Spacer()
                HStack { Spacer(); ProgressView().controlSize(.large); Spacer() }
                Text("Waiting for server…").foregroundStyle(.secondary)
                Spacer()
            } else {
                Spacer()
                ContentUnavailableView(
                    "Server Not Running",
                    systemImage: "server.rack",
                    description: Text("Enable the proxy server to manage accounts.")
                )
                Spacer()
            }
        }
    }

    private func activateAccount(provider: String, accountId: String) async {
        do {
            try await dataService.activateAccount(provider: provider, accountId: accountId)
            errorMessage = nil
        } catch {
            errorMessage = "Failed to activate account: \(error.localizedDescription)"
        }
    }

    private func removeAccount(provider: String, accountId: String) async {
        do {
            try await dataService.removeAccount(provider: provider, accountId: accountId)
            errorMessage = nil
        } catch {
            errorMessage = "Failed to remove account: \(error.localizedDescription)"
        }
    }

    private func login(provider: String) async {
        loginInProgress = provider
        errorMessage = nil
        do {
            try await CLIRunner.login(provider: provider)
            try? await Task.sleep(for: .seconds(1))
            await dataService.reloadAccounts()
        } catch {
            errorMessage = "Login failed: \(error.localizedDescription)"
        }
        loginInProgress = nil
    }
}

private struct AccountRow: View {
    let account: Components.Schemas.AccountDetail
    let providerName: String
    let onActivate: () -> Void
    let onRemove: () -> Void

    private var displayName: String {
        if let label = account.label, label != providerName {
            return label
        }
        return account.account_id
    }

    var body: some View {
        HStack {
            Button(action: onActivate) {
                Image(systemName: account.is_active ? "circle.inset.filled" : "circle")
                    .foregroundStyle(account.is_active ? Color.accentColor : Color.secondary)
            }
            .buttonStyle(.plain)
            .disabled(account.is_active)

            Text(displayName)
                .lineLimit(1)

            Spacer()

            HStack(spacing: 6) {
                Text(stateLabel)
                    .font(.caption)
                    .foregroundStyle(stateColor)

                if let remaining = remainingText {
                    Text(remaining)
                        .font(.caption2)
                        .foregroundStyle(.secondary)
                }
            }

            Button(role: .destructive, action: onRemove) {
                Image(systemName: "xmark.circle.fill")
                    .foregroundStyle(.secondary)
            }
            .buttonStyle(.plain)
        }
    }

    private var stateColor: Color {
        switch account.token_state {
        case .valid: .green
        case .expired: .orange
        case .invalid: .red
        }
    }

    private var stateLabel: String {
        switch account.token_state {
        case .valid: "Active"
        case .expired: "Expired"
        case .invalid: "Invalid"
        }
    }

    private var remainingText: String? {
        guard let expiresAt = account.expires_at else { return nil }
        let now = Int64(Date().timeIntervalSince1970)
        let remaining = expiresAt - now
        guard remaining > 0 else { return nil }

        let days = remaining / 86400
        let hours = (remaining % 86400) / 3600

        if days > 0 {
            return "expires \(days)d"
        } else if hours > 0 {
            return "expires \(hours)h"
        } else {
            return "expires <1h"
        }
    }
}

#Preview {
    AccountsView()
        .environment(ProcessManager())
        .environment(DataService())
}
