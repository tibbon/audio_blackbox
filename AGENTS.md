# Contributor map

How to land a change. Read the README for what the project *is*; this file is the workflow.

## Workflow

In Claude Code, `/ship-ticket DOLL-N` runs every step below the same way each time (DOLL-654, `.claude/workflows/ship-ticket.js`): intake, a read-only plan that stops with questions when the ticket needs a decision, branch, implement, `make check`, a review loop, PR, CI, merge, and Linear. The review loop runs reviewers from [docs/reviewers/](docs/reviewers/README.md) chosen by the files the branch touches. A skeptic agent confirms, refutes, or marks each finding out of scope for the ticket, and one fixer applies small confirmed fixes. Later rounds review only those fixes. The loop converges when a round confirms nothing to fix, and stops early when a round confirms more than half (rounded up) of the one before. Out-of-scope requests become a follow-up ticket instead of scope creep. Nothing is pushed unless review converged and the full `make check` is green. `scripts/test-ship-ticket.mjs` tests the workflow's control flow with mocked agents.

- `/ship-ticket DOLL-N plan` stops after the plan; `implement`, `review` and `pr` stop after those stages. A stage word right after the ticket always stops there, and any words after it are guidance. `no-merge` leaves the green PR open. `rounds=N` and `bar=high` tune the review loop. Any other words go to the planner as guidance.
- `/ship-ticket review` runs only the review loop on the current branch, committing fixes as `fixup!` commits.
- `/ship-ticket` with no ticket lists the open candidates.
- Rerunning `/ship-ticket DOLL-N` resumes an existing branch and PR. Confirmed pre-existing problems become Linear follow-up tickets instead of scope creep.

By hand, the same steps:

1. Pick or file a ticket in the Linear [Audio Blackbox project](https://linear.app/cyberdyne-systems/project/audio-blackbox-fdadb8f8be42). Title and description are the source of truth — paste any context the PR needs.
2. Branch off `main` as `tibbon/doll-N-short-slug`. The Linear branch button generates this name verbatim.
3. Review the branch against [docs/REVIEW-CHECKLIST.md](docs/REVIEW-CHECKLIST.md), then open a PR. Mention the ticket in the body (`Closes DOLL-N.`) and list what checklist §6.4 asks for.
4. Run `make check` before opening the PR (DOLL-652). It runs the CI gates locally — fmt, clippy on all three feature sets with `-D warnings`, rustdoc, tests, the benchmark smoke floors, `Cargo.lock` freshness, `cargo deny`, `cargo machete`, MSRV, FFI header parity, ACKNOWLEDGMENTS drift, swift-format, swiftlint `--strict`, xcodebuild test under Swift 6 strict concurrency with warnings as errors, swiftlint analyze, String Catalog sync, pbxproj parity, a Release-configuration app build, plus the local-only Claude workflow tests (needs `node`) — and Actions minutes are scarce, so green locally first. `make check-rust` / `make check-swift` run one half; `make fmt` autoformats both languages.
5. CI must be fully green before merge. Lanes: Format, Clippy (+ rustdoc, machete), MSRV (1.98), Test, FFI, Security audit (cargo deny), Benchmark smoke test, Swift app (+ swift-format, swiftlint, analyze).
6. Merge via `gh pr merge <num> --rebase --admin` (linear history; keeps GitHub UI bright green for solo branches).
7. Mark the Linear ticket Done with the PR URL attached.

## Guardrails (DOLL-652)

[docs/REVIEW-CHECKLIST.md](docs/REVIEW-CHECKLIST.md) is the review standard and the rules of engagement for agents: what the lints enforce, what a reviewer still has to judge, and the definition of done. Read section 0 before your first change. The short version:

- **Never silence a lint.** Rust: `#[expect(lint, reason = "...")]`, never `#[allow]` (it is a compile error). Package-wide policy lives in `Cargo.toml [lints]` with a comment; a lint that must be parked goes in its backlog block with a site count and a ticket (empty since DOLL-653). Swift: `// swiftlint:disable:next rule - reason`, never `disable all`.
- **Never weaken a test** to make a change pass; fix the test in its own commit and say so.
- **`unsafe`, FFI, `@unchecked Sendable`, `assumeIsolated`, `Task.detached`** each carry a `// SAFETY:` comment stating the invariant. If you cannot state it, the code is not ready.
- **Hard bans with reasons** are in `clippy.toml` (`thread::sleep`, `home_dir`, `partial_cmp`, `env::set_var`/`remove_var`) and `.swiftlint.yml` custom rules (`print`, GCD hops, deprecated SwiftUI API, unbalanced security-scoped access, and more). The diagnostic tells you the alternative.
- **Swift 6 strict concurrency with `MainActor` default isolation** is on (`BlackBoxApp/Guardrails.xcconfig`, applied through `project.yml`). Types that run off the main actor say so explicitly.

## Invariants

These are non-obvious and have bitten past releases. Read before changing related code.

- **`make rust-lib` before `xcodebuild`.** The Xcode project links against `target/release/libblackbox.a` produced by `cargo build --features ffi`. Without it, the link step fails with `library not found for -lblackbox`. `make swift-app` and `make archive` depend on it; running `xcodebuild` directly does not. CI handles this in the `swift-app` lane.
- **`BLACKBOX_*` env vars take precedence over `blackbox.toml`.** See `src/config.rs`. Tests inject configuration via env vars to avoid touching the config file.
- **`panic = "abort"` is a release-build invariant (DOLL-90).** Any panic in production is a bug we want to surface via crash report — *not* unwind across the FFI boundary. Do not add `catch_unwind` wrappers; do not flip `panic = "unwind"` for release.
- **`make check-app-store` is the OpenAPI lint that catches schema drift before `fastlane mac metadata`.** It validates the metadata directory against the App Store Connect schema fastlane targets. `make verify` runs it; `make fl-metadata` does NOT (you must run `check-app-store` yourself, or run `make verify` first). If it fails, fix the metadata; don't bypass it — Apple's web upload will reject the same payload.
- **`project.yml` is the source of truth for the Xcode project**, but the generated `.xcodeproj` is committed too. The `swift-app` CI lane enforces they match: it installs a pinned `xcodegen`, regenerates, and fails on any diff (DOLL-160). If you edit `project.yml`, run `make xcodegen` and commit the regenerated `.xcodeproj` in the same change, or CI will reject the PR.
- **Lint policy is in `Cargo.toml`, thresholds and bans in `clippy.toml`, and every `allow` there has a reason.** Clippy does not merge configs, and `-D warnings` in CI makes every `warn` an error at the gate. If a new lint fires on code you did not touch, fix it or park it in the `Cargo.toml` backlog block with a count and a ticket — do not `#[expect]` your way through unrelated files.
- **`project.yml` `settings` override `Guardrails.xcconfig`.** xcodegen writes `settings` as build settings, which beat the xcconfig. Language mode, concurrency, warnings-as-errors and hardening keys belong in the xcconfig; `settings` keeps only identity/version keys `scripts/check-versions.sh` reads.
- **`include/blackbox_ffi.h` is hand-maintained** (no cbindgen). When you add or remove a `pub extern "C" fn` in `src/ffi.rs`, edit the header by hand and confirm with `make check-ffi-header`. The swift-app CI lane runs the same check before building, so missing-header drift fails fast there too (DOLL-190).

See [ARCHITECTURE.md](ARCHITECTURE.md) for the threading model, lock-acquisition order, and the deeper invariants behind the audio path / FFI boundary.

For a map of `DOLL-N` ticket references scattered through code and comments, see [docs/decisions.md](docs/decisions.md) — one-paragraph summary per load-bearing decision, so a future contributor without Linear access can still navigate the history.

## Environment variables (DOLL-198)

Every `AppConfig` field has a `BLACKBOX_*` env-var override; for backward compatibility most also accept an unprefixed legacy name. `BLACKBOX_*` wins when both are set and it parses; a `BLACKBOX_*` value that fails to parse falls through to the legacy name (`env_override` in `src/config.rs`).

| Field | `BLACKBOX_*` | Legacy alias | Notes |
|-------|------------|-------------|-------|
| (config file path) | `BLACKBOX_CONFIG` | — | Absolute or relative path to a `.toml`. Wins over the search order. |
| `audio_channels` | `BLACKBOX_AUDIO_CHANNELS` | `AUDIO_CHANNELS` | Comma + range form (`"0,2-4,7"`). 0-based. |
| `debug` | `BLACKBOX_DEBUG` | `DEBUG` | `true`/`false`. |
| `duration` | `BLACKBOX_DURATION` | `RECORD_DURATION` | Seconds; `0` = unlimited. |
| `output_mode` | `BLACKBOX_OUTPUT_MODE` | `OUTPUT_MODE` | `"single"` or `"split"`. |
| `silence_threshold` | `BLACKBOX_SILENCE_THRESHOLD` | `SILENCE_THRESHOLD` | Normalized amplitude 0.0–1.0 (`0.01` ≈ 1% full-scale); `0` disables; negative/NaN fall back to the default. |
| `continuous_mode` | `BLACKBOX_CONTINUOUS_MODE` | `CONTINUOUS_MODE` | `true`/`false`. |
| `recording_cadence` | `BLACKBOX_RECORDING_CADENCE` | `RECORDING_CADENCE` | Seconds between rotations. |
| `output_dir` | `BLACKBOX_OUTPUT_DIR` | `OUTPUT_DIR` | Path; rejects `..` traversal. |
| `performance_logging` | `BLACKBOX_PERFORMANCE_LOGGING` | `PERFORMANCE_LOGGING` | Needs `benchmarking` feature. |
| `input_device` | `BLACKBOX_INPUT_DEVICE` | `INPUT_DEVICE` | cpal device name; unset = system default. |
| `min_disk_space_mb` | `BLACKBOX_MIN_DISK_SPACE_MB` | `MIN_DISK_SPACE_MB` | `0` disables the check. |
| `bits_per_sample` | `BLACKBOX_BITS_PER_SAMPLE` | `BITS_PER_SAMPLE` | 16 / 24 / 32; others rejected. |
| `silence_gate_enabled` | `BLACKBOX_SILENCE_GATE_ENABLED` | `SILENCE_GATE_ENABLED` | `true`/`false`. |
| `silence_gate_timeout_secs` | `BLACKBOX_SILENCE_GATE_TIMEOUT_SECS` | `SILENCE_GATE_TIMEOUT_SECS` | Seconds before gate closes. |

**Precedence**: env > TOML > built-in defaults (`src/constants.rs::DEFAULT_*`). Inside env, a parseable `BLACKBOX_*` > unprefixed legacy. This applies to the CLI; the app configures the engine through the FFI (`blackbox_set_config_json`), which never reads env or TOML.

**Validation policy**: forgiving — unparseable env values log and fall back rather than error (see `src/config.rs` module doc).

## Releases

Tag-driven via the release workflow; see [SETUP.md](SETUP.md#releasing). Tag with `make release VERSION=X.Y.Z`, not a bare `git tag`: the Makefile checks the version against `Cargo.toml` before tagging, and the workflow does not. `scripts/check-versions.sh` enforces alignment between `Cargo.toml`, the `Makefile`, `project.yml`, and `Info.plist`. Fastlane handles TestFlight + App Store submission using ASC API key auth (key path is `~/Library/Application Support/com.dollhousemediatech.blackbox/keys/AuthKey_*.p8` — outside the repo, see DOLL-155).

## Style

- **Comments**: explain *why*, not *what*. Reference Linear ticket IDs for context that isn't obvious from the diff.
- **Tests**: integration tests live in `src/tests/`; the inline `mod tests` in `src/lib.rs` is for tiny smoke tests only — anything substantial belongs in a dedicated file under `src/tests/`.
- **CI is macOS-only** (DOLL-153). The shipped product is the Mac App Store app; running Ubuntu lanes added cost without adding signal for the SwiftUI / CoreAudio surface.
