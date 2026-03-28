import SwiftUI

struct Card<Content: View>: View {
    let title: String
    @ViewBuilder var content: Content

    init(_ title: String, @ViewBuilder content: () -> Content) {
        self.title = title
        self.content = content()
    }

    var body: some View {
        VStack(alignment: .leading, spacing: 10) {
            Text(title)
                .font(.system(size: 10, weight: .semibold))
                .foregroundStyle(.secondary)
                .kerning(0.5)

            content
        }
        .frame(maxWidth: .infinity, alignment: .leading)
        .padding(16)
        .background(.background, in: .rect(cornerRadius: 10))
    }
}

struct HeroNumber: View {
    let value: UInt64

    var body: some View {
        Text("\(value)")
            .font(.system(size: 34, weight: .bold, design: .rounded))
            .monospacedDigit()
    }
}

func formatTokenParts(_ count: UInt64) -> (String, String) {
    switch count {
    case 0..<1_000:
        return ("\(count)", "")
    case 1_000..<1_000_000:
        return (String(format: "%.1f", Double(count) / 1_000), "K")
    default:
        return (String(format: "%.2f", Double(count) / 1_000_000), "M")
    }
}

func formatTokens(_ count: UInt64) -> String {
    let (num, unit) = formatTokenParts(count)
    return "\(num)\(unit)"
}
