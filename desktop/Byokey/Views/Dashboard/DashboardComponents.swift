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
            if !title.isEmpty {
                Text(title)
                    .font(.system(size: 10, weight: .bold))
                    .foregroundStyle(Color.accentColor.opacity(0.8))
                    .kerning(0.8)
            }

            content
        }
        .frame(maxWidth: .infinity, alignment: .leading)
        .padding(16)
        .background(.white.opacity(0.85), in: .rect(cornerRadius: 14))
        .shadow(color: .black.opacity(0.04), radius: 8, x: 0, y: 2)
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
