# Guardrails reviewer

Runs when: always. Routing lives in `.claude/workflows/ship-ticket.js`.

This lane checks the agent failure modes in `docs/REVIEW-CHECKLIST.md` §0 and §5, and that the
contracts and docs around the code moved with it.

## In your lane
- Lint silencing (§0, §5): new `#[expect]` whose `reason` does not state a true invariant
  ("clippy is wrong", "needed to compile"); an `#[expect]` placed on unrelated files to get green
  (AGENTS.md: park it in the `Cargo.toml` backlog block with a count instead); `// swiftlint:disable`
  without `:next` or without a reason; new `@unchecked Sendable`, `nonisolated(unsafe)`, `try!`,
  `as!`, force unwraps outside tests.
- Crate-wide lint policy changed in `Cargo.toml [lints]` or `clippy.toml` without a comment, or a
  backlog lint re-enabled without fixing its sites.
- Test tampering (§5): a loosened assertion, a widened float tolerance with no reason, a deleted
  test, a new `#[ignore]` or `XCTSkip`, a test edited in the same commit as the code it guards
  without the PR saying why (§0 wants that change in its own commit).
- Stale contracts (§5): a Rust struct or export changed without `include/blackbox_ffi.h` and
  `RustBridge.swift` updated in the same change; `project.yml` edited without the regenerated
  `.xcodeproj`; a changed invariant that AGENTS.md "Invariants", `ARCHITECTURE.md`, or
  `docs/decisions.md` still describes the old way; a new `BLACKBOX_*` field missing from the
  AGENTS.md env-var table; a new tool or setup step missing from `SETUP.md`.
- Fake verification (§5): a PR or commit message that claims tests pass, "verified on device", or
  benchmark numbers the branch did not produce. Numbers must come from a `scripts/check.sh` run.
- Comment rot (§5): comments that restate code, describe an earlier version, or cite a
  `DOLL-N` for something the diff already explains. New `DOLL-N` references that the ticket map
  in `docs/decisions.md` should list but does not (for load-bearing decisions only).
- Deprecated or iOS idioms (§0): `NavigationView`, `.foregroundColor`, one-parameter `onChange`,
  `ObservableObject` for a new model, GCD or `Thread` for new concurrency, `NSLock`, `UIKit`.
- Over-generalization and ceremony (§5): a trait, protocol, or generic with one implementer; a
  "manager", "service", or "helper" that forwards calls without owning an invariant.
- Copy-paste drift (§5): a second implementation of an existing helper in Rust or Swift.
- New dependencies (§0, §1.8): justified in one line, not replacing twenty lines of code;
  `ACKNOWLEDGMENTS.md` regenerated; no `anyhow` in the library (the crate uses `thiserror` +
  `BlackboxError`); no `ObservableObject` in the app.
- PR description obligations (§6.4): when the branch changes a public API, the FFI or header, a
  buffer or file layout (`StatusFlags`, WAV format), dependencies, or adds lint expectations, the
  PR description lists each one, with the reason for every expectation.

## Not your lane
- Whether the code is correct: `baseline`. Whether an `unsafe` or FFI contract holds: `ffi-unsafe`.
- Test quality beyond tampering (flaky waits, weak assertions): `tests`.
- CI workflow, script, and dependency-manifest mechanics: `build-release`.
- Anything `scripts/check.sh` fails on mechanically (fmt, clippy -D warnings, swiftlint --strict,
  header parity, catalog sync). The gate catches those; do not report them.

## Severity in this lane
- critical: a change that disables a CI gate or the pre-commit check for everyone.
- high: a silenced lint or weakened test that hides a real defect; a header or `RustBridge.swift`
  that no longer matches the Rust export it describes.
- medium: an `#[expect]` or swiftlint disable with a false or empty reason; AGENTS.md,
  `ARCHITECTURE.md`, or `docs/decisions.md` now wrong about an invariant; a new dependency with
  no justification; a §6.4 item missing from the PR description.
- low: comment rot, a ticket reference that adds nothing, naming.

## Facts that prevent false positives
- `#[allow]` is already a compile error (`clippy::allow_attributes` is deny), and an `#[expect]`
  without `reason` fails too. Do not report their absence; judge the reason's truth.
- Existing documented exceptions are fine: `thread::sleep` in the writer idle backoff (DOLL-270),
  the CLI status tick in `src/bin/main.rs`, and the perf-log thread in `src/benchmarking.rs`; the
  file-level expects in `src/bin/bench_writer.rs`; `MainActor.assumeIsolated` in
  `GlobalHotkeyManager.swift` (DOLL-161); the deliberate leak in `macos_sample_rate_listener.rs`.
- `CHANGELOG.md` is written in the release PR, not per change; `[Unreleased]` staying empty is
  normal. For a user-visible change, a line in the PR description is enough (low if missing).
- The SwiftLint size thresholds are back at the kit's values (DOLL-653); flag any raise. The
  `Cargo.toml` backlog block is empty; flag any addition that lacks a site count and a ticket.
- Existing Swift tests are XCTest; that is not a deprecated idiom.
