# BukoSpeak

Voice chat for a few friends: one channel, a join button, who's in and who's speaking. Low
latency, Opus at high bitrates, and an iOS app that keeps its microphone through phone calls.

## Layout

| Folder | What |
|---|---|
| `proto/` | The protocol: control messages, audio datagrams, certificate fingerprints |
| `server/` | The server, which relays each member's audio to everyone else |
| `core/` | The client side the apps share: the engine, joining and pinning the server and joining again after a lost connection, the channel as the member sees it and what to announce about it, noise suppression, gain control, echo cancellation, voice activation, Opus, jitter buffers and the mix |
| `ios/` | The iOS app (SwiftUI, CallKit) and `ios/dev`, which builds it on a Mac |
| `ios/ffi/` | The C interface the iOS app calls the engine through, and its header; plays the call on the iPhone's microphone and speaker, through VoiceProcessingIO |
| `desktop/` | The desktop app for Windows, macOS and Linux (GPUI), and `bukospeak`, a terminal client for trying the server on the desktop's devices; both play the call through cpal |

One QUIC connection per member (TLS 1.3): control messages on its first stream, Opus frames as
datagrams. The server's certificate is self-signed; clients pin its fingerprint on the first
join, as SSH does with known hosts, and refuse to connect if it changes.

## Development

Tools come from [mise](https://mise.jdx.dev): `mise install`, then `mise tasks` lists the tasks.

```sh
mise run check                                                  # formatting, lints and tests, as CI runs them
BUKOSPEAK_ROOM_KEY=dev cargo run -p bukospeak-server            # a server on [::]:9988, its identity in data/
cargo run --bin bukospeak-desktop                               # the desktop app
cargo run --bin bukospeak -- devices                            # the audio devices and their ids
cargo run --bin bukospeak -- join localhost:9988 --name first --room-key dev
```

On Linux, cpal needs ALSA's headers (`alsa-lib` on Arch, `libasound2-dev` on Debian), and the
desktop app those of fontconfig, freetype, wayland and xkbcommon (CI's Linux job lists Debian's).
`mise run header` rewrites the C header after the `ios/ffi` crate's interface changed.

The iOS app builds on a Mac over ssh: `ios/dev build` syncs the repository there, builds the
Rust library for the iPhone and then the app; `ios/dev ota` prints a tailnet-only page that
installs it. `ios/dev help` lists the commands.

## Releasing the desktop app

The workspace's `version` raised, committed, and tagged with it (`v0.2.0`): pushing the tag runs
`release.yml`, which makes a GitHub Release of
- `BukoSpeak-win-Setup.exe`, an installer whose app downloads newer releases in the background
  and applies them on its next start; it is unsigned, so SmartScreen warns on the first install,
- and archives of the app and `bukospeak` for Windows (x86_64), macOS (Apple Silicon) and Linux
  (x86_64). The macOS binaries are unsigned: `xattr -d com.apple.quarantine` lets them start.

## Running the server

```sh
podman run -d --name bukospeak -p 9988:9988/udp -v bukospeak:/data \
  -e BUKOSPEAK_ROOM_KEY=… ghcr.io/s1adem4n/bukospeak-server
```

It logs its certificate's fingerprint on start, which clients show on their first join.
