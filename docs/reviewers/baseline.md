# Baseline reviewer

Runs when: always. Routing lives in `.claude/workflows/ship-ticket.js`.

## In your lane
- Correctness of the changed lines: wrong condition or operator, off-by-one in frame, sample, or
  channel arithmetic, swapped arguments, an unhandled enum case or error path that production hits.
- Regressions: read the callers of every changed function and the shared invariants nearby. A
  change to `AudioRecorder`, `CpalAudioProcessor`, `AppConfig`, or an FFI export has callers in
  `src/bin/`, `src/ffi.rs`, `src/tests/`, and `RustBridge.swift`.
- Ticket and plan versus diff: the diff does what the ticket asks. Work outside that scope with no
  visible justification is scope creep (medium). Incidental edits the change needs (an import, a
  test helper) are fine.
- New behavior has a test that would fail if the code regressed (§6.3). Changed numeric behavior
  has a tolerance and a reference value.
- Silent fallbacks that turn a failure into plausible wrong output (§5): `?? default`,
  `unwrap_or_default()`, `try?`, `let _ = result`, an early `return` that skips a flush or join.
- State transitions: start/stop/monitor mutual exclusion, rotation, finalize, and sleep/wake paths
  keep their documented order (`ARCHITECTURE.md` "Sleep / wake matrix", `start()` re-entrancy guard).
- Error messages carry what failed and with what input, and stay safe to log (§1.1).

## Not your lane
- Real-time thread rules, allocation, atomics on the capture path: `realtime-audio`.
- `unsafe`, FFI contracts, header, `RustBridge.swift` pointer handling: `ffi-unsafe`.
- Rust error design, casts, API surface, config precedence: `rust-core`.
- Swift concurrency, SwiftUI/AppKit idioms, sandbox, localization: `swift-app`.
- Test quality and test tampering in changed test files: `tests`. You still flag missing tests.
- Lint silencing, docs drift, PR-description obligations: `guardrails`.
- CI, scripts, dependencies, release: `build-release`.
- Anything `scripts/check.sh` fails on mechanically (fmt, clippy -D warnings, swiftlint --strict,
  header parity, catalog sync). The gate catches those; do not report them.

## Severity in this lane
- critical: a path that loses or truncates a recording, deletes a non-silent file, or crashes on
  normal input (finalize skipped on an error branch, a panic reachable from a bad config value).
- high: a feature does the wrong thing in normal use; an existing behavior regresses (rotation
  stops, silence gate never reopens, device selection ignored).
- medium: a bug behind a narrow trigger; new behavior without a test; a swallowed error; the diff
  goes outside the ticket without saying why.
- low: clearer structure, naming, a simpler equivalent.

## Facts that prevent false positives
- Release builds use `panic = "abort"` (DOLL-90, `Cargo.toml [profile.release]`). A reachable panic
  is still a bug, but the remedy is to not panic, never `catch_unwind`.
- Config validation is forgiving by design (`src/config.rs` module doc): bad TOML, unparseable env
  vars, and out-of-range numbers log a warning and fall back to defaults. That fallback is not a
  silent-fallback finding. Precedence is env over TOML over defaults, `BLACKBOX_*` over legacy names.
- `bench_writer.rs` deliberately copies `f32_to_wav_sample` from the library; the comment above the
  copy says why. Drift between the two is a finding; the duplication is not.
- The silence-check worker's bounded `sync_channel` back-pressures the writer on purpose
  (`ARCHITECTURE.md` "Writer thread").
- Linux builds compile but are best-effort and not in CI (DOLL-153). Do not ask for Linux coverage.
- Review only what changed. Unchanged code visible in a hunk is pre-existing (see README).
