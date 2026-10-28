import SwiftUI

/// A member's initial on a color of their own, the same their name gets in the desktop app; a
/// silhouette without a name.
struct Avatar: View {
    let name: String
    let size: CGFloat
    @Environment(\.colorScheme) private var colorScheme

    var body: some View {
        let (background, foreground) = colors
        Group {
            if let initial = name.first {
                Text(initial.uppercased())
            } else {
                Image(systemName: "person.fill")
            }
        }
        .font(.system(size: size * 0.42, weight: .semibold, design: .rounded))
        .foregroundStyle(foreground)
        .frame(width: size, height: size)
        .background(background, in: .circle)
        .overlay { Circle().strokeBorder(.primary.opacity(0.08), lineWidth: 1) }
    }

    /// The background and the initial's color: a hue from the name, deep in the dark and pale in
    /// the light, as the desktop app picks them.
    private var colors: (Color, Color) {
        guard !name.isEmpty else { return (Color(.systemGray4), Color(.systemGray)) }
        let hue = Self.hues[Int(Self.hash(name) % UInt32(Self.hues.count))] / 360
        return colorScheme == .dark
            ? (Color(hue: hue, saturation: 0.3, lightness: 0.24), Color(hue: hue, saturation: 0.7, lightness: 0.82))
            : (Color(hue: hue, saturation: 0.65, lightness: 0.92), Color(hue: hue, saturation: 0.5, lightness: 0.36))
    }

    private static let hues: [Double] = [212, 258, 328, 18, 40, 152, 184, 352]

    /// FNV-1a of the name's UTF-8: the desktop app's hash, so a name gets the same hue in both.
    private static func hash(_ name: String) -> UInt32 {
        name.utf8.reduce(0x811c_9dc5) { hash, byte in (hash ^ UInt32(byte)) &* 0x0100_0193 }
    }
}

extension Color {
    /// A color by hue, saturation and lightness, as the desktop app's colors are given.
    fileprivate init(hue: Double, saturation: Double, lightness: Double) {
        let brightness = lightness + saturation * min(lightness, 1 - lightness)
        self.init(hue: hue, saturation: brightness == 0 ? 0 : 2 * (1 - lightness / brightness), brightness: brightness)
    }
}
