import SwiftUI

/// The channel: who is in it and who speaks, and the buttons to join, mute and leave.
struct ChannelView: View {
    @Environment(Call.self) private var call
    @State private var showsSettings = false

    var body: some View {
        NavigationStack {
            List {
                if case .failed(let message) = call.phase {
                    Section {
                        Text(message)
                            .foregroundStyle(.red)
                        if let server = call.changedServer {
                            Button("Trust the server's new certificate") { call.trustAnew(server) }
                        }
                    }
                }
                Section(status) {
                    ForEach(call.members) { member in
                        MemberRow(
                            member: member,
                            isYou: member.id == call.you,
                            speaks: call.speaking.contains(member.id)
                        )
                    }
                }
            }
            .navigationTitle("BukoSpeak")
            .toolbar {
                ToolbarItem(placement: .topBarTrailing) {
                    Button("Settings", systemImage: "gearshape") { showsSettings = true }
                }
            }
            .safeAreaInset(edge: .bottom) {
                controls
                    .padding()
            }
            .sheet(isPresented: $showsSettings) {
                SettingsView()
            }
        }
    }

    private var status: String {
        switch call.phase {
        case .idle, .failed: ""
        case .joining: "Joining…"
        case .reconnecting: "Reconnecting…"
        case .connected: call.isHeld ? "On hold" : "\(call.members.count) here"
        }
    }

    @ViewBuilder private var controls: some View {
        switch call.phase {
        case .idle, .failed:
            Button {
                Task { await call.join() }
            } label: {
                Label("Join", systemImage: "phone.fill")
                    .frame(maxWidth: .infinity)
            }
            .buttonStyle(.borderedProminent)
            .controlSize(.large)
            .disabled(!call.settings.isComplete)
        case .joining, .connected, .reconnecting:
            HStack {
                Button {
                    Task { await call.toggleMute() }
                } label: {
                    Label(call.isMuted ? "Unmute" : "Mute", systemImage: call.isMuted ? "mic.slash.fill" : "mic.fill")
                        .frame(maxWidth: .infinity)
                }
                .buttonStyle(.bordered)
                .disabled(call.phase == .joining)
                Button(role: .destructive) {
                    Task { await call.leave() }
                } label: {
                    Label("Leave", systemImage: "phone.down.fill")
                        .frame(maxWidth: .infinity)
                }
                .buttonStyle(.borderedProminent)
            }
            .controlSize(.large)
        }
    }
}

/// A member, with a dot that lights up while they speak.
private struct MemberRow: View {
    let member: Member
    let isYou: Bool
    let speaks: Bool

    var body: some View {
        HStack(spacing: 12) {
            Circle()
                .fill(speaks ? Color.green : Color.secondary.opacity(0.25))
                .frame(width: 10, height: 10)
                .animation(.easeOut(duration: 0.15), value: speaks)
            Text(member.name)
            if isYou {
                Text("you")
                    .foregroundStyle(.secondary)
            }
        }
        .accessibilityElement(children: .combine)
        .accessibilityValue(speaks ? "speaking" : "")
    }
}
