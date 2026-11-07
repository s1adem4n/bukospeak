import SwiftUI

struct ControlBar: View {
    @Environment(Channel.self) private var channel
    @Environment(\.call) private var call

    var body: some View {
        VStack(spacing: Space.gap) {
            StatusLine()
            GlassEffectContainer(spacing: Space.gap) {
                HStack(spacing: Space.gap) {
                    if channel.isInCall {
                        Button(action: call.toggleMute) {
                            Label(
                                channel.isMuted ? "Stumm" : "Stummschalten",
                                systemImage: channel.isMuted ? "mic.slash.fill" : "mic.fill"
                            )
                            .contentTransition(.symbolEffect(.replace))
                        }
                        .buttonStyle(ControlStyle(kind: channel.isMuted ? .off : .raised, fills: true))
                        .disabled(channel.isHeld)
                        .accessibilityLabel(channel.isMuted ? "Stummschaltung aufheben" : "Stummschalten")
                        RoutePicker()
                            .frame(width: Measure.routePicker, height: Measure.routePicker)
                            .frame(width: Measure.control, height: Measure.control)
                            .glassEffect(.regular.interactive(), in: .circle)
                        Button("Verlassen", systemImage: "phone.down.fill", action: call.leave)
                            .labelStyle(.iconOnly)
                            .buttonStyle(ControlStyle(kind: .danger))
                    } else {
                        Button(action: call.join) {
                            if channel.phase == .joining {
                                ProgressView()
                                    .tint(Color(.systemBackground))
                            } else {
                                Text(channel.failure == nil ? "Beitreten" : "Erneut beitreten")
                            }
                        }
                        .buttonStyle(ControlStyle(kind: .primary, fills: true))
                    }
                }
            }
        }
    }
}

private struct StatusLine: View {
    @Environment(Channel.self) private var channel
    // The line keeps its height without a status, so the buttons stay in place.
    @ScaledMetric(relativeTo: .footnote) private var line: CGFloat = 18

    var body: some View {
        HStack(spacing: Space.inline) {
            if let status {
                Dot(color: status.dot)
                Text(status.text)
                    .fontWeight(.medium)
                    .foregroundStyle(status.ink)
                    .lineLimit(1)
                Spacer(minLength: Space.gap)
                if let since = channel.since, channel.isInCall, !channel.isHeld {
                    Text(timerInterval: since...Date.distantFuture, countsDown: false)
                        .monospacedDigit()
                        .foregroundStyle(.tertiary)
                }
            }
        }
        .font(.footnote)
        .frame(maxWidth: .infinity, minHeight: line, alignment: .leading)
        .accessibilityElement(children: .combine)
    }

    private struct Status {
        let text: String
        let dot: Color
        let ink: Color
    }

    private var status: Status? {
        switch channel.phase {
        case .failed(let failure): Status(text: failure.text, dot: .red, ink: .red)
        case .idle: nil
        case .joining: Status(text: "Verbinde…", dot: Color(.tertiaryLabel), ink: .secondary)
        case .connected where channel.isHeld: Status(text: "Gehalten", dot: Color(.tertiaryLabel), ink: .secondary)
        case .connected: Status(text: "Verbunden", dot: .green, ink: .secondary)
        case .reconnecting: Status(text: "Verbinde neu…", dot: .orange, ink: .primary)
        }
    }
}

private struct Dot: View {
    let color: Color

    var body: some View {
        Circle()
            .fill(color)
            .frame(width: Measure.dot, height: Measure.dot)
            .padding(Measure.dotHalo)
            .background(color.opacity(0.16), in: .circle)
    }
}

struct ControlStyle: ButtonStyle {
    enum Kind {
        case primary
        case raised
        case off
        case danger
    }

    let kind: Kind
    var fills = false
    @Environment(\.isEnabled) private var isEnabled

    func makeBody(configuration: Configuration) -> some View {
        configuration.label
            .font(.body.weight(.semibold))
            .foregroundStyle(foreground)
            .padding(.horizontal, fills ? Space.gutter : 0)
            .frame(minWidth: Measure.control, maxWidth: fills ? .infinity : nil, minHeight: Measure.control)
            // The glass beneath isn't hit, so the whole shape takes the tap.
            .contentShape(.capsule)
            .glassEffect(glass.interactive(), in: .capsule)
            .opacity(isEnabled ? 1 : 0.5)
    }

    private var foreground: Color {
        switch kind {
        case .primary: Color(.systemBackground)
        case .raised: .primary
        case .off, .danger: .white
        }
    }

    private var glass: Glass {
        switch kind {
        case .primary: .regular.tint(.primary)
        case .raised: .regular
        case .off, .danger: .regular.tint(.red)
        }
    }
}
