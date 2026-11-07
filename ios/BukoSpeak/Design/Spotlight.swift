import SwiftUI

struct Spotlight: View {
    var body: some View {
        EllipticalGradient(
            colors: [.white.opacity(0.18), .white.opacity(0.04), .clear],
            center: .top,
            endRadiusFraction: 0.8
        )
        .opacity(0.7)
        .ignoresSafeArea()
        .accessibilityHidden(true)
    }
}
