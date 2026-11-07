# BukoSpeak

See `README.md` for what the parts are. Run `mise run check` before calling a change done.

## Code

- Clean from the start: no workarounds, no dead code, no silently dropped errors. A dropped
  result (`let _ =`) is right by its context, or by a comment when the context can't show it.
- Errors are typed (`thiserror`) in libraries; `anyhow` only in binaries and the server.
- Shared dependency versions and lints live in the workspace `Cargo.toml`.
- Code explains itself through names, types and small functions. A comment that seems needed
  means the code should be clearer; rename, extract or retype instead of commenting.
- No doc comments, section labels, comments restating code, commented-out code or history.
- A comment stays only for a why the code cannot carry: an outside constraint, a platform or
  protocol quirk, a choice that looks wrong but isn't. Tests carry their meaning in their names.
- Unit tests sit in a `#[cfg(test)] mod tests` at the end of the file they test and may use its
  internals; integration tests, which use only public APIs, possibly of several crates, go in a
  crate's `tests/` folder (`server/tests`: a real server and clients over QUIC on localhost).
- Tests exercise real behavior where they can; timing logic runs on the deterministic
  simulator in `core/src/simulator.rs`, not on wall clocks.
- No real people's names in code, tests or docs; use placeholders such as `first` or `member`.
- `unsafe` only in `ios/ffi`, which binds native interfaces, and `maxine`, which binds NVIDIA's
  runtime; each block says why it is sound (`// SAFETY:`).
- After changing the `ios/ffi` crate's interface, `mise run header` rewrites its C header.

## iOS

`ios/dev build` builds the app on the Mac mini (`ios/dev help` lists the commands); a first build
there, or one after `Cargo.lock` changed, takes many minutes, so run it in the background. Only
`ios/dev build` lints and compiles the iOS parts of `ios/ffi`, whose dependencies need the iOS
SDK. There is no simulator workflow: the app is tried on the iPhone, installed with
`ios/dev ota`.
