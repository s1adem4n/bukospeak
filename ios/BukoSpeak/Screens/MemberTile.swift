import SwiftUI

/// A member: their initial, as Contacts shows it, in a ring that lights up while they speak, and
/// their name; faded while they reconnect.
struct MemberTile: View {
    let member: Member
    let speaks: Bool
    let isMuted: Bool

    var body: some View {
        VStack(spacing: 8) {
            Avatar(name: member.name, size: 72)
                .padding(4)
                .overlay {
                    Circle()
                        .strokeBorder(.green, lineWidth: 3)
                        .opacity(speaks ? 1 : 0)
                }
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
                .font(.subheadline)
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
