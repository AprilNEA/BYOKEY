import SwiftUI

// MARK: - Spacing Scale

enum Spacing {
    static let xs: CGFloat = 4
    static let sm: CGFloat = 8
    static let md: CGFloat = 12
    static let lg: CGFloat = 16
    static let xl: CGFloat = 24
    static let xxl: CGFloat = 32
}

// MARK: - Adaptive Colors

extension NSColor {
    /// Card and elevated surface backgrounds.
    static let surfacePrimary = NSColor(name: nil) { appearance in
        if appearance.bestMatch(from: [.darkAqua, .aqua]) == .darkAqua {
            return .white.withAlphaComponent(0.07)
        }
        return .white.withAlphaComponent(0.85)
    }

    /// Hovered card surface.
    static let surfaceHovered = NSColor(name: nil) { appearance in
        if appearance.bestMatch(from: [.darkAqua, .aqua]) == .darkAqua {
            return .white.withAlphaComponent(0.10)
        }
        return .white.withAlphaComponent(0.92)
    }

    /// Subtle recessed surface (search fields, picker backgrounds).
    static let surfaceSecondary = NSColor(name: nil) { appearance in
        if appearance.bestMatch(from: [.darkAqua, .aqua]) == .darkAqua {
            return .white.withAlphaComponent(0.05)
        }
        return NSColor(white: 0.5, alpha: 0.08)
    }
}

extension Color {
    static let surfacePrimary = Color(nsColor: .surfacePrimary)
    static let surfaceHovered = Color(nsColor: .surfaceHovered)
    static let surfaceSecondary = Color(nsColor: .surfaceSecondary)
}

// MARK: - Canvas Background

/// Adaptive gradient behind all content. Light: cool lavender. Dark: subtle charcoal.
struct CanvasBackground: View {
    @Environment(\.colorScheme) private var colorScheme

    var body: some View {
        LinearGradient(
            colors: colorScheme == .dark ? darkStops : lightStops,
            startPoint: .topLeading,
            endPoint: .bottomTrailing
        )
        .ignoresSafeArea()
    }

    private var lightStops: [Color] {
        [
            Color(red: 0.98, green: 0.96, blue: 0.98),
            Color(red: 0.95, green: 0.96, blue: 1.0),
            Color(red: 0.93, green: 0.95, blue: 1.0),
        ]
    }

    private var darkStops: [Color] {
        [
            Color(red: 0.11, green: 0.10, blue: 0.14),
            Color(red: 0.09, green: 0.10, blue: 0.13),
            Color(red: 0.08, green: 0.09, blue: 0.12),
        ]
    }
}

// MARK: - Card Surface Modifier

/// Applies adaptive card background, dark-mode stroke, and shadow.
struct CardSurface: ViewModifier {
    var cornerRadius: CGFloat = 14
    var isHovered: Bool = false
    @Environment(\.colorScheme) private var colorScheme

    func body(content: Content) -> some View {
        content
            .background(
                isHovered ? Color.surfaceHovered : Color.surfacePrimary,
                in: .rect(cornerRadius: cornerRadius)
            )
            .overlay {
                if colorScheme == .dark {
                    RoundedRectangle(cornerRadius: cornerRadius)
                        .strokeBorder(.white.opacity(0.06), lineWidth: 0.5)
                }
            }
            .shadow(
                color: .black.opacity(shadowOpacity),
                radius: isHovered ? 12 : 8,
                x: 0,
                y: 2
            )
    }

    private var shadowOpacity: Double {
        colorScheme == .dark
            ? (isHovered ? 0.4 : 0.3)
            : (isHovered ? 0.07 : 0.04)
    }
}

extension View {
    func cardSurface(cornerRadius: CGFloat = 14, isHovered: Bool = false) -> some View {
        modifier(CardSurface(cornerRadius: cornerRadius, isHovered: isHovered))
    }
}

// MARK: - Section Label

/// Uppercase accent-tinted section header.
struct SectionLabel: View {
    let text: String
    var subtitle: String?

    init(_ text: String, subtitle: String? = nil) {
        self.text = text
        self.subtitle = subtitle
    }

    var body: some View {
        VStack(alignment: .leading, spacing: 2) {
            Text(text)
                .font(.system(size: 10, weight: .bold))
                .foregroundStyle(Color.accentColor.opacity(0.8))
                .kerning(0.8)
            if let subtitle {
                Text(subtitle)
                    .font(.caption)
                    .foregroundStyle(.tertiary)
            }
        }
    }
}

// MARK: - Status Dot

/// Colored circle for server/account state indicators.
struct StatusDot: View {
    enum Status {
        case active, warning, error, inactive

        var color: Color {
            switch self {
            case .active: .green
            case .warning: .orange
            case .error: .red
            case .inactive: .gray
            }
        }
    }

    let status: Status
    var size: CGFloat = 8

    init(_ status: Status, size: CGFloat = 8) {
        self.status = status
        self.size = size
    }

    var body: some View {
        Circle()
            .fill(status.color)
            .frame(width: size, height: size)
            .accessibilityLabel(accessibilityText)
    }

    private var accessibilityText: String {
        switch status {
        case .active: "Active"
        case .warning: "Warning"
        case .error: "Error"
        case .inactive: "Inactive"
        }
    }
}

// MARK: - Server Starting Placeholder

/// Shared "Waiting for server…" loading state.
struct ServerStartingView: View {
    var body: some View {
        VStack(spacing: Spacing.sm) {
            ProgressView().controlSize(.large)
            Text("Waiting for server…")
                .foregroundStyle(.secondary)
        }
        .frame(maxWidth: .infinity, maxHeight: .infinity)
    }
}
