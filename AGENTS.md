# BukoSpeak

See `README.md` for what the parts are. Run `mise run check` before calling a change done.

## Code

- Clean from the start: no workarounds, no dead code, no silently dropped errors. A dropped
  result (`let _ =`) says in a comment or by its context why dropping is right.
- Errors are typed (`thiserror`) in libraries; `anyhow` only in binaries and the server.
- Shared dependency versions and lints live in the workspace `Cargo.toml`.
- Doc comments say what something is or does for its caller; comments say why, not what.
- Unit tests sit in a `#[cfg(test)] mod tests` at the end of the file they test and may use its
  internals; integration tests, which use only public APIs, possibly of several crates, go in a
  crate's `tests/` folder (`server/tests`: a real server and clients over QUIC on localhost).
- Tests exercise real behavior where they can; timing logic runs on the deterministic
  simulator in `core/src/simulator.rs`, not on wall clocks.
- No real people's names in code, tests or docs; use placeholders such as `first` or `member`.
- `unsafe` only in `ios/ffi` and `audio/ios`, which bind native interfaces; each block says why it
  is sound (`// SAFETY:`).
- After changing the `ios/ffi` crate's interface, `mise run header` rewrites its C header.

## iOS

`ios/dev build` builds the app on the Mac mini (`ios/dev help` lists the commands); a first build
there, or one after `Cargo.lock` changed, takes many minutes, so run it in the background. Only
`ios/dev build` lints and compiles `audio/ios` and the iOS parts of `ios/ffi`, whose dependencies
need the iOS SDK. There is no simulator workflow: the app is tried on the iPhone, installed
with `ios/dev ota`.
