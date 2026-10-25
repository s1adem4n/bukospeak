# BukoSpeak

Voice chat for a few friends: one channel, a join button, who's in and who's speaking. Low
latency, Opus at high bitrates, and an iOS app that keeps its microphone through phone calls.

## Layout

| Folder | What |
|---|---|
| `proto/` | The protocol: control messages, audio datagrams, certificate fingerprints |
| `server/` | The server, which relays each member's audio to everyone else |
| `core/` | The client side the apps share: the engine, joining and pinning the server, noise suppression, voice activation, Opus, jitter buffers and the mix |
| `audio/desktop/` | Plays a call on the desktop's audio devices, through cpal |
| `cli/` | `bukospeak`, a terminal client for trying the server on the desktop's devices |

One QUIC connection per member (TLS 1.3): control messages on its first stream, Opus frames as
datagrams. The server's certificate is self-signed; clients pin its fingerprint on the first
join, as SSH does with known hosts, and refuse to connect if it changes.

## Development

Tools come from [mise](https://mise.jdx.dev): `mise install`, then `mise tasks` lists the tasks.

```sh
mise run check                                                  # formatting, lints and tests, as CI runs them
BUKOSPEAK_ROOM_KEY=dev cargo run -p bukospeak-server            # a server on [::]:9988, its identity in data/
cargo run -p bukospeak-cli -- devices                         # the audio devices and their ids
cargo run -p bukospeak-cli -- join localhost:9988 --name first --room-key dev
```

On Linux, cpal needs ALSA's headers (`alsa-lib` on Arch, `libasound2-dev` on Debian).

## Running the server

```sh
podman run -d --name bukospeak -p 9988:9988/udp -v bukospeak:/data \
  -e BUKOSPEAK_ROOM_KEY=… ghcr.io/s1adem4n/bukospeak-server
```

It logs its certificate's fingerprint on start, which clients show on their first join.
