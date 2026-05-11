# Contributor map

How to land a change. Read the README for what the project *is*; this file is the workflow.

## Workflow

1. Pick or file a ticket in the Linear [Audio Blackbox project](https://linear.app/cyberdyne-systems/project/audio-blackbox-fdadb8f8be42). Title and description are the source of truth — paste any context the PR needs.
2. Branch off `main` as `tibbon/doll-N-short-slug`. The Linear branch button generates this name verbatim.
3. Open a PR. Mention the ticket in the body (`Closes DOLL-N.`).
4. CI must be fully green before merge. Lanes: Format, Clippy, MSRV (1.95), Test, Security audit, Benchmark smoke test, Swift app.
5. Merge via `gh pr merge <num> --rebase --admin` (linear history; keeps GitHub UI bright green for solo branches).
6. Mark the Linear ticket Done with the PR URL attached.

## Invariants

These are non-obvious and have bitten past releases. Read before changing related code.

- **`make rust-lib` before `xcodebuild`.** The Xcode project links against `target/release/libblackbox.a` produced by `cargo build --features ffi`. Without it, the link step fails with `library not found for -lblackbox`. `make swift-app` and `make archive` depend on it; running `xcodebuild` directly does not. CI handles this in the `swift-app` lane.
- **`BLACKBOX_*` env vars take precedence over `blackbox.toml`.** See `src/config.rs`. Tests inject configuration via env vars to avoid touching the config file.
- **`panic = "abort"` is a release-build invariant (DOLL-90).** Any panic in production is a bug we want to surface via crash report — *not* unwind across the FFI boundary. Do not add `catch_unwind` wrappers; do not flip `panic = "unwind"` for release.
- **`make check-app-store` is the OpenAPI lint that catches schema drift before `fastlane mac metadata`.** It validates the metadata directory against the App Store Connect schema fastlane targets. `make verify` runs it; `make fl-metadata` does NOT (you must run `check-app-store` yourself, or run `make verify` first). If it fails, fix the metadata; don't bypass it — Apple's web upload will reject the same payload.
- **`project.yml` is the source of truth for the Xcode project**, but the generated `.xcodeproj` is committed too. Drift recovery is currently manual (DOLL-160 parked the auto-check pending a design decision around fastlane's release-time pbxproj mutations).
- **`include/blackbox_ffi.h` is hand-maintained** (no cbindgen). When you add or remove a `pub extern "C" fn` in `src/ffi.rs`, edit the header by hand and confirm with `make check-ffi-header`. The swift-app CI lane runs the same check before building, so missing-header drift fails fast there too (DOLL-190).

See [ARCHITECTURE.md](ARCHITECTURE.md) for the threading model, lock-acquisition order, and the deeper invariants behind the audio path / FFI boundary.

For a map of `DOLL-N` ticket references scattered through code and comments, see [docs/decisions.md](docs/decisions.md) — one-paragraph summary per load-bearing decision, so a future contributor without Linear access can still navigate the history.

## Environment variables (DOLL-198)

Every `AppConfig` field has a `BLACKBOX_*` env-var override; for backward compatibility most also accept an unprefixed legacy name. `BLACKBOX_*` always wins when both are set.

| Field | `BLACKBOX_*` | Legacy alias | Notes |
|-------|------------|-------------|-------|
| (config file path) | `BLACKBOX_CONFIG` | — | Absolute or relative path to a `.toml`. Wins over the search order. |
| `audio_channels` | `BLACKBOX_AUDIO_CHANNELS` | `AUDIO_CHANNELS` | Comma + range form (`"0,2-4,7"`). 0-based. |
| `debug` | `BLACKBOX_DEBUG` | `DEBUG` | `true`/`false`. |
| `duration` | `BLACKBOX_DURATION` | `RECORD_DURATION` | Seconds; `0` = unlimited. |
| `output_mode` | `BLACKBOX_OUTPUT_MODE` | `OUTPUT_MODE` | `"single"` or `"split"`. |
| `silence_threshold` | `BLACKBOX_SILENCE_THRESHOLD` | `SILENCE_THRESHOLD` | Percent (0–100); `0` or negative disables. |
| `continuous_mode` | `BLACKBOX_CONTINUOUS_MODE` | `CONTINUOUS_MODE` | `true`/`false`. |
| `recording_cadence` | `BLACKBOX_RECORDING_CADENCE` | `RECORDING_CADENCE` | Seconds between rotations. |
| `output_dir` | `BLACKBOX_OUTPUT_DIR` | `OUTPUT_DIR` | Path; rejects `..` traversal. |
| `performance_logging` | `BLACKBOX_PERFORMANCE_LOGGING` | `PERFORMANCE_LOGGING` | Needs `benchmarking` feature. |
| `input_device` | `BLACKBOX_INPUT_DEVICE` | `INPUT_DEVICE` | cpal device name; unset = system default. |
| `min_disk_space_mb` | `BLACKBOX_MIN_DISK_SPACE_MB` | `MIN_DISK_SPACE_MB` | `0` disables the check. |
| `bits_per_sample` | `BLACKBOX_BITS_PER_SAMPLE` | `BITS_PER_SAMPLE` | 16 / 24 / 32; others rejected. |
| `silence_gate_enabled` | `BLACKBOX_SILENCE_GATE_ENABLED` | `SILENCE_GATE_ENABLED` | `true`/`false`. |
| `silence_gate_timeout_secs` | `BLACKBOX_SILENCE_GATE_TIMEOUT_SECS` | `SILENCE_GATE_TIMEOUT_SECS` | Seconds before gate closes. |

**Precedence**: env > TOML > built-in defaults (`src/constants.rs::DEFAULT_*`). Inside env, `BLACKBOX_*` > unprefixed legacy.

**Validation policy**: forgiving — unparseable env values log and fall back rather than error (see `src/config.rs` module doc).

## Releases

Tag-driven via the release workflow. `scripts/check-versions.sh` enforces alignment between `Cargo.toml`, `project.yml`, and `Info.plist`. Fastlane handles TestFlight + App Store submission using ASC API key auth (key path is `~/Library/Application Support/com.dollhousemediatech.blackbox/keys/AuthKey_*.p8` — outside the repo, see DOLL-155).

## Style

- **Comments**: explain *why*, not *what*. Reference Linear ticket IDs for context that isn't obvious from the diff.
- **Tests**: integration tests live in `src/tests/`; the inline `mod tests` in `src/lib.rs` is for tiny smoke tests only — anything substantial belongs in a dedicated file under `src/tests/`.
- **CI is macOS-only** (DOLL-153). The shipped product is the Mac App Store app; running Ubuntu lanes added cost without adding signal for the SwiftUI / CoreAudio surface.
