# Changelog

All notable changes to BlackBox Audio Recorder are documented in this file.

The format is based on [Keep a Changelog 1.1.0](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

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

[Unreleased]: https://github.com/tibbon/audio_blackbox/compare/v1.1.0...HEAD
[1.1.0]: https://github.com/tibbon/audio_blackbox/compare/v1.0.2...v1.1.0
[1.0.2]: https://github.com/tibbon/audio_blackbox/compare/v1.0.1...v1.0.2
[1.0.1]: https://github.com/tibbon/audio_blackbox/releases/tag/v1.0.1
