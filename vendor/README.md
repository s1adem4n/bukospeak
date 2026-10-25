# Vendored crates

Upstream crates that BukoSpeak patches, wired in by `[patch.crates-io]` in the workspace's
`Cargo.toml`. Each is the published crate as it is, but for the changes listed here.

## sonora and sonora-aec3 0.2.0

From <https://github.com/dignifiedquire/sonora>, BSD-3-Clause (`LICENSE` beside each).

The capture callback runs their high-pass filter and AEC3, and a realtime callback must not
allocate; as published they allocate about 40 times per 10 ms frame. The changes take that to
none, and keep upstream's tests passing:

- `sonora-aec3/src/echo_remover.rs`: the per-channel working storage of `process_capture` lives
  in `CaptureScratch`, made with the remover and zeroed before each block, instead of fresh
  vectors per block.
- `sonora-aec3/src/subtractor_output.rs`: `SubtractorOutput` derives `Debug`, for
  `CaptureScratch`'s.
- `sonora-aec3/src/frame_blocker.rs`: `insert_sub_frame_and_extract_block` takes bands of
  channels as anything that lends slices, so callers pass their buffers as they are; and each
  channel's buffer has its capacity from the start, which `vec!`'s clones had not.
- `sonora/src/echo_canceller3.rs`: render and capture pass their sub-frame buffers to the frame
  blockers directly, instead of collecting views of them per sub-frame.
- `sonora/src/audio_buffer.rs`: `copy_to_buffer` resamples into the destination channel,
  instead of into a temporary vector.
