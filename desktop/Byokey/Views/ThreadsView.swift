import SwiftUI
import SwiftProtobuf

struct ThreadsView: View {
    @Environment(ProcessManager.self) private var pm
    @Environment(DataService.self) private var dataService
    @State private var selectedID: String?
    @State private var detail: AmpThreadDetail?
    @State private var isLoadingDetail = false
    @State private var detailError: String?

    var body: some View {
        // Deliberately bypasses DetailPage: a list+detail split view needs
        // the full detail-pane width and height, not the 1100pt centered
        // content area DetailPage enforces for typographic pages.
        VStack(alignment: .leading, spacing: 0) {
            HStack(alignment: .firstTextBaseline, spacing: 16) {
                Text("Threads")
                    .font(.system(size: 28, weight: .bold))
                Spacer(minLength: 16)
                if !dataService.ampThreads.isEmpty {
                    Text("\(dataService.ampThreads.count)")
                        .font(.system(size: 13, weight: .medium, design: .rounded))
                        .monospacedDigit()
                        .foregroundStyle(.secondary)
                        .padding(.horizontal, 8)
                        .padding(.vertical, 2)
                        .background(.secondary.opacity(0.1), in: Capsule())
                }
            }
            .padding(.horizontal, 24)
            .padding(.top, 24)
            .padding(.bottom, 12)

            Divider()

            content
                .frame(maxWidth: .infinity, maxHeight: .infinity)
        }
        .frame(maxWidth: .infinity, maxHeight: .infinity)
        .task(id: selectedID) { await loadSelected() }
    }

    @ViewBuilder
    private var content: some View {
        if pm.isReachable {
            if dataService.ampThreads.isEmpty, dataService.isLoading {
                ProgressView("Loading threads…")
                    .frame(maxWidth: .infinity, maxHeight: .infinity)
            } else if dataService.ampThreads.isEmpty {
                ContentUnavailableView(
                    "No Amp Threads",
                    systemImage: "bubble.left.and.bubble.right",
                    description: Text("Run Amp CLI to record conversations. Threads appear as they're written to disk.")
                )
            } else {
                HSplitView {
                    ThreadList(
                        threads: dataService.ampThreads,
                        selectedID: $selectedID
                    )
                    .frame(minWidth: 240, idealWidth: 300, maxWidth: 420)
                    .frame(maxHeight: .infinity)
                    .background(Color.surfaceSecondary.opacity(0.5))

                    ThreadDetailPane(
                        detail: detail,
                        isLoading: isLoadingDetail,
                        error: detailError,
                        emptyHint: selectedID == nil
                    )
                    .frame(minWidth: 360, maxWidth: .infinity, maxHeight: .infinity)
                }
                .frame(maxWidth: .infinity, maxHeight: .infinity)
            }
        } else if pm.isRunning {
            ServerStartingView()
        } else {
            ContentUnavailableView(
                "Server Not Running",
                systemImage: "server.rack",
                description: Text("Enable the proxy server to read Amp threads.")
            )
        }
    }

    private func loadSelected() async {
        guard let id = selectedID else {
            detail = nil
            return
        }
        isLoadingDetail = true
        detailError = nil
        defer { isLoadingDetail = false }
        if let got = await dataService.fetchThread(id: id) {
            detail = got
        } else {
            detail = nil
            detailError = "Failed to load thread"
        }
    }
}

// MARK: - Thread List

private struct ThreadList: View {
    let threads: [AmpThreadSummary]
    @Binding var selectedID: String?

    var body: some View {
        ScrollView {
            LazyVStack(spacing: 0) {
                ForEach(threads, id: \.id) { t in
                    ThreadRow(
                        thread: t,
                        isSelected: selectedID == t.id
                    ) {
                        selectedID = t.id
                    }
                }
            }
            .padding(.vertical, 4)
        }
        .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .top)
    }
}

private struct ThreadRow: View {
    let thread: AmpThreadSummary
    let isSelected: Bool
    let onTap: () -> Void
    @State private var isHovered = false

    var body: some View {
        Button(action: onTap) {
            VStack(alignment: .leading, spacing: 4) {
                Text(thread.title.isEmpty ? "Untitled" : thread.title)
                    .font(.system(size: 13, weight: .medium))
                    .lineLimit(2)
                    .multilineTextAlignment(.leading)
                    .foregroundStyle(titleColor)

                HStack(spacing: 6) {
                    Text(formatDate(thread.created))
                    Text("·")
                    Text("\(thread.messageCount) msg")
                    if thread.hasLastModel, !thread.lastModel.isEmpty {
                        Text("·")
                        Text(shortModel(thread.lastModel))
                            .lineLimit(1)
                            .truncationMode(.tail)
                    }
                }
                .font(.system(size: 10))
                .foregroundStyle(secondaryColor)
                .monospacedDigit()

                if thread.hasTotalInputTokens || thread.hasTotalOutputTokens {
                    HStack(spacing: 10) {
                        if thread.hasTotalInputTokens {
                            Label(formatTokens(thread.totalInputTokens), systemImage: "arrow.down")
                                .labelStyle(.titleAndIcon)
                        }
                        if thread.hasTotalOutputTokens {
                            Label(formatTokens(thread.totalOutputTokens), systemImage: "arrow.up")
                                .labelStyle(.titleAndIcon)
                        }
                    }
                    .font(.system(size: 9, design: .rounded))
                    .foregroundStyle(secondaryColor)
                    .monospacedDigit()
                }
            }
            .frame(maxWidth: .infinity, alignment: .leading)
            .padding(.horizontal, 12)
            .padding(.vertical, 9)
            .background(rowBackground, in: .rect(cornerRadius: 6))
            .padding(.horizontal, 6)
        }
        .buttonStyle(.plain)
        .contentShape(Rectangle())
        .onHover { isHovered = $0 }
    }

    private var titleColor: Color {
        if isSelected { return .white }
        return thread.title.isEmpty ? .secondary : .primary
    }

    private var secondaryColor: Color {
        isSelected ? .white.opacity(0.7) : .secondary.opacity(0.7)
    }

    private var rowBackground: Color {
        if isSelected {
            return Color.accentColor
        }
        if isHovered {
            return Color.primary.opacity(0.06)
        }
        return .clear
    }

    private func formatDate(_ ms: UInt64) -> String {
        let d = Date(timeIntervalSince1970: TimeInterval(ms) / 1000.0)
        let fmt = DateFormatter()
        if Calendar.current.isDateInToday(d) {
            fmt.dateFormat = "HH:mm"
        } else if Calendar.current.isDate(d, equalTo: Date(), toGranularity: .year) {
            fmt.dateFormat = "MMM d"
        } else {
            fmt.dateFormat = "yyyy-MM-dd"
        }
        return fmt.string(from: d)
    }

    private func shortModel(_ model: String) -> String {
        // "claude-opus-4-6" → "opus-4-6"; "gpt-4o" → "gpt-4o"; keep family + version
        let parts = model.split(separator: "-")
        guard parts.count >= 3, ["claude", "gemini"].contains(parts.first?.lowercased()) else {
            return model
        }
        return parts.dropFirst().joined(separator: "-")
    }
}

// MARK: - Detail Pane

private struct ThreadDetailPane: View {
    let detail: AmpThreadDetail?
    let isLoading: Bool
    let error: String?
    let emptyHint: Bool

    var body: some View {
        Group {
            if isLoading {
                ProgressView().frame(maxWidth: .infinity, maxHeight: .infinity)
            } else if let error {
                ContentUnavailableView("Error", systemImage: "exclamationmark.triangle", description: Text(error))
            } else if emptyHint {
                ContentUnavailableView("Select a thread", systemImage: "bubble.left.and.bubble.right")
            } else if let d = detail {
                ThreadDetailContent(detail: d)
            } else {
                ContentUnavailableView("Thread not found", systemImage: "questionmark.circle")
            }
        }
        .frame(maxWidth: .infinity, maxHeight: .infinity)
    }
}

private struct ThreadDetailContent: View {
    let detail: AmpThreadDetail

    var body: some View {
        VStack(alignment: .leading, spacing: 0) {
            header
                .frame(maxWidth: 820)
                .frame(maxWidth: .infinity, alignment: .center)
                .padding(.horizontal, 20)
                .padding(.vertical, 14)

            Divider()

            ScrollView {
                LazyVStack(alignment: .leading, spacing: 14) {
                    ForEach(Array(detail.messages.enumerated()), id: \.offset) { _, msg in
                        MessageView(message: msg)
                    }
                }
                .frame(maxWidth: 820)
                .frame(maxWidth: .infinity, alignment: .center)
                .padding(20)
            }
            .frame(maxWidth: .infinity, maxHeight: .infinity)
        }
        .frame(maxWidth: .infinity, maxHeight: .infinity)
    }

    private var header: some View {
        VStack(alignment: .leading, spacing: 4) {
            Text(detail.title.isEmpty ? "Untitled thread" : detail.title)
                .font(.system(size: 18, weight: .semibold))
                .lineLimit(2)

            HStack(spacing: 10) {
                metaItem(icon: "clock", text: relativeDate(detail.created))
                if !detail.agentMode.isEmpty {
                    metaItem(icon: "person.crop.circle", text: detail.agentMode)
                }
                metaItem(icon: "text.bubble", text: "\(detail.messages.count) messages")
                if let (inTok, outTok) = totalTokens, inTok > 0 || outTok > 0 {
                    metaItem(icon: "arrow.down", text: formatTokens(inTok))
                    metaItem(icon: "arrow.up", text: formatTokens(outTok))
                }
                Spacer()
                Text(detail.id)
                    .font(.system(size: 10, design: .monospaced))
                    .foregroundStyle(.tertiary)
                    .textSelection(.enabled)
            }
            .font(.system(size: 11))
            .foregroundStyle(.secondary)
        }
    }

    private var totalTokens: (UInt64, UInt64)? {
        var inTok: UInt64 = 0
        var outTok: UInt64 = 0
        for m in detail.messages where m.hasUsage {
            inTok += m.usage.inputTokens
            outTok += m.usage.outputTokens
        }
        return (inTok, outTok)
    }

    private func metaItem(icon: String, text: String) -> some View {
        Label { Text(text) } icon: { Image(systemName: icon) }
            .labelStyle(.titleAndIcon)
            .monospacedDigit()
    }

    private func relativeDate(_ ms: UInt64) -> String {
        let d = Date(timeIntervalSince1970: TimeInterval(ms) / 1000.0)
        let fmt = RelativeDateTimeFormatter()
        fmt.unitsStyle = .abbreviated
        return fmt.localizedString(for: d, relativeTo: Date())
    }
}

// MARK: - Message

private struct MessageView: View {
    let message: AmpMessage

    var body: some View {
        VStack(alignment: .leading, spacing: 8) {
            HStack(spacing: 8) {
                roleBadge
                if message.hasUsage, !message.usage.model.isEmpty {
                    Text(message.usage.model)
                        .font(.system(size: 10, design: .monospaced))
                        .foregroundStyle(.tertiary)
                }
                Spacer()
                if message.hasUsage {
                    usageRow
                }
            }

            VStack(alignment: .leading, spacing: 8) {
                ForEach(Array(message.content.enumerated()), id: \.offset) { _, block in
                    ContentBlockView(block: block)
                }
            }
        }
        .frame(maxWidth: .infinity, alignment: .leading)
    }

    @ViewBuilder
    private var roleBadge: some View {
        let (label, color): (String, Color) = switch message.role {
        case "user": ("User", .blue)
        case "assistant": ("Assistant", .purple)
        case "info": ("Info", .gray)
        default: (message.role.capitalized, .gray)
        }
        Text(label)
            .font(.system(size: 10, weight: .semibold))
            .foregroundStyle(color)
            .padding(.horizontal, 7)
            .padding(.vertical, 2)
            .background(color.opacity(0.12), in: Capsule())
    }

    private var usageRow: some View {
        HStack(spacing: 8) {
            if message.usage.hasInputTokens {
                Label(formatTokens(message.usage.inputTokens), systemImage: "arrow.down")
            }
            if message.usage.hasOutputTokens {
                Label(formatTokens(message.usage.outputTokens), systemImage: "arrow.up")
            }
        }
        .font(.system(size: 10, design: .rounded))
        .foregroundStyle(.tertiary)
        .labelStyle(.titleAndIcon)
        .monospacedDigit()
    }
}

// MARK: - Content Blocks

private struct ContentBlockView: View {
    let block: AmpContentBlock

    var body: some View {
        switch block.block {
        case .text(let t):
            TextBlock(text: t)
        case .thinking(let t):
            CollapsibleBlock(
                label: "Thinking",
                icon: "brain",
                accent: .orange,
                defaultExpanded: false
            ) {
                Text(t)
                    .font(.system(size: 12, design: .monospaced))
                    .foregroundStyle(.secondary)
                    .textSelection(.enabled)
                    .frame(maxWidth: .infinity, alignment: .leading)
            }
        case .toolUse(let tu):
            ToolUseBlock(use: tu)
        case .toolResult(let tr):
            ToolResultBlock(result: tr)
        case .unknownType(let name):
            Label("Unknown block: \(name)", systemImage: "questionmark.square.dashed")
                .font(.system(size: 11))
                .foregroundStyle(.tertiary)
        case .none:
            EmptyView()
        }
    }
}

private struct TextBlock: View {
    let text: String

    var body: some View {
        Text(text)
            .font(.system(size: 13))
            .textSelection(.enabled)
            .frame(maxWidth: .infinity, alignment: .leading)
    }
}

private struct ToolUseBlock: View {
    let use: AmpToolUse

    var body: some View {
        CollapsibleBlock(
            label: use.name.isEmpty ? "Tool" : use.name,
            icon: "hammer",
            accent: .indigo,
            defaultExpanded: false,
            trailing: {
                AnyView(
                    Text(use.id.suffix(8))
                        .font(.system(size: 9, design: .monospaced))
                        .foregroundStyle(.tertiary)
                )
            }
        ) {
            if use.hasInput {
                JSONView(json: structToJSON(use.input))
            } else {
                Text("(no input)")
                    .font(.system(size: 11))
                    .foregroundStyle(.tertiary)
            }
        }
    }
}

private struct ToolResultBlock: View {
    let result: AmpToolResult

    private var statusColor: Color {
        switch result.run.status {
        case "done": .green
        case "error": .red
        case "cancelled", "rejected-by-user": .orange
        default: .gray
        }
    }

    var body: some View {
        CollapsibleBlock(
            label: "Result",
            icon: "checkmark.seal",
            accent: statusColor,
            defaultExpanded: false,
            trailing: {
                AnyView(
                    Text(result.run.status)
                        .font(.system(size: 10, weight: .medium))
                        .foregroundStyle(statusColor)
                )
            }
        ) {
            VStack(alignment: .leading, spacing: 6) {
                if result.run.hasError {
                    JSONView(json: valueToJSON(result.run.error), accent: .red)
                }
                if result.run.hasResult {
                    JSONView(json: valueToJSON(result.run.result))
                }
                if !result.run.hasError, !result.run.hasResult {
                    Text("(no output)")
                        .font(.system(size: 11))
                        .foregroundStyle(.tertiary)
                }
            }
        }
    }
}

// MARK: - Collapsible container

private struct CollapsibleBlock<Content: View>: View {
    let label: String
    let icon: String
    let accent: Color
    var defaultExpanded: Bool = false
    var trailing: (() -> AnyView)?
    @ViewBuilder var content: () -> Content

    @State private var expanded: Bool = false

    init(
        label: String,
        icon: String,
        accent: Color,
        defaultExpanded: Bool = false,
        trailing: (() -> AnyView)? = nil,
        @ViewBuilder content: @escaping () -> Content
    ) {
        self.label = label
        self.icon = icon
        self.accent = accent
        self.defaultExpanded = defaultExpanded
        self.trailing = trailing
        self.content = content
        _expanded = State(initialValue: defaultExpanded)
    }

    var body: some View {
        VStack(alignment: .leading, spacing: 0) {
            Button {
                withAnimation(.easeInOut(duration: 0.15)) { expanded.toggle() }
            } label: {
                HStack(spacing: 6) {
                    Image(systemName: "chevron.right")
                        .font(.system(size: 9, weight: .semibold))
                        .foregroundStyle(.tertiary)
                        .rotationEffect(.degrees(expanded ? 90 : 0))
                    Image(systemName: icon)
                        .font(.system(size: 11))
                        .foregroundStyle(accent)
                    Text(label)
                        .font(.system(size: 11, weight: .medium))
                    Spacer()
                    trailing?()
                }
                .contentShape(Rectangle())
            }
            .buttonStyle(.plain)

            if expanded {
                content()
                    .padding(.top, 8)
                    .padding(.leading, 20)
                    .transition(.opacity.combined(with: .move(edge: .top)))
            }
        }
        .padding(.vertical, 6)
        .padding(.horizontal, 10)
        .background(accent.opacity(0.04), in: .rect(cornerRadius: 8))
        .overlay(
            RoundedRectangle(cornerRadius: 8)
                .strokeBorder(accent.opacity(0.15), lineWidth: 0.5)
        )
    }
}

// MARK: - JSON rendering

private struct JSONView: View {
    let json: String
    var accent: Color = .secondary

    var body: some View {
        ScrollView(.horizontal, showsIndicators: false) {
            Text(json)
                .font(.system(size: 11, design: .monospaced))
                .foregroundStyle(accent)
                .textSelection(.enabled)
                .padding(8)
                .frame(maxWidth: .infinity, alignment: .leading)
        }
        .background(Color.black.opacity(0.04), in: .rect(cornerRadius: 6))
    }
}

private func structToJSON(_ s: Google_Protobuf_Struct) -> String {
    (try? s.jsonString()) ?? "{}"
}

private func valueToJSON(_ v: Google_Protobuf_Value) -> String {
    (try? v.jsonString()) ?? "null"
}

#Preview {
    ThreadsView()
        .environment(AppEnvironment.shared)
        .environment(ProcessManager())
        .environment(DataService())
}
