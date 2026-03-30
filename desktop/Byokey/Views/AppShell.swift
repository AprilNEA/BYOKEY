import SwiftUI

enum SidebarItem: String, Identifiable, CaseIterable {
    case activity = "Activity"
    case overview = "Overview"
    case accounts = "Accounts"
    case models = "Models"
    case amp = "Amp"
    case usage = "Usage"
    case settings = "Settings"

    var id: Self { self }

    var icon: String {
        switch self {
        case .activity: "waveform.path.ecg"
        case .overview: "square.grid.2x2"
        case .accounts: "person.2"
        case .models: "cpu"
        case .amp: "bolt.fill"
        case .usage: "chart.bar"
        case .settings: "gearshape"
        }
    }

    var section: String? {
        switch self {
        case .activity, .overview: nil
        case .accounts, .models: "Provider"
        case .amp: "Agent"
        case .usage, .settings: "Proxy"
        }
    }
}

struct AppShell<Detail: View>: View {
    @Binding private var selection: SidebarItem?
    @Binding private var showLog: Bool
    @ViewBuilder private let detail: () -> Detail

    init(
        selection: Binding<SidebarItem?>,
        showLog: Binding<Bool>,
        @ViewBuilder detail: @escaping () -> Detail
    ) {
        _selection = selection
        _showLog = showLog
        self.detail = detail
    }

    var body: some View {
        ZStack {
            LinearGradient(
                colors: [
                    Color(red: 0.98, green: 0.96, blue: 0.98),
                    Color(red: 0.95, green: 0.96, blue: 1.0),
                    Color(red: 0.93, green: 0.95, blue: 1.0),
                ],
                startPoint: .topLeading,
                endPoint: .bottomTrailing
            )
            .ignoresSafeArea()

            HStack(spacing: 0) {
                SidebarView(selection: $selection, showLog: $showLog)
                    .frame(width: 200)

                VStack(spacing: 0) {
                    GeometryReader { _ in
                        detail()
                            .frame(maxWidth: .infinity, maxHeight: .infinity)
                    }

                    if showLog {
                        Divider()
                        LogPanel()
                    }
                }
            }
            .frame(maxWidth: .infinity, maxHeight: .infinity)
        }
    }
}

struct DetailPage<Accessory: View, Content: View>: View {
    let title: String
    @ViewBuilder let accessory: () -> Accessory
    @ViewBuilder let content: () -> Content

    init(_ title: String, @ViewBuilder content: @escaping () -> Content) where Accessory == EmptyView {
        self.title = title
        self.accessory = { EmptyView() }
        self.content = content
    }

    init(
        _ title: String,
        @ViewBuilder accessory: @escaping () -> Accessory,
        @ViewBuilder content: @escaping () -> Content
    ) {
        self.title = title
        self.accessory = accessory
        self.content = content
    }

    var body: some View {
        VStack(alignment: .leading, spacing: 12) {
            HStack(alignment: .firstTextBaseline, spacing: 16) {
                Text(title)
                    .font(.system(size: 28, weight: .bold))

                Spacer(minLength: 16)
                accessory()
            }
            .padding(.bottom, 8)

            content()
        }
        .padding(24)
        .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .topLeading)
    }
}

private struct SidebarView: View {
    @Binding var selection: SidebarItem?
    @Binding var showLog: Bool
    @Environment(ProcessManager.self) private var pm

    var body: some View {
        VStack(alignment: .leading, spacing: 0) {
            ForEach(SidebarItem.allCases) { item in
                if let section = item.section,
                   item == SidebarItem.allCases.first(where: { $0.section == section })
                {
                    Text(section)
                        .font(.system(size: 12, weight: .semibold))
                        .foregroundStyle(.secondary)
                        .padding(.leading, 20)
                        .padding(.top, 20)
                        .padding(.bottom, 6)
                }

                Button {
                    selection = item
                } label: {
                    Label {
                        Text(item.rawValue)
                    } icon: {
                        Image(systemName: item.icon)
                            .frame(width: 22)
                    }
                    .frame(maxWidth: .infinity, alignment: .leading)
                    .padding(.vertical, 7)
                    .padding(.horizontal, 14)
                    .background(
                        selection == item
                            ? Color.accentColor.opacity(0.12)
                            : Color.clear,
                        in: .rect(cornerRadius: 8)
                    )
                    .foregroundStyle(selection == item ? Color.accentColor : .primary)
                }
                .buttonStyle(.plain)
                .padding(.horizontal, 10)
            }

            Spacer()

            Button {
                withAnimation(.easeInOut(duration: 0.2)) {
                    showLog.toggle()
                }
            } label: {
                HStack(spacing: 6) {
                    Image(systemName: "terminal")
                        .frame(width: 22)
                    Text("Log")
                    Spacer()
                    if !pm.logs.isEmpty {
                        Text("\(pm.logs.count)")
                            .font(.caption2)
                            .monospacedDigit()
                            .foregroundStyle(.tertiary)
                    }
                    Image(systemName: showLog ? "chevron.down" : "chevron.up")
                        .font(.caption2)
                        .foregroundStyle(.tertiary)
                }
                .foregroundStyle(.secondary)
                .padding(.horizontal, 24)
                .padding(.vertical, 12)
            }
            .buttonStyle(.plain)
        }
        .font(.system(size: 14))
        .padding(.top, 44)
    }
}

private struct LogPanel: View {
    @Environment(ProcessManager.self) private var pm

    var body: some View {
        VStack(spacing: 0) {
            ScrollViewReader { proxy in
                ScrollView([.vertical, .horizontal], showsIndicators: true) {
                    if pm.logs.isEmpty {
                        Text("Waiting for log entries…")
                            .font(.system(size: 11, design: .monospaced))
                            .foregroundStyle(.tertiary)
                            .frame(maxWidth: .infinity, alignment: .leading)
                            .padding(8)
                    } else {
                        VStack(alignment: .leading, spacing: 0) {
                            ForEach(Array(pm.logs.enumerated()), id: \.offset) { i, line in
                                Text(AnsiParser.parse(line))
                                    .font(.system(size: 11, design: .monospaced))
                                    .textSelection(.enabled)
                                    .frame(maxWidth: .infinity, alignment: .leading)
                                    .fixedSize(horizontal: true, vertical: false)
                                    .id(i)
                            }
                        }
                        .padding(8)
                    }
                }
                .frame(height: 140)
                .onChange(of: pm.logs.count) {
                    proxy.scrollTo(pm.logs.count - 1, anchor: .bottom)
                }
            }

            HStack(spacing: 12) {
                Text("\(pm.logs.count) lines")
                    .foregroundStyle(.tertiary)
                    .monospacedDigit()
                Spacer()
                Button("Clear", systemImage: "trash") {
                    pm.clearLogs()
                }
                .buttonStyle(.borderless)
                .labelStyle(.iconOnly)
            }
            .font(.caption2)
            .padding(.horizontal, 12)
            .padding(.vertical, 4)
            .background(.ultraThinMaterial)
        }
    }
}
