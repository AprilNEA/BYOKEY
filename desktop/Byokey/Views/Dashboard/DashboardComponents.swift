import SwiftUI

struct Card<Content: View>: View {
    let title: String
    /// When `true`, the card's background stretches to fill the available
    /// vertical space (e.g. in an HStack row where siblings are taller).
    /// Leave `false` for cards in a vertical stack — otherwise they'll
    /// consume all remaining space instead of using their intrinsic height.
    var fillHeight: Bool = false
    @ViewBuilder var content: Content

    init(
        _ title: String,
        fillHeight: Bool = false,
        @ViewBuilder content: () -> Content
    ) {
        self.title = title
        self.fillHeight = fillHeight
        self.content = content()
    }

    var body: some View {
        VStack(alignment: .leading, spacing: 10) {
            if !title.isEmpty {
                SectionLabel(title)
            }

            content
        }
        .frame(
            maxWidth: .infinity,
            maxHeight: fillHeight ? .infinity : nil,
            alignment: .topLeading
        )
        .padding(Spacing.lg)
        .cardSurface()
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
