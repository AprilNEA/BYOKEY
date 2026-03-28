import SwiftUI

enum AnsiParser {
    /// Parses ANSI SGR escape codes and returns a styled `AttributedString`.
    static func parse(_ raw: String) -> AttributedString {
        var result = AttributedString()
        var fg: Color?
        var dim = false
        var idx = raw.startIndex

        while idx < raw.endIndex {
            if raw[idx] == "\u{1B}", let sgr = parseSGR(raw, from: idx) {
                applyCodes(sgr.codes, fg: &fg, dim: &dim)
                idx = sgr.end
                continue
            }

            let start = idx
            idx = raw.index(after: idx)
            while idx < raw.endIndex, raw[idx] != "\u{1B}" {
                idx = raw.index(after: idx)
            }

            var seg = AttributedString(raw[start..<idx])
            if let c = fg {
                seg.foregroundColor = dim ? c.opacity(0.5) : c
            } else if dim {
                seg.foregroundColor = .secondary
            }
            result.append(seg)
        }

        return result
    }

    /// Strips all ANSI escape sequences, returning plain text.
    static func strip(_ raw: String) -> String {
        raw.replacing(/\u{1B}\[[0-9;]*m/, with: "")
    }

    // MARK: - Private

    private static func parseSGR(
        _ s: String, from start: String.Index
    ) -> (codes: [Int], end: String.Index)? {
        var i = s.index(after: start)
        guard i < s.endIndex, s[i] == "[" else { return nil }
        i = s.index(after: i)

        var params = ""
        while i < s.endIndex {
            let c = s[i]
            if c == "m" {
                let codes = params.isEmpty
                    ? [0]
                    : params.split(separator: ";").compactMap { Int($0) }
                return (codes, s.index(after: i))
            }
            guard c.isNumber || c == ";" else { return nil }
            params.append(c)
            i = s.index(after: i)
        }
        return nil
    }

    private static func applyCodes(_ codes: [Int], fg: inout Color?, dim: inout Bool) {
        for code in codes {
            switch code {
            case 0:       fg = nil; dim = false
            case 2:       dim = true
            case 22:      dim = false
            case 31, 91:  fg = .red
            case 32, 92:  fg = .green
            case 33, 93:  fg = .yellow
            case 34, 94:  fg = .blue
            case 35, 95:  fg = .purple
            case 36, 96:  fg = .cyan
            case 39:      fg = nil
            default:      break
            }
        }
    }
}
