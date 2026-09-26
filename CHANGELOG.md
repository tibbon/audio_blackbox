# Changelog

All notable changes to BlackBox Audio Recorder are documented in this file.

The format is based on [Keep a Changelog 1.1.0](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Changed
- App Store description: claims now match the engine. A crash costs up to
  ~10 s of audio rather than "always valid" files; the ring buffer absorbs
  disk stalls rather than guaranteeing "no dropped samples"; channel support
  is "tested to 64, up to 255". Adds sleep/wake resume to the feature list.
- Repo ships `blackbox.example.toml` instead of a tracked `blackbox.toml`
  (now gitignored); a test keeps the example in sync with the defaults.
- 24/32-bit and multichannel WAVs are written with a `WAVE_FORMAT_EXTENSIBLE`
  header, which strict readers expect for those formats. 16-bit mono and
  stereo files keep the classic 44-byte header.
- In continuous mode with the silence gate on, the rotation clock restarts
  when the gate opens, so the first file after a gate open is a full cadence.
- The recording caption in the menu reads "2 channels" instead of "2 ch",
  and the menu's remaining strings are localizable.
- The CLI warns about unknown keys in `blackbox.toml`, and rejects a zero
  silence-gate timeout or a rotation cadence long enough to overflow.
- Split-mode files number channels from 1 to match the app: the first input
  channel's file is `...-ch1.wav` (was `...-ch0.wav`). The number is still
  the device channel, so recording channels 2 and 4 writes `-ch2` and `-ch4`.
  CLI `audio_channels` stays 0-based. Existing files are not renamed.
- Onboarding asks before setting a global shortcut instead of claiming ⌘⇧R
  (the browsers' hard reload) on its own. No shortcut is set unless you
  click "Use ⇧⌘R" or record one; a shortcut you already saved is kept.
- Changing continuous mode, the rotation interval, minimum free space,
  silence detection or its threshold, or auto-split or its timeout while
  recording now asks to restart the recording, as bit depth and channels
  already did. Before, the change silently waited for the next session.
- Choosing a new output folder while recording restarts the recording in
  the new folder. Before, the engine kept writing to the old folder after
  the app had given up access to it.
- "Run Setup Again" is disabled while recording, and re-running setup starts
  from your saved recording mode and rotation interval instead of the
  first-run defaults. Skip keeps a folder you already picked.
- The pre-flight warning for long takes says files are split at 4 GB, not
  truncated.
- The CLI exits with a non-zero status when it fails, and stops (finalizing
  its files) as soon as the engine reports a full disk, a write failure or a
  stream error, instead of recording nothing until the timer or Ctrl-C.
- The CLI creates a default `blackbox.toml` only when no config file exists,
  at `BLACKBOX_CONFIG` if that is set, and logs the file it actually loaded.
  In continuous mode it logs the duration as "unlimited".
- The CLI refuses to start when the configured input device isn't present,
  instead of recording from the default microphone. One invalid value in
  `blackbox.toml` now skips just that key, not the whole file, and an empty
  `BLACKBOX_CONFIG` counts as unset.
- The 4 GB file-size warning is a notification, never a dialog, and appears
  after recording has started.
- Recordings are synced to disk at each 10 s header refresh, so a power cut
  loses at most about 10 s.

### Added
- Crash recovery: recordings a crash or power cut left as `.recording.wav`
  are repaired and renamed to ordinary WAVs at the next launch (app and CLI).
- The input device list refreshes when a device is plugged in or removed.
  The menu shows the device a recording actually uses, and the pre-flight
  summary says when the chosen device is not connected.

### Fixed
- Rotation boundaries no longer drift over long sessions.
- A full disk at sample rates above 48 kHz is reported as a write failure,
  not as heavy load.
- The sample-rate listener watches the device actually recording, even if
  the default input changes during start.
- The 30 s and 1 min rotation presets reopen as themselves, not "Custom".
- The 4 GB file-size warning no longer repeats when a recording restarts
  internally.
- Quitting from Activity Monitor, an installer or AppleScript finalizes the
  recording and quits, instead of being cancelled.
- The CLI exits promptly with performance logging on.
- Every recording mode starts a new file before a WAV reaches its 4 GB size
  limit. A single long take (continuous mode off) used to grow past it, and
  players then read only the first 4 GB.
- The CLI finalizes its recording on SIGTERM and SIGHUP (`kill`, a service
  manager stop, a closed terminal), not only on Ctrl-C.
- When the ring buffer overflows, only whole frames are kept. A partial frame
  used to shift every later sample onto the wrong channel for the rest of the
  session.
- If the silence gate cannot open its files, recording stops and you are
  notified, instead of showing "Recording" while nothing reached disk.
- A file whose finalize failed (for example on a full disk) is no longer
  mistaken for silence and deleted, and its audio stays readable.
- Stopping no longer waits for queued silence checks, which could hang the
  app for minutes after a long, mostly silent multichannel session.
- The silence gate's pre-roll is written in order, including when it wraps
  the ring buffer, so the start of a take is no longer shifted or lost.
- Level meters hold each peak until the display reads it, so clips and short
  transients show up.
- Meter bars are labelled with the device channels being recorded:
  recording channels 3 and 4 shows "Ch 3" and "Ch 4", not "Ch 1" and "Ch 2".
- Logout, restart and shutdown are no longer blocked while recording, and
  going to sleep finalizes the recording before the Mac sleeps.
- Stopping or starting a recording right after waking cancels the automatic
  resume, so a manual stop is no longer undone.
- A stop that failed while finalizing no longer leaves the menu showing
  "Recording" forever.
- The "Restart Recording" notification action starts recording again (it
  did nothing).
- When an automatic restart after a sample-rate change fails, the
  notification says recording stopped instead of "restarted".
- A shortcut change that fails (because another app owns the combination)
  keeps the old shortcut working.
- Closing the meter window while the microphone prompt is up no longer
  leaves the microphone in use.
- Refreshing an outdated output-folder bookmark keeps access to the folder.
- Auto-record waits for you to re-pick an output folder the app can no
  longer reach, and cancelling that picker falls back to the app's own
  folder instead of an unwritable one.
- A "Disabled" minimum free space setting survives a relaunch; the app used
  to fall back to 500 MB and stop on a check you had turned off.
- The channel range field is checked before it is saved, so a half-typed
  range can no longer make every later recording fail to start.
- The menu's warning icons (errors, dropped samples, low battery, 4 GB) show
  on macOS 27.
- A disk write error that clears no longer shifts channels for the rest of
  the file; in split mode, frames a channel file missed are filled with
  silence so the files stay aligned.
- Crash recovery skips files another running recorder still has open, and
  cleans up empty leftovers instead of warning about them at every launch.
- Stopping right as the silence gate opens keeps the start of the take, the
  shutdown drain respects the 4 GB limit, and a rotation that coincides with
  a gate close no longer leaves an empty WAV.
- A new recording never truncates an existing temp file with the same name.
- A sample-rate change during start is detected instead of producing a file
  that plays at the wrong speed.
- A restart that can't stop the engine keeps the live recording controllable
  instead of showing it as idle.
- Finishing onboarding while recording applies the chosen recording mode to
  the live session.
- The 4 GB estimate after a sample-rate change uses the new rate.
- The menu names the device the recording opened, even after devices change.
- Picking a device from the menu with Settings open restarts once, not twice.
- Recording a new shortcut no longer triggers the current one, and the
  shortcut button can't get stuck on "Press shortcut…".
- Counted messages ("1 sample dropped", recovered recordings, rotation
  intervals) use proper plural forms.
- The debug logging toggle takes effect without a relaunch.
- A second Ctrl-C ends the CLI's wait for silence checks.

## [1.5.0] — 2026-09-21

A maintenance and hardening release: ~70 commits of internal refactoring,
stricter lint policy, and dependency/security updates on top of 1.4.0. No
user-facing feature changes and no breaking changes.

### Changed
- Swift app restructured for readability: `RecordingState` split into
  per-concern extensions; `SettingsView`, the onboarding flow, the menu
  sections, `AppDelegate` and `SleepWakePolicy` each moved into their own
  files; `MeterView`/`MeterBar` and the settings tabs broken into section
  properties (DOLL-653).
- Adopted a strict Rust and Swift lint policy — Swift 6 strict concurrency,
  `swift-format` + SwiftLint (with `closure_body_length`), and a tightened
  clippy configuration (complexity budgets ratcheted to the tree's floor;
  bans on NaN-unsafe float `partial_cmp` and process-wide env mutation).
- Unified local, pre-commit and CI checks into a single guardrail loop
  (DOLL-652); added the `/ship-ticket` workflow and reviewer briefs
  (DOLL-654).
- Raised the minimum supported Rust version to 1.98 and moved the release
  pipeline to Ruby 4.0 (DOLL-657).

### Fixed
- bench-writer single mode declared a 16-bit WAV header but wrote 24-bit
  samples; `percent_of` is now exact and rounds to the nearest basis point.

### Security
- Updated every dependency to its latest release, clearing RUSTSEC-2026-0190
  (anyhow) and RUSTSEC-2026-0274 (rtrb); bumped fastlane to 2.240.0 and
  refreshed all pinned GitHub Actions.

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
