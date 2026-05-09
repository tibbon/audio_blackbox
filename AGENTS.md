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

See [ARCHITECTURE.md](ARCHITECTURE.md) for the threading model, lock-acquisition order, and the deeper invariants behind the audio path / FFI boundary.

## Releases

Tag-driven via the release workflow. `scripts/check-versions.sh` enforces alignment between `Cargo.toml`, `project.yml`, and `Info.plist`. Fastlane handles TestFlight + App Store submission using ASC API key auth (key path is `~/Library/Application Support/com.dollhousemediatech.blackbox/keys/AuthKey_*.p8` — outside the repo, see DOLL-155).

## Style

- **Comments**: explain *why*, not *what*. Reference Linear ticket IDs for context that isn't obvious from the diff.
- **Tests**: integration tests live in `src/tests/`; the inline `mod tests` in `src/lib.rs` is for tiny smoke tests only — anything substantial belongs in a dedicated file under `src/tests/`.
- **CI is macOS-only** (DOLL-153). The shipped product is the Mac App Store app; running Ubuntu lanes added cost without adding signal for the SwiftUI / CoreAudio surface.
