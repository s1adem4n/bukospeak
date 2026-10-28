import SwiftUI

/// The logo: a silver stage microphone, `height` high.
struct BrandMark: View {
    let height: CGFloat

    var body: some View {
        Image(.brandMark)
            .resizable()
            .scaledToFit()
            .frame(height: height)
            .shadow(color: .black.opacity(0.3), radius: height / 12, y: height / 25)
            .accessibilityHidden(true)
    }
}
