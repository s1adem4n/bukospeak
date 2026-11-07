import SwiftUI

struct ChannelView: View {
    @Environment(Channel.self) private var channel
    @Environment(Settings.self) private var settings
    @Binding var showsSettings: Bool

    var body: some View {
        VStack(spacing: 0) {
            HStack(spacing: Space.gap) {
                Text(settings.host)
                    .font(.subheadline.weight(.medium))
                    .foregroundStyle(.secondary)
                    .lineLimit(1)
                Spacer()
                Button("Einstellungen", systemImage: "gearshape") { showsSettings = true }
                    .labelStyle(.iconOnly)
                    .buttonStyle(.glass)
                    .buttonBorderShape(.circle)
                    .controlSize(.large)
            }
            .padding(.horizontal, Space.gutter)
            StageView()
            ControlBar()
                .padding(.horizontal, Space.gutter)
                .padding(.bottom, Space.inline)
        }
        .background(Color(.systemGroupedBackground))
        .animation(Motion.spring, value: channel.phase)
        .animation(Motion.spring, value: channel.isHeld)
        .sheet(isPresented: $showsSettings) {
            SettingsView()
        }
    }
}
