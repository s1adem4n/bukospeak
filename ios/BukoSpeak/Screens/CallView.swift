import SwiftUI

/// The call: who is in it and who speaks, and beneath them the controls; faded, with why, while
/// nobody hears it.
struct CallView: View {
    @Environment(Call.self) private var call
    /// Where the start screen's logo becomes the microphone button.
    let stage: Namespace.ID

    var body: some View {
        ScrollView {
            LazyVGrid(columns: [GridItem(.adaptive(minimum: 96), spacing: 16)], spacing: 24) {
                ForEach(Array(call.members.enumerated()), id: \.element.id) { index, member in
                    MemberTile(
                        member: member,
                        speaks: call.speaking.contains(member.id),
                        // Your own as you set it, before the server tells it back.
                        isMuted: member.id == call.you ? call.isMuted : member.muted
                    )
                    // One after the other on entering, as the logo lands.
                    .transition(
                        .scale(scale: 0.6).combined(with: .opacity)
                            .animation(Motion.spring.delay(0.1 + Double(index) * 0.05))
                    )
                }
            }
            .padding()
        }
        .opacity(isDimmed ? 0.4 : 1)
        .overlay {
            if isConnecting {
                ProgressView()
                    .controlSize(.large)
                    .transition(.opacity)
            }
        }
        .safeAreaInset(edge: .bottom) {
            ControlBar(stage: stage)
        }
    }

    /// The channel recedes while nobody hears it; joining shows who is there in full.
    private var isDimmed: Bool {
        call.phase == .reconnecting || call.isHeld
    }

    private var isConnecting: Bool {
        call.phase == .joining || call.phase == .reconnecting
    }
}

/// How the call is, in the navigation bar's middle: a dot in the state's color, the state, and
/// once it is up how long.
struct CallStatus: View {
    @Environment(Call.self) private var call

    var body: some View {
        let (label, color) = state
        HStack(spacing: 8) {
            Circle()
                .fill(color)
                .frame(width: 8, height: 8)
                .shadow(color: color.opacity(0.6), radius: 3)
            Text(label)
                .fontWeight(.semibold)
            if let since = call.since, call.phase == .connected, !call.isHeld {
                Text(timerInterval: since...Date.distantFuture, countsDown: false)
                    .monospacedDigit()
                    .foregroundStyle(.secondary)
            }
        }
        .font(.subheadline)
        .accessibilityElement(children: .combine)
    }

    private var state: (String, Color) {
        switch call.phase {
        case .connected where call.isHeld: ("Gehalten", .secondary)
        case .connected: ("Verbunden", .green)
        case .reconnecting: ("Verbinde neu…", .orange)
        case .joining, .idle, .failed: ("Verbinde…", .secondary)
        }
    }
}

/// Where the call plays, the microphone and hanging up: three circles of glass alike in one bar.
struct ControlBar: View {
    @Environment(Call.self) private var call
    /// Where the start screen's logo becomes the microphone button.
    let stage: Namespace.ID

    var body: some View {
        GlassEffectContainer(spacing: 20) {
            HStack(spacing: 20) {
                RoutePicker()
                    .frame(width: GlassCircle.size, height: GlassCircle.size)
                    .glassEffect(.regular.interactive(), in: .circle)
                Button {
                    Task { await call.toggleMute() }
                } label: {
                    Image(systemName: call.isMuted ? "mic.slash.fill" : "mic.fill")
                        .contentTransition(.symbolEffect(.replace))
                }
                // Red while muted.
                .buttonStyle(GlassCircle(tint: call.isMuted ? .red : nil))
                .matchedGeometryEffect(id: "microphone", in: stage)
                // The microphone is off while another call holds this one.
                .disabled(call.isHeld)
                .accessibilityLabel(call.isMuted ? "Stummschaltung aufheben" : "Stummschalten")
                Button {
                    Task { await call.leave() }
                } label: {
                    Image(systemName: "phone.down.fill")
                }
                .buttonStyle(GlassCircle(tint: .red))
                .accessibilityLabel("Verlassen")
            }
        }
        .padding(.bottom, 8)
    }
}

/// A control bar's button: its symbol in a circle of glass as large as the route picker's,
/// tinted for what wants attention.
private struct GlassCircle: ButtonStyle {
    static let size: CGFloat = 64

    let tint: Color?
    @Environment(\.isEnabled) private var isEnabled

    func makeBody(configuration: Configuration) -> some View {
        configuration.label
            .font(.title2)
            .foregroundStyle(tint == nil ? AnyShapeStyle(.primary) : AnyShapeStyle(.white))
            .frame(width: Self.size, height: Self.size)
            // The whole circle takes the tap, not only the symbol's strokes: the glass beneath
            // isn't hit.
            .contentShape(.circle)
            .glassEffect(.regular.tint(tint).interactive(), in: .circle)
            .opacity(isEnabled ? 1 : 0.4)
    }
}
