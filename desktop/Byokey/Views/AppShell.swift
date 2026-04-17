import SwiftUI

enum SidebarItem: String, Identifiable, CaseIterable {
    case activity = "Activity"
    case overview = "Overview"
    case accounts = "Accounts"
    case models = "Models"
    case amp = "Amp"
    case threads = "Threads"
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
        case .threads: "bubble.left.and.bubble.right"
        case .usage: "chart.bar"
        case .settings: "gearshape"
        }
    }

    var section: String? {
        switch self {
        case .activity, .overview: nil
        case .accounts, .models: "Provider"
        case .amp, .threads: "Agent"
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
            CanvasBackground()

            HStack(spacing: 0) {
                SidebarView(selection: $selection, showLog: $showLog)
                    .frame(width: 200)

                VStack(spacing: 0) {
                    detail()
                        .frame(maxWidth: .infinity, maxHeight: .infinity)
                        .clipShape(.rect(cornerRadius: DetailContainer.cornerRadius))

                    if showLog {
                        Divider()
                            .padding(.horizontal, 12)
                        LogPanel()
                    }
                }
                .modifier(DetailContainer())
                // `.windowStyle(.hiddenTitleBar)` lets content flow under the
                // traffic-light region (~28pt). Use 40pt on top so the visible
                // gap below the traffic lights equals the 12pt gap at the
                // bottom of the window.
                .padding(EdgeInsets(top: 40, leading: 0, bottom: 12, trailing: 12))
            }
            .frame(maxWidth: .infinity, maxHeight: .infinity)
        }
    }
}

/// Surge-style floating "card" container around the detail pane: rounded
/// rectangle with an elevated fill and a hairline stroke, sitting on the
/// window's canvas background with a small inset on every edge. Applied at
/// the shell level so every tab's content gets the treatment without each
/// view having to know about it.
private struct DetailContainer: ViewModifier {
    static let cornerRadius: CGFloat = 14
    @Environment(\.colorScheme) private var colorScheme

    func body(content: Content) -> some View {
        content
            .background(
                Color.surfacePrimary,
                in: .rect(cornerRadius: Self.cornerRadius)
            )
            .overlay {
                RoundedRectangle(cornerRadius: Self.cornerRadius)
                    .strokeBorder(strokeColor, lineWidth: 0.5)
            }
            .shadow(color: shadowColor, radius: 14, x: 0, y: 4)
    }

    private var strokeColor: Color {
        colorScheme == .dark ? .white.opacity(0.07) : .black.opacity(0.08)
    }

    private var shadowColor: Color {
        colorScheme == .dark ? .black.opacity(0.35) : .black.opacity(0.05)
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
        .frame(maxWidth: 1100, maxHeight: .infinity, alignment: .topLeading)
        .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .top)
    }
}

private struct SidebarView: View {
    @Binding var selection: SidebarItem?
    @Binding var showLog: Bool
    @Environment(ProcessManager.self) private var pm
    @Environment(AppEnvironment.self) private var appEnv

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
                    withAnimation(.easeInOut(duration: 0.15)) {
                        selection = item
                    }
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

            Divider()
                .padding(.horizontal, 20)

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
                .padding(.vertical, 10)
            }
            .buttonStyle(.plain)

            Divider()
                .padding(.horizontal, 20)

            DaemonStatusRow()
        }
        .font(.system(size: 14))
        .padding(.top, 44)
    }
}

// MARK: - Daemon Status

private struct DaemonStatusRow: View {
    @Environment(ProcessManager.self) private var pm
    @Environment(AppEnvironment.self) private var appEnv

    private var status: (dot: StatusDot.Status, label: String, tone: Color) {
        if pm.isReachable {
            return (.active, "Connected", .green)
        }
        if pm.isRunning {
            return (.warning, "Starting…", .orange)
        }
        return (.error, "Disconnected", .secondary)
    }

    var body: some View {
        HStack(spacing: 8) {
            StatusDot(status.dot, size: 7)

            VStack(alignment: .leading, spacing: 1) {
                Text(status.label)
                    .font(.system(size: 11, weight: .medium))
                    .foregroundStyle(status.tone == .secondary ? Color.secondary : status.tone)

                Text(verbatim: "127.0.0.1:\(appEnv.port)")
                    .font(.system(size: 9, design: .monospaced))
                    .foregroundStyle(.tertiary)
            }

            Spacer()

            Button {
                if pm.isRunning {
                    pm.stop()
                } else {
                    pm.start()
                }
            } label: {
                Image(systemName: pm.isRunning ? "stop.circle" : "play.circle")
                    .font(.system(size: 13))
                    .foregroundStyle(.secondary)
                    .contentTransition(.symbolEffect(.replace))
            }
            .buttonStyle(.plain)
            .help(pm.isRunning ? "Stop daemon" : "Start daemon")
        }
        .padding(.horizontal, 20)
        .padding(.vertical, 10)
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
