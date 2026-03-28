import SwiftUI

struct DashboardLogCard: View {
    @Environment(ProcessManager.self) private var pm
    @State private var isExpanded = false

    var body: some View {
        Card("LOG") {
            ScrollViewReader { proxy in
                ScrollView([.vertical, .horizontal], showsIndicators: true) {
                    if pm.logs.isEmpty {
                        Text("Waiting for log entries…")
                            .font(.system(size: 11, design: .monospaced))
                            .foregroundStyle(.tertiary)
                            .frame(maxWidth: .infinity, alignment: .leading)
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
                    }
                }
                .frame(height: isExpanded ? 200 : 80)
                .animation(.easeInOut(duration: 0.2), value: isExpanded)
                .onChange(of: pm.logs.count) {
                    proxy.scrollTo(pm.logs.count - 1, anchor: .bottom)
                }
            }

            Divider()

            HStack(spacing: 12) {
                Text("\(pm.logs.count) lines")
                    .foregroundStyle(.tertiary)
                    .monospacedDigit()
                Spacer()
                Button(isExpanded ? "Collapse" : "Expand", systemImage: isExpanded ? "chevron.up" : "chevron.down") {
                    isExpanded.toggle()
                }
                .buttonStyle(.borderless)
                .labelStyle(.iconOnly)
                Button("Clear", systemImage: "trash") {
                    pm.clearLogs()
                }
                .buttonStyle(.borderless)
                .labelStyle(.iconOnly)
            }
            .font(.caption2)
        }
    }
}
