import SwiftUI

/// The logo, large: a silver stage microphone.
struct BrandMark: View {
    var body: some View {
        Image(.brandMark)
            .resizable()
            .scaledToFit()
            .frame(height: 200)
            .shadow(color: .black.opacity(0.3), radius: 16, y: 8)
            .accessibilityHidden(true)
    }
}
