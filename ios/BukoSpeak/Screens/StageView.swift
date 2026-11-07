import SwiftUI

// The desktop's seats, by how many there are, as wide as the screen allows.
struct SeatLayout {
    let columns: Int
    let aspect: CGFloat
    let radius: CGFloat
    let avatar: Avatar.Size
    let gap: CGFloat
    let name: Font

    static let two = SeatLayout(
        columns: 2,
        aspect: 176 / 224,
        radius: 20,
        avatar: .xl,
        gap: 16,
        name: .subheadline.weight(.medium)
    )
    static let four = SeatLayout(
        columns: 2,
        aspect: 176 / 152,
        radius: 20,
        avatar: .lg,
        gap: 12,
        name: .subheadline.weight(.medium)
    )
    static let many = SeatLayout(
        columns: 3,
        aspect: 128 / 112,
        radius: 16,
        avatar: .md,
        gap: 8,
        name: .footnote.weight(.medium)
    )

    func size(across width: CGFloat) -> CGSize {
        let gaps = Space.gap * CGFloat(columns - 1)
        let tile = ((width - gaps) / CGFloat(columns)).rounded(.down)
        return CGSize(width: tile, height: (tile / aspect).rounded())
    }

    static func of(_ seats: Int) -> SeatLayout {
        switch seats {
        case ...2: .two
        case ...4: .four
        default: .many
        }
    }
}

struct StageView: View {
    @Environment(Channel.self) private var channel

    var body: some View {
        let seats = channel.seats
        let layout = SeatLayout.of(seats.count)
        let rows = stride(from: 0, to: seats.count, by: layout.columns).map {
            Array(seats[$0..<min($0 + layout.columns, seats.count)])
        }
        GeometryReader { space in
            let tile = layout.size(across: space.size.width - Space.gutter * 2)
            ScrollView {
                VStack(spacing: Space.gap) {
                    ForEach(rows, id: \.first?.id) { row in
                        HStack(spacing: Space.gap) {
                            ForEach(row) { seat in
                                SeatView(seat: seat, layout: layout)
                                    .frame(width: tile.width, height: tile.height)
                                    .transition(.arrival)
                            }
                            // The last row's tiles keep the width of those above it.
                            if seats.count > layout.columns {
                                ForEach(row.count..<layout.columns, id: \.self) { _ in
                                    Color.clear.frame(width: tile.width, height: tile.height)
                                }
                            }
                        }
                    }
                }
                .padding(Space.gutter)
                .frame(
                    maxWidth: .infinity,
                    minHeight: space.size.height,
                    alignment: seats.count <= 4 ? .center : .top
                )
            }
            .scrollBounceBehavior(.basedOnSize)
        }
        .opacity(channel.phase == .reconnecting || channel.isHeld ? Dim.reconnecting : 1)
        .animation(Motion.dim, value: channel.phase == .reconnecting || channel.isHeld)
        .animation(Motion.spring, value: seats.map(\.id))
    }
}

struct SeatView: View {
    let seat: Seat
    let layout: SeatLayout

    var body: some View {
        switch seat {
        case .member(let member, let speaks):
            MemberTile(member: member, speaks: speaks, layout: layout)
        case .you(let joining):
            YourSeat(joining: joining, layout: layout)
        }
    }
}

private struct MemberTile: View {
    let member: Member
    let speaks: Bool
    let layout: SeatLayout

    var body: some View {
        let shape = RoundedRectangle(cornerRadius: layout.radius, style: .continuous)
        VStack(spacing: layout.gap) {
            Avatar(name: member.name, size: layout.avatar, badge: badge)
            SeatName(text: member.name, layout: layout)
        }
        .frame(maxWidth: .infinity, maxHeight: .infinity)
        .background(Color(.secondarySystemGroupedBackground), in: shape)
        .overlay { shape.strokeBorder(.primary.opacity(0.06), lineWidth: Measure.hairline) }
        .shadow(color: .black.opacity(0.06), radius: 12, y: 4)
        // Over the hairline, so the thicker edge doesn't move what is inside.
        .overlay {
            shape.strokeBorder(.green, lineWidth: Measure.speakingEdge)
                .opacity(speaks ? 1 : 0)
                .animation(speaks ? Motion.speakOn : Motion.speakOff, value: speaks)
        }
        .opacity(member.reconnecting ? Dim.reconnecting : 1)
        .animation(Motion.dim, value: member.reconnecting)
        .accessibilityElement(children: .ignore)
        .accessibilityLabel(member.name)
        .accessibilityValue(
            [speaks ? "spricht" : nil, member.muted ? "stumm" : nil, member.reconnecting ? "verbindet neu" : nil]
                .compactMap(\.self).joined(separator: ", ")
        )
    }

    private var badge: Avatar.Badge? {
        if member.reconnecting { return .reconnecting }
        return member.muted ? .muted : nil
    }
}

private struct YourSeat: View {
    let joining: Bool
    let layout: SeatLayout
    @State private var breathes = false

    var body: some View {
        VStack(spacing: layout.gap) {
            Avatar(name: nil, size: layout.avatar)
            SeatName(text: "Du", layout: layout)
        }
        .opacity(joining && breathes ? 1 : Dim.waiting)
        .animation(joining ? Motion.waiting : Motion.spring, value: joining && breathes)
        .onAppear { breathes = true }
        .frame(maxWidth: .infinity, maxHeight: .infinity)
        .overlay {
            RoundedRectangle(cornerRadius: layout.radius, style: .continuous)
                .strokeBorder(.tertiary, style: StrokeStyle(lineWidth: Measure.hairline, dash: [4, 4]))
        }
        .accessibilityElement(children: .ignore)
        .accessibilityLabel(joining ? "Du trittst bei" : "Dein Platz")
    }
}

private struct SeatName: View {
    let text: String
    let layout: SeatLayout

    var body: some View {
        Text(text)
            .font(layout.name)
            .foregroundStyle(.secondary)
            .lineLimit(1)
            .padding(.horizontal, Space.gap)
    }
}

extension AnyTransition {
    // As a tile arrives on the desktop: it rises into place as it fades in.
    fileprivate static let arrival = AnyTransition.opacity.combined(with: .offset(y: 8))
        .animation(.timingCurve(0.22, 1, 0.36, 1, duration: 0.24))
}
