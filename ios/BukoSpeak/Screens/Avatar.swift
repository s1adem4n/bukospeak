import SwiftUI

/// A member's initial in a gray circle, as Contacts shows someone without a photo.
struct Avatar: View {
    let name: String
    let size: CGFloat

    var body: some View {
        Text(name.prefix(1).uppercased())
            .font(.system(size: size * 0.42, weight: .semibold, design: .rounded))
            .foregroundStyle(.white)
            .frame(width: size, height: size)
            .background(
                LinearGradient(colors: [Color(.systemGray2), Color(.systemGray)], startPoint: .top, endPoint: .bottom),
                in: .circle
            )
    }
}
