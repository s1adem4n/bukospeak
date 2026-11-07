import SwiftUI

struct Avatar: View {
    struct Size {
        let diameter: CGFloat
        let initial: CGFloat
        let badge: CGFloat

        static let md = Size(diameter: 48, initial: 20, badge: 20)
        static let lg = Size(diameter: 64, initial: 24, badge: 24)
        static let xl = Size(diameter: 80, initial: 32, badge: 28)
    }

    enum Badge {
        case muted
        case reconnecting
    }

    let name: String?
    let size: Size
    var badge: Badge?
    @Environment(\.colorScheme) private var colorScheme

    private static let badgeEdge: CGFloat = 2

    var body: some View {
        let (background, foreground) = colors
        Group {
            if let initial = name?.first {
                Text(initial.uppercased())
                    .font(.system(size: size.initial, weight: .semibold))
            } else {
                Image(systemName: "person")
                    .font(.system(size: size.initial, weight: .regular))
            }
        }
        .foregroundStyle(foreground)
        .frame(width: size.diameter, height: size.diameter)
        .background(background, in: .circle)
        .overlay { Circle().strokeBorder(.primary.opacity(0.08), lineWidth: Measure.hairline) }
        .overlay(alignment: .bottomTrailing) {
            if let badge {
                BadgeView(badge: badge, size: size.badge, edge: Self.badgeEdge)
                    .offset(x: Self.badgeEdge, y: Self.badgeEdge)
                    .transition(.scale.combined(with: .opacity))
            }
        }
        .animation(Motion.spring, value: badge)
    }

    // As the desktop app picks them, so a name gets the same colors in both.
    private var colors: (Color, Color) {
        guard let name, !name.isEmpty else { return (Color(.tertiarySystemFill), Color(.secondaryLabel)) }
        let hue = Self.hues[Int(Self.hash(name) % UInt32(Self.hues.count))] / 360
        return colorScheme == .dark
            ? (Color(hue: hue, saturation: 0.3, lightness: 0.24), Color(hue: hue, saturation: 0.7, lightness: 0.82))
            : (Color(hue: hue, saturation: 0.65, lightness: 0.92), Color(hue: hue, saturation: 0.5, lightness: 0.36))
    }

    private static let hues: [Double] = [212, 258, 328, 18, 40, 152, 184, 352]

    private static func hash(_ name: String) -> UInt32 {
        name.utf8.reduce(0x811c_9dc5) { hash, byte in (hash ^ UInt32(byte)) &* 0x0100_0193 }
    }
}

private struct BadgeView: View {
    let badge: Avatar.Badge
    let size: CGFloat
    let edge: CGFloat

    var body: some View {
        Image(systemName: badge == .muted ? "mic.slash.fill" : "arrow.trianglehead.2.clockwise")
            .font(.system(size: size * 0.5, weight: .semibold))
            .foregroundStyle(badge == .muted ? AnyShapeStyle(.red) : AnyShapeStyle(.secondary))
            .frame(width: size, height: size)
            .padding(edge)
            .background(Color(.secondarySystemGroupedBackground), in: .circle)
    }
}

extension Color {
    fileprivate init(hue: Double, saturation: Double, lightness: Double) {
        let brightness = lightness + saturation * min(lightness, 1 - lightness)
        self.init(hue: hue, saturation: brightness == 0 ? 0 : 2 * (1 - lightness / brightness), brightness: brightness)
    }
}
