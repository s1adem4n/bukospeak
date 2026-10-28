# Vendored crates

Upstream crates that BukoSpeak patches, wired in by `[patch.crates-io]` in the workspace's
`Cargo.toml`. Each is the published crate as it is, but for the changes listed here.
`deep_filter` is the exception: no published crate but a trimmed copy whose inference is our
own, depended on by path.

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

## opus 0.4.0

From <https://github.com/SpaceManiac/opus-rs>, MIT or Apache-2.0 (`LICENSE-MIT`,
`LICENSE-APACHE`).

The mixer conceals lost frames with libopus's neural model, which wants a decoder complexity of
5 or more and libopus built with it; as published the decoder can't take a complexity and the
build leaves the model out.

- `src/lib.rs`: `set_complexity` and `get_complexity` for decoders.
- `Cargo.toml`: the feature `dred`, passed on to `opusic-sys`, whose build of libopus brings the
  neural concealment along with DRED.

## deep_filter 0.5.7-pre

From <https://github.com/ismailivanov/DeepFilterNetPlus>, a maintained fork of
<https://github.com/Rikorose/DeepFilterNet>, at commit `2dc5273d16e1e3769527ad0740654c5ccde581cc`:
its crate `libDF`, MIT or Apache-2.0 (`LICENSE-MIT`, `LICENSE-APACHE`). The fork keeps upstream's
algorithm and removed most of its per-hop allocations.

The capture enhances the microphone's signal with DeepFilterNet3, a hop of 10 ms at a time, on
the uplink thread. Only that is kept: one channel, the model's weights read from memory, and
inference; the dataset, training, LADSPA, C, WASM and command-line parts, the audio file
formats, and logging are gone. The fork runs the model's three networks on tract, which pulses
them from ONNX on every start; here they are written out, layer by layer, so the crate needs no
inference engine, builds the model in milliseconds, and allocates nothing per hop. The output
matches the fork's within float rounding, about 135 dB below the signal, instead of bit for bit:
tract's kernels round differently and approximate the activations (`tests/fork.rs`, against
output the fork produced from the same signal, with the default settings and with the post
filter and an attenuation limit).

- `src/lib.rs`: the STFT, the features and the ERB gains of the fork's `lib.rs`, for one channel;
  the complex features are written in the layout the encoder takes, instead of permuted after.
- `src/filter.rs`: the enhancement of a hop of the fork's `tract.rs`, for one channel, so without
  its mask reduction across channels, which for one channel computes the gains as they are. The
  model's settings are DeepFilterNet3's, fixed, instead of read from its `config.ini`; errors are
  typed (`thiserror`) instead of `anyhow`; nothing logs. `reset` is new: it starts the model anew
  as if just made, its networks' states included, which the fork can't. `hop_size` and `delay`
  tell the caller the hop and the delay the model makes.
- `src/network.rs`: new; the encoder and the two decoders as the fork's ONNX networks compute
  them, a hop at a time: the convolutions along time keep the hops they look back on, the GRUs
  their states.
- `src/weights.rs`: new; reads the model file.
- `examples/convert.rs`: new; writes the model file from the fork's ONNX networks, reading
  protobuf itself: `cargo run --example convert -- export models/DeepFilterNet3.bin`, where
  `export` holds the three networks the fork's `models/DeepFilterNet3_onnx.tar.gz` unpacks, sha256
  `c94d91f70911001c946e0fabb4aa9adc37045f45a03b56008cb0c8244cb63616`.
- `models/DeepFilterNet3.bin`: made so; the weights come with the code's licences.
