# Architecture

## Network

One QUIC connection per member, TLS 1.3 with the server's self-signed certificate pinned by its
fingerprint on the first join. Control messages (JSON) travel on the connection's first stream,
audio as datagrams. The server relays each member's datagrams to everyone else and never decodes
them.

## Audio format

48 kHz mono, `f32` internally, in 10 ms frames of 480 samples: RNNoise's frame size and Opus'
shortest useful frame, so no stage rebuffers.

Opus: VOIP application, 48 kbit/s, in-band FEC, 10 ms frames. At this bitrate libopus stays in
hybrid mode, where in-band FEC exists (CELT-only frames carry none). A test checks the mode in
each packet's TOC byte.

A datagram is a 7-byte header and the Opus packet:

| Bytes | Field |
|---|---|
| 0–1 | sender's id, stamped by the server |
| 2–5 | frame index on the sender's 10 ms timeline: it counts every encoded frame, sent or not, so it is a timestamp. It keeps counting while the audio engine rebuilds its streams, since the capture outlives them; a receiver that sees it start over treats it as a new timeline |
| 6 | bit 7: last frame of a talkspurt; bits 0–6: the frame's level in −dBov, as in RFC 6464 |

## Threads

```
capture callback (realtime)  10 ms frames → [HPF, AEC] → RNNoise → [AGC] → gate → Opus → ring → wakes the uplink
uplink thread                 drains the ring into QUIC datagrams
network task (tokio)          datagrams of current members, and their leaves → one ring to the render callback
render callback (realtime)   jitter buffers → Opus decoders → mix → soft limiter → device
                              the network task also polls who speaks (atomics the callbacks write) → events
```

Realtime callbacks never allocate, lock or wait; they talk to other threads only through
single-producer single-consumer rings and atomics, and wake the uplink with `Thread::unpark`.
Decoders are preallocated slots. The output device's clock drives playout: the render callback
pulls what it needs, there is no timer. The network task drops frames of members it doesn't know
as present, so a frame still on the way after its sender left doesn't take a voice for good.

RNNoise allocates its FFT state once per thread, on the first frame it processes there; that
first callback after a stream starts is the one exception.

DSP runs in the capture callback: it has the highest priority, holds up under load, and keeps
the AEC reference aligned with the capture frame.

## Jitter buffer

One per sender, pure and synchronous. Its target delay follows the measured arrival jitter. It
adapts continuously: when it holds more than its target it drops the quietest of the next frames
(by the level in the header), when it runs dry or holds too little it inserts concealment. A
missing frame is recovered by FEC from the next packet if that has arrived, else concealed by
Opus PLC. The last frame of a talkspurt ends playout; a lost last frame ends it after a timeout.

## Gate

Voice activation opens when RNNoise's voice probability is high and the level stands above an
adaptive noise floor, both measured after RNNoise and before AGC. It holds 300 ms after speech
and sends 50 ms of pre-roll when it opens.

## Engine and platforms

The `Engine` in the core owns the runtime and the network side. The apps control it through a
synchronous UniFFI API (join, leave, settings) and get events through a callback interface, on
a thread of their own. Audio never crosses UniFFI.

A join hands back the call's two realtime ends, a `Microphone` the capture callback pushes
48 kHz mono into and a `Speaker` the render callback pulls it from, in buffers of any length.
They, the audio and the uplink thread exist only for a call.

- Desktop (Linux, Windows): the `desktop` crate plays them on `cpal` streams, which a thread of
  its own builds anew when a stream reports its device lost or itself invalid (a stream the
  backend rerouted itself plays on). A stream lends its end to the callback, which hands it back
  when the stream drops. Devices without 48 kHz are mixed to mono and resampled with rubato's
  `Fft` in 10 ms chunks; the drift between clocks is the jitter buffers' to absorb. Device
  choice is part of the desktop crate's API.
- iOS: Swift sets up the audio session, CallKit and the VoiceProcessingIO unit, and registers
  Rust `extern "C"` functions from the `ffi` crate as its input and render callbacks. Swift
  disposes the unit before it frees the engine. VoiceProcessingIO does AEC and AGC there.

The audio stops, restarts and rebuilds independently of the network session (interruptions,
route changes, device loss); the speaker flushes its jitter buffers when it resumes.

Noise suppression can be turned off, for a microphone whose noise the system or the device
suppresses already: RNNoise then still tells the gate what is speech, and the frame goes out as
captured.

`unsafe` is allowed only in the `ffi` crate.

## Order of work

1. Protocol (frame index as timestamp, level), jitter buffer, and a deterministic simulator
   (virtual clocks, drift, jitter, loss traces)
2. Playout: decoding, mixing, limiter, speaking state
3. Gate and capture: RNNoise, Opus, the hybrid-mode test
4. Engine: threads, uplink, network task, synchronous API; the `desktop` crate (`cpal`), devices in the CLI
5. Desktop AGC and high-pass with `sonora`
6. Desktop AEC: `cpal` with `sonora`, or native WASAPI (communications mode, loopback reference)
7. iOS app (`ffi`, VoiceProcessingIO, CallKit), Windows app (WinUI 3)
