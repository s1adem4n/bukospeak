import SwiftUI

/// A member: their avatar in a ring that lights up and lifts while they speak, and their name;
/// faded while they reconnect.
struct MemberTile: View {
    let member: Member
    let speaks: Bool
    let isMuted: Bool

    private static let avatar: CGFloat = 72
    /// The space between the avatar and its ring, and the ring's width.
    private static let gap: CGFloat = 4
    private static let ring: CGFloat = 3

    var body: some View {
        VStack(spacing: 10) {
            Avatar(name: member.name, size: Self.avatar)
                .padding(Self.gap + Self.ring)
                .overlay {
                    Circle()
                        .strokeBorder(.green, lineWidth: Self.ring)
                        .shadow(color: .green.opacity(0.6), radius: 6)
                        .opacity(speaks ? 1 : 0)
                }
                .scaleEffect(speaks ? 1.04 : 1)
                .overlay(alignment: .bottomTrailing) {
                    if isMuted {
                        Image(systemName: "mic.slash.fill")
                            .font(.caption2.bold())
                            .foregroundStyle(.white)
                            .frame(width: 24, height: 24)
                            .background(.red, in: .circle)
                            .overlay { Circle().strokeBorder(Color(.systemBackground), lineWidth: 2.5) }
                            .transition(.scale.combined(with: .opacity))
                    }
                }
                .animation(Motion.speaking, value: speaks)
                .animation(Motion.spring, value: isMuted)
            Text(member.name)
                .font(.subheadline.weight(.medium))
                .lineLimit(1)
        }
        .opacity(member.reconnecting ? 0.4 : 1)
        .animation(Motion.spring, value: member.reconnecting)
        .accessibilityElement(children: .ignore)
        .accessibilityLabel(member.name)
        .accessibilityValue(
            [speaks ? "spricht" : nil, isMuted ? "stumm" : nil, member.reconnecting ? "verbindet neu" : nil]
                .compactMap(\.self).joined(separator: ", ")
        )
    }
}
