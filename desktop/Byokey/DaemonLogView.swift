import SwiftUI

@Observable
final class DaemonLogStream {
    private(set) var lines: [String] = []
    private var process: Process?

    func start() {
        guard process == nil else { return }

        let proc = Process()
        proc.executableURL = URL(filePath: "/usr/bin/log")
        proc.arguments = [
            "stream",
            "--predicate",
            """
            process == "io.byokey.desktop.daemon" \
            OR (sender == "launchd" AND eventMessage CONTAINS[cd] "byokey")
            """,
            "--style", "compact",
            "--level", "info",
        ]

        let pipe = Pipe()
        proc.standardOutput = pipe
        proc.standardError = Pipe()

        pipe.fileHandleForReading.readabilityHandler = { [weak self] handle in
            let data = handle.availableData
            guard !data.isEmpty,
                  let str = String(data: data, encoding: .utf8)
            else { return }

            let newLines = str
                .components(separatedBy: .newlines)
                .filter { !$0.isEmpty && !$0.hasPrefix("Filtering the log data") }

            guard !newLines.isEmpty else { return }
            DispatchQueue.main.async { [weak self] in
                guard let self else { return }
                self.lines.append(contentsOf: newLines)
                if self.lines.count > 1000 {
                    self.lines = Array(self.lines.suffix(500))
                }
            }
        }

        proc.terminationHandler = { [weak self] _ in
            DispatchQueue.main.async {
                self?.process = nil
            }
        }

        do {
            try proc.run()
            process = proc
        } catch {
            lines.append("[Error] Failed to start log stream: \(error.localizedDescription)")
        }
    }

    func stop() {
        if let proc = process, proc.isRunning {
            proc.terminate()
        }
        process = nil
    }

    func clear() {
        lines.removeAll()
    }

    deinit { stop() }
}

struct DaemonLogView: View {
    @State var stream = DaemonLogStream()

    var body: some View {
        Section {
            VStack(spacing: 0) {
                logContent
                    .frame(height: 48)

                Divider()

                HStack(spacing: 12) {
                    Text("\(stream.lines.count) lines")
                        .foregroundStyle(.tertiary)
                        .monospacedDigit()
                    Spacer()
                    Button("Clear", systemImage: "trash") {
                        stream.clear()
                    }
                    .buttonStyle(.borderless)
                    .labelStyle(.iconOnly)
                }
                .font(.caption2)
                .padding(.top, 4)
            }
        } header: {
            Text("Log")
        }
        .onAppear { stream.start() }
        .onDisappear { stream.stop() }
    }

    private var logContent: some View {
        ScrollViewReader { proxy in
            ScrollView(.vertical, showsIndicators: false) {
                if stream.lines.isEmpty {
                    Text("Waiting for log entries…")
                        .font(.system(size: 11, design: .monospaced))
                        .foregroundStyle(.tertiary)
                        .frame(maxWidth: .infinity, alignment: .leading)
                } else {
                    VStack(alignment: .leading, spacing: 0) {
                        ForEach(Array(stream.lines.enumerated()), id: \.offset) { index, line in
                            Text(line)
                                .font(.system(size: 11, design: .monospaced))
                                .lineLimit(1)
                                .truncationMode(.tail)
                                .textSelection(.enabled)
                                .frame(maxWidth: .infinity, alignment: .leading)
                                .id(index)
                        }
                    }
                }
            }
            .onChange(of: stream.lines.count) {
                proxy.scrollTo(stream.lines.count - 1, anchor: .bottom)
            }
        }
    }
}
