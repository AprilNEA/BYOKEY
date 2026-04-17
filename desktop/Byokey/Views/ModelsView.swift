import SwiftUI

struct ModelsView: View {
    @Environment(ProcessManager.self) private var pm
    @Environment(DataService.self) private var dataService
    @State private var searchText = ""
    @State private var copiedId: String?

    private var grouped: [(provider: String, models: [ModelEntry])] {
        let source = dataService.models
        let filtered = searchText.isEmpty
            ? source
            : source.filter {
                $0.id.localizedCaseInsensitiveContains(searchText)
                    || $0.owned_by.localizedCaseInsensitiveContains(searchText)
            }

        return Dictionary(grouping: filtered, by: \.owned_by)
            .sorted(by: { $0.key < $1.key })
            .map { (provider: $0.key, models: $0.value.sorted(by: { $0.id < $1.id })) }
    }

    var body: some View {
        DetailPage("Models", accessory: {
            if pm.isReachable {
                searchField
            }
        }) {
            if pm.isReachable {
                Form {
                    if dataService.models.isEmpty, dataService.isLoading {
                        Section {
                            HStack {
                                Spacer()
                                ProgressView().controlSize(.small)
                                Text("Loading models…").foregroundStyle(.secondary)
                                Spacer()
                            }
                            .padding(.vertical, 8)
                        }
                    } else if grouped.isEmpty {
                        Section {
                            if searchText.isEmpty {
                                Text("No models available")
                                    .foregroundStyle(.secondary)
                            } else {
                                Text("No models matching \"\(searchText)\"")
                                    .foregroundStyle(.secondary)
                            }
                        }
                    } else {
                        summarySection

                        ForEach(grouped, id: \.provider) { group in
                            Section {
                                ForEach(group.models) { model in
                                    modelRow(model)
                                }
                            } header: {
                                HStack {
                                    Circle()
                                        .fill(providerColor(group.provider))
                                        .frame(width: 8, height: 8)
                                    Text(group.provider)
                                    Spacer()
                                    Text("\(group.models.count)")
                                        .foregroundStyle(.tertiary)
                                        .monospacedDigit()
                                }
                            }
                        }
                    }
                }
                .formStyle(.grouped)
                .scrollContentBackground(.hidden)
            } else if pm.isRunning {
                ServerStartingView()
            } else {
                Spacer()
                ContentUnavailableView(
                    "Server Not Running",
                    systemImage: "cpu",
                    description: Text("Enable the proxy server to browse models.")
                )
                Spacer()
            }
        }
    }

    private var searchField: some View {
        HStack(spacing: 4) {
            Image(systemName: "magnifyingglass")
                .foregroundStyle(.secondary)
            TextField("Filter models…", text: $searchText)
                .textFieldStyle(.plain)
        }
        .padding(.horizontal, 10)
        .padding(.vertical, 6)
        .background(Color.surfaceSecondary, in: .rect(cornerRadius: 8))
        .frame(maxWidth: 220)
    }

    // MARK: - Summary

    private var summarySection: some View {
        Section {
            HStack(spacing: 16) {
                LabeledContent("Total") {
                    Text("\(dataService.models.count)")
                        .fontWeight(.semibold)
                        .monospacedDigit()
                }

                Divider().frame(height: 16)

                LabeledContent("Providers") {
                    Text("\(grouped.count)")
                        .fontWeight(.semibold)
                        .monospacedDigit()
                }

                Divider().frame(height: 16)

                HStack(spacing: 4) {
                    ForEach(grouped.prefix(6), id: \.provider) { group in
                        HStack(spacing: 3) {
                            Circle()
                                .fill(providerColor(group.provider))
                                .frame(width: 6, height: 6)
                            Text("\(group.models.count)")
                                .monospacedDigit()
                        }
                    }
                }
                .font(.caption)
                .foregroundStyle(.secondary)
            }
        }
    }

    // MARK: - Row

    private func modelRow(_ model: ModelEntry) -> some View {
        HStack {
            Text(model.id)
                .fontDesign(.monospaced)
                .lineLimit(1)

            Spacer()

            if let usage = dataService.usage?.models[model.id] {
                Text("\(usage.requests) req")
                    .font(.caption2)
                    .foregroundStyle(.tertiary)
                    .monospacedDigit()
            }

            Button {
                NSPasteboard.general.clearContents()
                NSPasteboard.general.setString(model.id, forType: .string)
                copiedId = model.id
                Task {
                    try? await Task.sleep(for: .seconds(1.5))
                    if copiedId == model.id { copiedId = nil }
                }
            } label: {
                Image(systemName: copiedId == model.id ? "checkmark" : "doc.on.doc")
                    .foregroundStyle(copiedId == model.id ? .green : .secondary)
            }
            .buttonStyle(.borderless)
            .help("Copy model ID")
        }
    }

    // MARK: - Helpers

    private func providerColor(_ provider: String) -> Color {
        switch provider.lowercased() {
        case let p where p.contains("claude"), let p where p.contains("anthropic"):
            return .orange
        case let p where p.contains("openai"), let p where p.contains("codex"):
            return .green
        case let p where p.contains("copilot"), let p where p.contains("github"):
            return .purple
        case let p where p.contains("gemini"), let p where p.contains("google"):
            return .blue
        case let p where p.contains("kiro"), let p where p.contains("amazon"):
            return .yellow
        default:
            return .gray
        }
    }
}

#Preview {
    ModelsView()
        .environment(ProcessManager())
        .environment(DataService())
}
