import SwiftUI

/// The app's colors, each for the dark and the light appearance. The Windows app keeps the same
/// values by hand.
enum Palette {
    static let background = Color(dark: 0x0A0B0D, light: 0xF4F5F7)
    /// Tiles, cards and grouped rows.
    static let surface = Color(dark: 0x16181C, light: 0xFFFFFF)
    /// Avatars and the secondary round buttons, raised from a surface.
    static let raised = Color(dark: 0x22252B, light: 0xE9EBEF)
    static let line = Color(dark: 0xFFFFFF, light: 0x000000, opacity: 0.07)
    static let text = Color(dark: 0xF2F3F5, light: 0x0B0C0E)
    static let secondaryText = Color(dark: 0x9AA0A8, light: 0x646A72)
    /// Text and icons on a primary button, whose fill is `text`.
    static let onPrimary = Color(dark: 0x0A0B0D, light: 0xFFFFFF)
    /// Who speaks.
    static let live = Color(dark: 0x4BE38A, light: 0x1FA85A)
    /// Muted, and hanging up.
    static let danger = Color(dark: 0xFF5A4E, light: 0xE5372B)
    static let warning = Color(dark: 0xFFB547, light: 0xC98200)
}

/// Spacing and corner radii.
enum Metrics {
    static let spacing: CGFloat = 12
    static let margin: CGFloat = 16
    static let tileRadius: CGFloat = 18
    static let avatar: CGFloat = 52
    static let roundButton: CGFloat = 56
    static let mainButton: CGFloat = 72
}

/// How things move.
enum Motion {
    /// Layout changes: members coming and going, phases changing.
    static let spring = Animation.spring(response: 0.35, dampingFraction: 0.85)
    /// The speaking ring, quick enough to follow speech.
    static let speaking = Animation.easeOut(duration: 0.15)
}

extension Color {
    /// A color that follows the appearance, from `0xRRGGBB` values.
    fileprivate init(dark: UInt32, light: UInt32, opacity: CGFloat = 1) {
        self.init(
            uiColor: UIColor { traits in
                let rgb = traits.userInterfaceStyle == .dark ? dark : light
                return UIColor(
                    red: CGFloat(rgb >> 16 & 0xFF) / 255,
                    green: CGFloat(rgb >> 8 & 0xFF) / 255,
                    blue: CGFloat(rgb & 0xFF) / 255,
                    alpha: opacity
                )
            }
        )
    }
}
