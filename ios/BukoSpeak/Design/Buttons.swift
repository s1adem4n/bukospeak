import SwiftUI

/// The one wide button of a screen, as Beitreten.
struct PrimaryButtonStyle: ButtonStyle {
    @Environment(\.isEnabled) private var isEnabled

    func makeBody(configuration: Configuration) -> some View {
        configuration.label
            .font(.headline)
            .foregroundStyle(Palette.onPrimary)
            .frame(maxWidth: .infinity, minHeight: 56)
            .background(Palette.text, in: .capsule)
            .opacity(isEnabled ? 1 : 0.4)
            .scaleEffect(configuration.isPressed ? 0.97 : 1)
            .animation(Motion.spring, value: configuration.isPressed)
    }
}

/// A round button of the call's controls, its icon the label.
struct RoundButtonStyle: ButtonStyle {
    enum Kind {
        /// The microphone: the largest, inverted while it is on.
        case main
        case secondary
        /// Hanging up.
        case destructive
    }

    let kind: Kind
    /// Whether the main button shows its off state: dark, with a red ring.
    var isOff = false

    func makeBody(configuration: Configuration) -> some View {
        let size = kind == .main ? Metrics.mainButton : Metrics.roundButton
        configuration.label
            .font(.system(size: size * 0.36, weight: .semibold))
            .foregroundStyle(foreground)
            .frame(width: size, height: size)
            .background(background, in: .circle)
            .overlay {
                if isOff {
                    Circle().strokeBorder(Palette.danger, lineWidth: 2)
                }
            }
            .scaleEffect(configuration.isPressed ? 0.92 : 1)
            .animation(Motion.spring, value: configuration.isPressed)
    }

    private var foreground: Color {
        switch kind {
        case .main: isOff ? Palette.danger : Palette.onPrimary
        case .secondary: Palette.text
        case .destructive: .white
        }
    }

    private var background: Color {
        switch kind {
        case .main: isOff ? Palette.raised : Palette.text
        case .secondary: Palette.raised
        case .destructive: Palette.danger
        }
    }
}
