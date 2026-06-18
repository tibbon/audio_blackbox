# Changelog

All notable changes to BlackBox Audio Recorder are documented in this file.

The format is based on [Keep a Changelog 1.1.0](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [1.4.0] — 2026-06-17

Polish-pass-7: roughly 100 commits of localization, real-time-safety
hardening, audio-quality, accessibility, and release-pipeline work on
top of 1.3.0. No breaking changes.

### Added
- Full localization groundwork: a base String Catalog
  (`Localizable.xcstrings`, 248 keys) with a deterministic sync script
  and a CI drift check; the remaining ~80 user-facing strings, plus
  AppKit/notification/status literals, are now wrapped; number, byte,
  and rate formatting is locale-aware.
- TPDF dither when down-converting f32 capture to 16-bit PCM —
  removes truncation distortion on quiet passages.
- Silence-gate pre-roll: the idle batch is retained and replayed when
  the gate opens, so the transient that trips the gate is no longer
  clipped off the front of the file.
- VoiceOver coverage across onboarding, the idle meter, the channel
  grid, and the menu; the live audio meter is exposed to VoiceOver.
- Menu recovery affordances for the no-devices and mic-denied states.
- Mechanically-generated third-party license attribution.
- Code-coverage measurement in CI (cargo-llvm-cov + Swift xccov).
- Notarized + stapled DMG in `make dmg`; `make export`/`dmg` now use a
  local Developer ID export plist.

### Changed
- Recordings default to the app container instead of `~/Music`.
- `start()` outcome is awaitable; the re-entrancy guard is held across
  the mic-permission await, and monitor/record mutual exclusion is
  enforced in the engine and the FFI.
- Stream-error auto-restart is capped and backed off on flaky devices.
- Menu warning severity is conveyed via glyph rather than color; the
  meter peak-hold marker decays from a clock; the 30 Hz meter timer is
  paused when its window is occluded or nothing is active.
- First-launch notification-permission request is deferred to the
  first recording start.
- Reduce Motion is honored for the level-bar animation; channel
  checkboxes have larger hit targets; Settings/onboarding width scales
  with Dynamic Type.
- FFI `last_error` now carries the full error source chain; config
  JSON parse failures are recorded there instead of swallowed.
- Build is documented as arm64-only; the dead `rust-lib-universal`
  target was removed.
- Toolchain/dependencies: cpal 0.18 (unified error API) plus routine
  Dependabot bumps.

### Fixed
- The recording engine stops on persistent `write_sample` failures,
  surfacing a distinct `write_failed` status flag and an accurate
  disk-error message; the self-stop latches across rotation
  file-creation failures.
- Live recording status is preserved across
  `blackbox_stop_monitoring`; the resume-on-wake flag survives
  sleep-initiated stops; sleep prevention is released on
  engine-initiated stops.
- Command-channel disconnect is treated as a writer-thread shutdown;
  `finalize_all` finishes every writer before returning an error;
  `rotate_files()` is guarded against `disk_stopped`.
- `silence_threshold` above 1.0 and a `recording_cadence` of 0 are
  rejected rather than producing a per-callback rotation storm.
- A RIFF word-alignment pad byte is written for odd-length data
  chunks.
- `restoreOutputDirBookmark` no longer falls through on access
  failure; the inverted Reset All Settings dialog logic is fixed; the
  mid-recording Restart dialog defaults to Cancel.
- `disambiguate_path` appends a nanosecond suffix instead of
  overwriting an existing file.

### Security
- Release path hardened: cargo-audit and `Cargo.lock` freshness are
  mirrored into the tag gate; the public GitHub Release is gated
  behind the same manual approval as TestFlight; `.p12` and
  provisioning artifacts are gitignored.

## [1.3.0] — 2026-05-20

Polish-pass-6 (build 14): accessibility, UX, real-time-safety, and
CI/release-pipeline hardening on top of 1.2.0. No breaking changes.

## [1.2.0] — 2026-05-08

Polish-pass-5: 21 tickets focused on accessibility, supply-chain
hardening, and dev-experience improvements on top of 1.1.0. No
breaking changes.

### Added
- Manual CodeQL workflow (`.github/workflows/codeql.yml`) scanning
  Swift + Rust + GitHub Actions, replacing the auto-detect default
  setup that wasted analysis time on Python and Ruby.
- MSRV verification job in CI — `cargo check` on the pinned 1.95
  toolchain so MSRV regressions can no longer ship silently.
- Release-time test gate: `release.yml` runs the test suite on the
  tag's exact SHA before any TestFlight upload or GitHub Release
  artifact is published.
- `BlackBoxApp/Gemfile` + `Gemfile.lock` pinning fastlane via
  bundler-cache, so the next minor fastlane regression can no
  longer break `deliver` mid-deploy.
- Dependabot now monitors the Bundler ecosystem alongside Cargo
  and GitHub Actions.

### Changed
- Settings now uses the SwiftUI `Settings` scene instead of a
  generic `Window`. `⌘,` opens it from anywhere in the app, and it
  inherits the platform's normal close-and-reopen semantics.
- Onboarding accessibility: VoiceOver announces the recording-mode
  cards as a single-select Picker group, and the step-indicator
  dots are reachable per-step Buttons (so VO users can step back
  through completed steps).
- Level-meter VoiceOver value bucketed to threshold crossings
  (Silent / Low / Moderate / Hot / Clipping) — no more flood of
  per-tick dB readings on every signal level.
- README rewritten to lead with the Mac App Store product instead
  of `cargo build` instructions.
- `cpal_processor.rs` and `writer_thread.rs` split along natural
  seams — CoreAudio sample-rate listener and the silence-check
  worker are now their own modules.
- All third-party GitHub Action references pinned to commit SHAs.
- Apple Team ID is now sourced from `Appfile` (canonical) rather
  than duplicated in Fastfile.
- CI runs macOS-only — Ubuntu lanes dropped since the shipped
  product is Mac App Store.

### Fixed
- `PerformanceTracker` now joins its worker thread on stop and
  drop, matching the join-on-drop pattern used elsewhere in the
  codebase.
- `is_silent` doc rewritten to describe the actual two-stage
  algorithm (peak fast-path, RMS fallback) rather than only RMS.
- `audio_processor.rs` trait doc-comment relocated off the `use`
  statements so rustdoc actually attaches it to `AudioProcessor`.
- README test-count and CI-job claims re-derived from reality after
  drift.

### Removed
- Dead-stub `bin/macos/MenuBarApp` module — leftover scaffolding
  from before the SwiftUI app existed; the actual menu-bar UI was
  always in `BlackBoxApp/`. Drops the `--menu-bar` CLI flag and
  the `menu-bar` Cargo feature.
- Ad-hoc-signed `.app.zip` artifact from the public GitHub Release
  — Gatekeeper-rejected for downloaders. TestFlight + Mac App
  Store remain the canonical distribution channels; the GitHub
  Release ships the CLI binary only.
- Orphaned mid-sentence doc comment in
  `tests/cpal_integration_tests.rs`, leftover from DOLL-118.

## [1.1.0] — 2026-05-07

Reliability and polish across the recording engine. Fifty-three tickets
shipped across atomic-ordering coherence, FFI safety, panic/alloc audit,
numerical correctness, API surface narrowing, test rigor, and binary
size. No breaking changes for end users.

### Added
- Menu bar icon pulses while recording so capture-live state is visible
  at a glance (matches Audio Hijack / QuickTime convention).
- Typed `BlackBoxError` propagation across the FFI boundary — the Swift
  bridge now returns `Result<Int, BlackBoxError>` for `fillPeakLevels`
  and `getDeviceChannelCount` instead of collapsing failures to `0`/`nil`.
- Distinct FFI error codes: `BLACKBOX_ERR_INVALID_ARG` (-8) for null
  arguments, separate from `BLACKBOX_ERR_INVALID_HANDLE` (-1).
- Real-time throughput assertion in the CI benchmark smoke test (≥10×
  real-time floor) — silent perf regressions can no longer ship.
- Weekly CI lane that runs the `#[ignore]`-d benchmark/perf test suite.
- `scripts/check-versions.sh` release pre-flight verifying `Cargo.toml`,
  `project.yml`, `Info.plist`, and `Makefile` are aligned.
- `scripts/lint-app-store-metadata.py` validates `age_rating_config.json`
  and metadata text-field lengths against Apple's current OpenAPI spec
  before deploy.
- Deterministic mock-clock helper for writer-thread rotation tests so
  tests don't depend on `thread::sleep` rendezvous.

### Changed
- Faster app launch — `@MainActor` filesystem I/O moved off the launch
  path; the menu bar surfaces interactive immediately.
- Smoother peak meter — `peakLevels` array no longer reallocates every
  tick once the channel count is known.
- Hardened f32→int audio sample conversion: NaN/Inf no longer become
  max-amplitude clicks; out-of-range samples clamp before rounding.
- Status-flag reads lifted out from behind the recorder mutex; the
  Swift status poll no longer blocks on multi-second device probes.
- Hot-path channel bounds-check hoisted out of `write_samples` inner
  loop into a per-batch pre-filter.
- Shipped binary is **~26% smaller** — release profile now uses
  `panic = "abort"` + `strip = true` and `benchmarking` is no longer
  a default feature (drops `sysinfo` from the App Store build).
- Single dedicated `SilenceCheckWorker` replaces per-rotation
  `thread::spawn`; bounded channel for backpressure, joined on Drop.
- `OutputMode` enum threaded through `AudioProcessor` trait — eliminated
  stringly-typed flow at config-load and per-rotation paths.
- All third-party GitHub Actions pinned to commit SHAs.
- `fastlane` pinned via `BlackBoxApp/Gemfile.lock` with `bundler-cache`
  in CI; updates flow through Dependabot.
- Toolchain bumped to Rust 1.95.

### Fixed
- First-run onboarding now saves a security-scoped bookmark when the
  user accepts the auto-populated default folder (previously dropped
  the bookmark, surfacing later as "Output Directory Unavailable").
- Auto-record-on-launch notification request happens at init, not
  lazily on first manual record — fixes the dropped first-run banner.
- Onboarding window no longer clips on the recording-mode step;
  re-opens to the foreground when re-invoked.
- `finalize` and `stop_monitoring` clear `sample_rate_changed` so
  status flags don't carry stale state across recordings.
- NaN samples no longer block the silence-gate from opening (the
  prior `>` comparison returned false on NaN, dropping recordings
  indefinitely).
- WAV-header byte counts use saturating arithmetic so a corrupted
  size field can't wrap silently.

### Removed
- Dead-stub `bin/macos/MenuBarApp` module — the actual menu-bar UI
  lives in the SwiftUI app, never in the CLI binary.
- `--menu-bar` CLI flag and the `menu-bar` Cargo feature flag.
- `catch_unwind` wrappers in the FFI layer; release builds now use
  `panic = "abort"` and the panic policy is documented.

### Security
- Dependabot weekly checks now cover Cargo, GitHub Actions, **and**
  Bundler (fastlane and transitive Ruby gems). Six dependency bumps
  merged this cycle (sysinfo 0.39, rtrb 0.3.4, libc 0.2.186,
  env_logger 0.11.10, toml 1.0.6+spec-1.1.0, tempfile 3.27.0).
- Floating GitHub Action major-tag references replaced with full
  commit SHAs to neutralize tag-move attacks against CI runners that
  hold App Store Connect API keys and signing certificates.

## [1.0.2] — 2026-03-06

### Fixed
- Fastlane CI provisioning profile download.
- Apple Generic Versioning enabled in xcodeproj.
- `testListInputDevices` skipped on CI runners without audio hardware
  via timeout instead of env var (which xcodebuild test host doesn't
  inherit).
- Eliminated redundant CI runs on tag pushes.

## [1.0.1] — 2026-03-06

Initial Mac App Store release. CLI binary plus SwiftUI menu-bar app.

### Added
- Continuous-mode and single-shot audio recording to local WAV files.
- Multichannel input device support.
- Lock-free real-time recording pipeline (RT callback → ring buffer →
  writer thread).
- Silence-gate auto-rotation.
- Live level meter with peak indicators.
- macOS menu-bar UI (LSUIElement) with status, level meter, and
  settings panes.
- Privacy-respecting design: no network access, all recordings stay
  local.

[Unreleased]: https://github.com/tibbon/audio_blackbox/compare/v1.4.0...HEAD
[1.4.0]: https://github.com/tibbon/audio_blackbox/compare/v1.3.0...v1.4.0
[1.3.0]: https://github.com/tibbon/audio_blackbox/compare/v1.2.0...v1.3.0
[1.2.0]: https://github.com/tibbon/audio_blackbox/compare/v1.1.0...v1.2.0
[1.1.0]: https://github.com/tibbon/audio_blackbox/compare/v1.0.2...v1.1.0
[1.0.2]: https://github.com/tibbon/audio_blackbox/compare/v1.0.1...v1.0.2
[1.0.1]: https://github.com/tibbon/audio_blackbox/releases/tag/v1.0.1
