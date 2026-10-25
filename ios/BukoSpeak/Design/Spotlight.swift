import SwiftUI

/// The light the logo stands in on the start screen: a soft cone from above, pulsing while the
/// call joins.
struct Spotlight: View {
    let isJoining: Bool

    var body: some View {
        EllipticalGradient(
            colors: [.white.opacity(0.18), .white.opacity(0.04), .clear],
            center: .top,
            endRadiusFraction: 0.8
        )
        .phaseAnimator([1, 0.45]) { light, brightness in
            light.opacity(isJoining ? brightness : 0.7)
        } animation: { _ in
            .easeInOut(duration: 0.8)
        }
        .ignoresSafeArea()
        .accessibilityHidden(true)
    }
}
