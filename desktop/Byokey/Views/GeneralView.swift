import SwiftUI

struct GeneralView: View {
    @Environment(ProcessManager.self) private var pm
    @Environment(DataService.self) private var dataService

    var body: some View {
        ScrollView {
            VStack(spacing: 16) {
                DashboardStatusBar()

                if pm.isReachable {
                    if dataService.providers.isEmpty, dataService.isLoading {
                        loadingState
                    } else if dataService.providers.isEmpty {
                        emptyState
                    } else {
                        DashboardStatsRow()
                        DashboardHistoryChart()
                        DashboardActivityCard()

                        if let rateLimits = dataService.rateLimits,
                           rateLimits.providers.contains(where: {
                               $0.accounts.contains(where: { !$0.snapshot.headers.isEmpty })
                           })
                        {
                            DashboardRateLimitsCard(data: rateLimits)
                        }
                    }
                }

                if let error = pm.errorMessage {
                    Label(error, systemImage: "exclamationmark.triangle.fill")
                        .foregroundStyle(.red)
                        .font(.caption)
                        .frame(maxWidth: .infinity, alignment: .leading)
                }
            }
            .padding(20)
        }
        .navigationTitle("Dashboard")
    }

    // MARK: - Empty / Loading States

    private var loadingState: some View {
        Card("") {
            HStack {
                Spacer()
                ProgressView()
                    .controlSize(.regular)
                Text("Loading…")
                    .foregroundStyle(.secondary)
                Spacer()
            }
            .padding(.vertical, 20)
        }
    }

    private var emptyState: some View {
        Card("GETTING STARTED") {
            VStack(alignment: .leading, spacing: 12) {
                Label("No provider accounts configured yet.", systemImage: "person.crop.circle.badge.plus")
                    .foregroundStyle(.secondary)

                Text("Add a provider account to start proxying AI API requests through BYOKEY.")
                    .font(.caption)
                    .foregroundStyle(.tertiary)
            }
        }
    }
}

#Preview {
    GeneralView()
        .environment(AppEnvironment.shared)
        .environment(ProcessManager())
        .environment(DataService())
        .frame(width: 640, height: 600)
}
