import SwiftUI

/// A member's initial in a gray circle, as Contacts shows someone without a photo; a silhouette
/// without a name.
struct Avatar: View {
    let name: String
    let size: CGFloat

    var body: some View {
        Group {
            if let initial = name.first {
                Text(initial.uppercased())
            } else {
                Image(systemName: "person.fill")
            }
        }
        .font(.system(size: size * 0.42, weight: .semibold, design: .rounded))
        .foregroundStyle(.white)
        .frame(width: size, height: size)
        .background(
            LinearGradient(colors: [Color(.systemGray2), Color(.systemGray)], startPoint: .top, endPoint: .bottom),
            in: .circle
        )
    }
}
