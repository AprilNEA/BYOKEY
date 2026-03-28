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
        Group {
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
                        ForEach(grouped, id: \.provider) { group in
                            Section(group.provider) {
                                ForEach(group.models) { model in
                                    modelRow(model)
                                }
                            }
                        }
                    }
                }
                .formStyle(.grouped)
                .searchable(text: $searchText, prompt: "Filter models…")
            } else if pm.isRunning {
                ContentUnavailableView {
                    ProgressView().controlSize(.large)
                } description: {
                    Text("Waiting for server…")
                }
            } else {
                ContentUnavailableView(
                    "Server Not Running",
                    systemImage: "cpu",
                    description: Text("Enable the proxy server to browse models.")
                )
            }
        }
        .navigationTitle("Models")
    }

    private func modelRow(_ model: ModelEntry) -> some View {
        HStack {
            Text(model.id)
                .fontDesign(.monospaced)

            Spacer()

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
}

#Preview {
    ModelsView()
        .environment(ProcessManager())
        .environment(DataService())
}
