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
| `maxine/` | NVIDIA Maxine's audio effects on RTX cards, through NVIDIA's runtime loaded while running; `measure`, an example that times an effect on the machine's card |
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
cargo run -F bukospeak-desktop/harness --bin bukospeak-ui -- list  # the desktop app's window, headless
```

`bukospeak-ui` draws the desktop app's window offscreen in a scene, a staged call and system, and
drives it by the names of its elements (`.mark(name)`): `snap call-muted --theme light`,
`snap settings --steps "click suppression"`, `film settings --steps "click suppression"` for the
frames of the motion a step starts, `marks lobby` for what there is to click, `gallery` for every
scene, light and dark. It draws motions finished and moves timers only as the script says, so a
picture is the same on every run; a film plays motions on the wall clock, as GPUI does.

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

WAV files in `/data/sounds`, named after what each tells (`joined`, `left`, `came`, `went`,
`muted`, `unmuted`, each `.wav`), replace the desktop app's own tones: the app installs them
from the server after it joins, and again whenever they changed there.

With `-e BUKOSPEAK_NGC_KEY=…`, an API key of [NGC](https://ngc.nvidia.com), NVIDIA's catalogue,
the server also offers NVIDIA's noise suppression to members with an RTX card under Windows: the
first time a card generation asks for it, the server downloads it from NGC (about 1.8 GB) into
`/data/downloads`, and members then download it from the server (about 1.7 GB).
