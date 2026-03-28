import SwiftUI

struct DashboardActivityCard: View {
    @Environment(DataService.self) private var dataService
    @State private var activityTab: ActivityTab = .providers

    var body: some View {
        Card("ACTIVITY") {
            Picker("", selection: $activityTab) {
                ForEach(ActivityTab.allCases, id: \.self) { tab in
                    Text(tab.rawValue).tag(tab)
                }
            }
            .pickerStyle(.segmented)
            .labelsHidden()

            switch activityTab {
            case .providers:
                providersList
            case .models:
                topModelsList
            }
        }
    }

    private var providersList: some View {
        VStack(spacing: 6) {
            if dataService.providers.isEmpty {
                Text("No providers")
                    .font(.caption)
                    .foregroundStyle(.tertiary)
                    .frame(maxWidth: .infinity, alignment: .center)
                    .padding(.vertical, 8)
            } else {
                ForEach(dataService.providers, id: \.id) { p in
                    HStack(spacing: 8) {
                        Circle()
                            .fill(providerColor(p))
                            .frame(width: 7, height: 7)
                        Text(p.display_name)
                            .lineLimit(1)
                        Spacer()
                        Text("\(p.models_count) models")
                            .foregroundStyle(.tertiary)
                            .monospacedDigit()
                    }
                    .font(.caption)
                    .opacity(p.enabled ? 1 : 0.5)
                }
            }
        }
    }

    private var topModelsList: some View {
        Group {
            let sorted = (dataService.usage?.models ?? [:])
                .sorted { $0.value.requests > $1.value.requests }
                .prefix(8)

            if sorted.isEmpty {
                Text("No model usage yet")
                    .font(.caption)
                    .foregroundStyle(.tertiary)
                    .frame(maxWidth: .infinity, alignment: .center)
                    .padding(.vertical, 8)
            } else {
                let maxReqs = sorted.first?.value.requests ?? 1

                VStack(spacing: 6) {
                    ForEach(Array(sorted), id: \.key) { model, stats in
                        VStack(spacing: 3) {
                            HStack {
                                Text(model)
                                    .lineLimit(1)
                                    .truncationMode(.middle)
                                Spacer()
                                Text("\(stats.requests) req")
                                    .foregroundStyle(.secondary)
                                    .monospacedDigit()
                            }
                            .font(.caption)

                            GeometryReader { geo in
                                RoundedRectangle(cornerRadius: 2)
                                    .fill(.blue.gradient.opacity(0.3))
                                    .frame(
                                        width: geo.size.width
                                            * CGFloat(stats.requests)
                                            / CGFloat(max(maxReqs, 1))
                                    )
                            }
                            .frame(height: 4)
                        }
                    }
                }
            }
        }
    }

    private func providerColor(_ p: Components.Schemas.ProviderStatus) -> Color {
        switch p.auth_status {
        case .valid: .green
        case .expired: .orange
        case .not_configured: .gray
        }
    }
}

enum ActivityTab: String, CaseIterable {
    case providers = "Providers"
    case models = "Models"
}
