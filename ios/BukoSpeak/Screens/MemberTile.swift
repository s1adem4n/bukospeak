import SwiftUI

/// A member: their initial in a ring that lights up while they speak, and their name.
struct MemberTile: View {
    let member: Member
    let speaks: Bool
    /// Whether to mark them muted, as this member when muted.
    let isMuted: Bool

    var body: some View {
        VStack(spacing: 10) {
            Text(member.name.prefix(1).uppercased())
                .font(.title3.weight(.semibold))
                .foregroundStyle(Palette.text)
                .frame(width: Metrics.avatar, height: Metrics.avatar)
                .background(Palette.raised, in: .circle)
                .padding(3)
                .overlay {
                    Circle()
                        .strokeBorder(Palette.live, lineWidth: 2.5)
                        .shadow(color: Palette.live.opacity(0.5), radius: 8)
                        .opacity(speaks ? 1 : 0)
                }
                .overlay(alignment: .bottomTrailing) {
                    if isMuted {
                        Image(systemName: "mic.slash.fill")
                            .font(.system(size: 10, weight: .bold))
                            .foregroundStyle(.white)
                            .frame(width: 22, height: 22)
                            .background(Palette.danger, in: .circle)
                            .overlay { Circle().strokeBorder(Palette.surface, lineWidth: 2.5) }
                            .transition(.scale.combined(with: .opacity))
                    }
                }
                .animation(Motion.speaking, value: speaks)
                .animation(Motion.spring, value: isMuted)
            Text(member.name)
                .font(.footnote.weight(.medium))
                .foregroundStyle(Palette.text)
                .lineLimit(1)
        }
        .frame(maxWidth: .infinity)
        .padding(.vertical, 16)
        .padding(.horizontal, 8)
        .background(Palette.surface, in: .rect(cornerRadius: Metrics.tileRadius))
        .overlay {
            RoundedRectangle(cornerRadius: Metrics.tileRadius).strokeBorder(Palette.line)
        }
        .accessibilityElement(children: .ignore)
        .accessibilityLabel(member.name)
        .accessibilityValue(
            [speaks ? "spricht" : nil, isMuted ? "stumm" : nil].compactMap(\.self).joined(separator: ", ")
        )
    }
}
