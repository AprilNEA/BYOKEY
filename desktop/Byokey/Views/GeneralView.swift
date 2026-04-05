import SwiftUI

struct GeneralView: View {
    @Environment(ProcessManager.self) private var pm
    @Environment(DataService.self) private var dataService

    var body: some View {
        DetailPage("Activity") {
            if pm.isReachable {
                if dataService.providers.isEmpty, dataService.isLoading {
                    loadingState
                } else if dataService.providers.isEmpty {
                    emptyState
                } else {
                    DashboardStatsRow()
                        .layoutPriority(1)
                    DashboardHistoryChart()
                        .layoutPriority(1)
                    if let rateLimits = dataService.rateLimits,
                       rateLimits.providers.contains(where: {
                           $0.accounts.contains(where: { !$0.snapshot.headers.additionalProperties.isEmpty })
                       })
                    {
                        DashboardRateLimitsCard(data: rateLimits)
                    }
                }
            } else if pm.isRunning {
                Spacer()
                HStack { Spacer(); ProgressView().controlSize(.large); Spacer() }
                Text("Waiting for server…").foregroundStyle(.secondary)
                Spacer()
            } else {
                Spacer()
                ContentUnavailableView(
                    "Server Not Running",
                    systemImage: "waveform.path.ecg",
                    description: Text("Enable the proxy server to view activity.")
                )
                Spacer()
            }

            if let error = pm.errorMessage {
                Label(error, systemImage: "exclamationmark.triangle.fill")
                    .foregroundStyle(.red)
                    .font(.caption)
                    .frame(maxWidth: .infinity, alignment: .leading)
            }

            Spacer(minLength: 0)
        }
    }

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
        .frame(width: 700, height: 600)
}
