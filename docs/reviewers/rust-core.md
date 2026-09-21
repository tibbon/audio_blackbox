# Rust core reviewer

Runs when: the branch touches any Rust file under `src/` outside `src/tests/`, or `Cargo.toml`.
Files the `realtime-audio` and `ffi-unsafe` lanes also cover get both reviews; stay in your lane. Routing lives in `.claude/workflows/ship-ticket.js`.

## In your lane
- Errors and panics (§1.1): library code never panics on bad input; `unwrap`, `expect`, `panic!`,
  and `[]` indexing on external data are findings outside tests. Fallible public fns return
  `Result<_, BlackboxError>`; new variants go in `src/error.rs` (`thiserror::Error`), carry what
  failed and with what input, and keep the source error. `Box<dyn Error>` only in `src/bin/` and
  tests. Panics that can still happen are documented under `# Panics`.
- `#[must_use]` on builders, handles, and results whose drop is a bug (§1.1).
- Ownership (§1.3): a `clone()` added to quiet the borrow checker is a question about who owns the
  value; `Arc::clone(&x)` rather than `x.clone()`; `Arc<Mutex<T>>` only with a named owner and a
  single writer; shared buffers are `Arc<[T]>`; hot loops take slices, not `Vec<T>`.
- Numeric and units (§1.4): every `as` cast is checked for truncation and sign (the crate allows
  `as_conversions` and relies on the `cast_*` lints; a deliberately lossy conversion goes through
  `src/numeric.rs` with its bound stated, except local `#[expect]`s in `bench-writer` and
  `property_size`, so read the remaining casts by hand); sizes crossing to C use `try_from`. Constants carry units in name or doc
  (`_hz`, `_secs`, `_mb`). Float comparisons use a tolerance; NaN is handled explicitly; sorting
  uses `total_cmp`.
- Configuration (`src/config.rs`): precedence stays env over TOML over defaults, and within env
  `BLACKBOX_*` over the legacy unprefixed name. A new `AppConfig` field gets a `BLACKBOX_*`
  override, a default in `src/constants.rs`, forgiving parsing (log and fall back), and a row in
  the AGENTS.md env-var table. Validation rules that exist today stay: `output_dir` rejects `..`
  traversal, `bits_per_sample` accepts only 16, 24, 32. Only `config.rs` reads env vars or home
  paths (§5 missing boundaries).
- API surface (§1.6): `pub` means Swift, a binary, or another crate calls it; everything else is
  `pub(crate)`. The lib root re-exports stay intentional (`src/lib.rs`). Public types implement
  `Debug`; `Debug` impls on types holding locks never take them.
- Threads (§1.5): `std::thread::spawn` in library code has an owned `JoinHandle`, a stop signal,
  and a checked join. Join and send results are logged or returned, never `let _`.
- Binaries (`src/bin/main.rs`, `src/bin/bench_writer.rs`): exit codes stay meaningful;
  `scripts/bench-assert.sh` parses bench-writer's "Real-time:" line, so that output format is a
  contract.
- Docs (§1.6): doc comments say what and why; intra-doc links to private items use backticks.

## Not your lane
- The capture callback, writer thread, WAV writer, and atomics: `realtime-audio`.
- `unsafe`, `extern "C"`, and the header: `ffi-unsafe`. Test quality: `tests`.
- Dependency manifests, MSRV, lint policy tables: `build-release`.
- Anything `scripts/check.sh` fails on mechanically (fmt, clippy -D warnings, swiftlint --strict,
  header parity, catalog sync). The gate catches those; do not report them.

## Severity in this lane
- critical: a panic reachable from user config or device input in the shipped app; a cast that
  corrupts a size written to disk or passed to C.
- high: config precedence or validation changed so a user's setting is ignored or wrong; an error
  mapped to the wrong `BlackboxError` variant so the app shows the wrong message.
- medium: a new fallible path that swallows its error; an `as` cast with a plausible truncation;
  a new `pub` item nothing outside the crate uses; a new config field missing its env override.
- low: naming, doc wording, a clearer iterator form.

## Facts that prevent false positives
- `panic = "abort"` in release (DOLL-90). Never recommend `catch_unwind`.
- Unparseable env values and bad TOML log and fall back to defaults on purpose (`src/config.rs`
  module doc). That is the documented policy, not a swallowed error.
- `#[allow]` does not compile here; lint exceptions are `#[expect(lint, reason = "...")]`.
- `bench_writer.rs` copies `f32_to_wav_sample` from the library on purpose and says so.
- Tests may `unwrap` and `expect` (`clippy.toml` allows them in tests).
- `thread::sleep` in the CLI status tick (`src/bin/main.rs`) and the perf-log thread
  (`src/benchmarking.rs`) carry `#[expect]` with reasons; those are the accepted exceptions.
- `anyhow` is not used in the library and should not be introduced.
