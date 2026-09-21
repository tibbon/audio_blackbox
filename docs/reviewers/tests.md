# Tests reviewer

Runs when: the branch touches `src/tests/`, `src/test_utils.rs`, `src/mock_processor.rs`, or
`BlackBoxApp/BlackBoxAppTests/`. Inline `mod tests` edits route by their file, so judge them when
you see them in the diff. Routing lives in `.claude/workflows/ship-ticket.js`.
Missing tests for new behavior belong to `baseline`; this lane judges the tests that exist.

## In your lane
- Tampering (§5): assertions loosened, tolerances widened without a stated reason, tests deleted
  or renamed so they no longer run, a new `#[ignore]` or `XCTSkip`, expected values changed to
  match new output without explaining why the old value was wrong. §0 wants a corrected test in
  its own commit with the reason in the PR.
- Assertions that assert (§1.7): a test that cannot fail (asserting on a value it just set,
  `assert!(result.is_ok() || result.is_err())`, a loop over an empty collection); `is_ok()` checks
  where the value matters; a panic-free run treated as proof.
- Public behavior (§1.7): tests go through the crate API or the FFI surface; reaching into private
  state to assert implementation details is a finding unless the test documents an invariant that
  has no public observable (the zero-allocation proof in `src/tests/alloc_tests.rs` is one).
- Waiting (§1.5, §1.7): no `thread::sleep` or `Task.sleep` to synchronize. Rust tests wait on state
  with `wait_for_samples_consumed` and `wait_for_flag_cleared` from `src/test_utils.rs`, or drive
  time with `MockClock`. Timeouts are generous and fail loudly, not silently pass.
- Floats and audio (§1.4): no `assert_eq!` on floats; tolerances are stated in the test with a
  reference value; integer PCM comparisons allow the documented rounding (`abs() <= 1` in
  `cpal_integration_tests.rs`); dither and silence thresholds are tested at their boundaries.
- Environment isolation: config tests use `temp_env::with_vars` and the `default_test_env()` helper
  so a developer's `BLACKBOX_*` variables cannot leak in; tests that write files use `tempfile`.
- FFI coverage (§2, §3.8): every export in `src/ffi.rs` is called from `src/tests/ffi_tests.rs`
  with null, zero, and oversized inputs, asserting the `BLACKBOX_ERR_*` code, not just "did not
  crash". A new export extends it. Swift wrapper tests cover error paths and empty results.
- Real-time proof: a hot-path change keeps `alloc_tests.rs` meaningful (it still exercises the
  changed path) rather than just passing.
- Flakiness: order dependence between tests that share process state (env vars, global
  allocator counter, `UserDefaults` suites), wall-clock thresholds tight enough to fail on a loaded
  CI runner, reliance on an audio device being present.
- Swift (§3.8): existing files are XCTest; new tests may use Swift Testing, but never both in one
  file. XCTest classes are `nonisolated` and mark main-actor tests `@MainActor`. UI logic is tested
  through models and pure helpers, not views.

## Not your lane
- Whether production code is correct: `baseline`, `realtime-audio`, `ffi-unsafe`, `rust-core`,
  `swift-app`. CI lanes that run the tests: `build-release`.
- Anything `scripts/check.sh` fails on mechanically (fmt, clippy -D warnings, swiftlint --strict,
  header parity, catalog sync). The gate catches those; do not report them.

## Severity in this lane
- critical: a change that stops a whole test module from compiling into any CI lane, so its tests
  silently never run.
- high: a weakened or deleted test that guarded a real behavior; `alloc_tests` no longer covering
  the hot path; an assertion changed to match a bug.
- medium: a sleep-based wait; a float `assert_eq!`; a test that cannot fail; a new export or error
  path with no FFI test; env leakage between tests.
- low: naming, duplicated setup that a helper already covers, message wording.

## Facts that prevent false positives
- The tracker-dependent tests in `src/tests/performance_tests.rs` are `#[ignore]` with a reason and
  run in the weekly ignored-tests lane (DOLL-450). Existing ignores with reasons are not tampering.
- Rust tests run with `--test-threads=1` in `scripts/check.sh` and CI, because several tests share
  env vars and the counting allocator. Do not report missing per-test locking for that.
- `clippy.toml` allows `unwrap`, `expect`, and indexing in tests.
- `src/tests/performance_tests.rs` carries a file-level `#[expect(clippy::disallowed_methods)]` because
  the tracker samples on its own thread at a fixed interval; that sleep is the point of the test.
- The silent-pass `is_ci()` skip pattern was removed (DOLL-455). Reintroducing any skip-when-CI
  logic is a finding.
- There is no top-level `tests/` directory by design; integration tests live under `src/tests/` to
  share `pub(crate)` access.
