# Decision log

Short summaries of the load-bearing DOLL tickets referenced from comments and docs. Reading the actual Linear ticket is still the authoritative source — this is a map so a contributor without Linear access (or after archive) can still navigate.

Tickets are listed by ID; add new entries as you reference them in code or docs. Format: one paragraph, "why" not "what".

## Architecture invariants

- **DOLL-90** — Release builds use `panic = "abort"`. Any panic in production is a bug we want surfaced via crash report, not unwound across the FFI boundary (UB on Apple Silicon; trips macOS watchdog). Do not add `catch_unwind` wrappers in `src/ffi.rs`.

- **DOLL-101** — Atomic ordering doctrine: payload-carrying flags (e.g. `recording_active` paired with `sample_rate_atomic`) use Acquire/Release. Status-only flags (`gate_idle`, `disk_space_low`, `stream_error`, `sample_rate_changed`, the ctrlc shutdown flag) use Relaxed. See `ARCHITECTURE.md § Atomic ordering`.

- **DOLL-124** — FFI lock-acquisition order: `recorder` is outermost; the other inner mutexes (`config`, `last_error`, `peak_levels`, `status`) are taken alone, never nested with each other. Documented in `src/ffi.rs:84-97`.

- **DOLL-147** — `cpal_processor.rs` and `writer_thread.rs` split into focused modules. The CoreAudio sample-rate listener and silence-check worker became their own files.

## FFI / boundary

- **DOLL-125** — `BlackBoxError` propagation across the FFI boundary. Swift bridge now returns typed errors instead of collapsing failures to `0`/`nil`.
- **DOLL-127** — Anti-revert anchor: concurrent FFI status reads test (writers flip flags under config-mutex contention) proves the lock-free claim.

## Swift app shell

- **DOLL-114** — Bookmark restoration deferred off the launch path via a background `Task`. The synchronous URL+startAccessingSecurityScopedResource+setConfig chain hit disk / IPC and delayed first menu-bar appearance.
- **DOLL-134** — Notification authorization is requested eagerly at init, not lazily on first manual record — fixes the dropped first-run auto-record banner.
- **DOLL-148** — SwiftUI `Settings` scene instead of a generic `Window` for the settings UI. `⌘,` opens it from anywhere; close/reopen semantics inherit from the platform.
- **DOLL-155** — ASC `.p8` API key moved out of repo root to `~/Library/Application Support/com.dollhousemediatech.blackbox/keys/`. One git-add-f away from leaking otherwise.
- **DOLL-157** — `GlobalHotkeyManager.register` surfaces OSStatus failures to the user (Settings tab) instead of silently logging.
- **DOLL-161** — `GlobalHotkeyManager` marked `@MainActor`; Carbon callback uses `MainActor.assumeIsolated` (Carbon delivers on main run loop).
- **DOLL-181** — Auto-record on launch awaits the bookmark-restore Task before calling `start()`. Without this, slow restores had auto-record writing to the sandbox default dir.
- **DOLL-182** — `wasSleepInterrupted` is cleared in `stop()`. Manual stop within the 1.5s deferred-resume window would otherwise let the resume Task resurrect a recording the user explicitly stopped.
- **DOLL-183** — `willPowerOff` drains the recording directly (instead of waiting for `applicationShouldTerminate`) so a ~5s shutdown grace doesn't kill us mid-finalize.

## UX / a11y

- **DOLL-141** — Onboarding step-indicator dots are per-step Buttons (VO users can step back through completed steps).
- **DOLL-142** — Level-meter VoiceOver value bucketed to threshold crossings (Silent / Low / Moderate / Hot / Clipping) — no per-tick dB flood.
- **DOLL-164** — Global Shortcut row in Settings: `accessibilityElement(.combine)` is scoped to the label + recorder; the Clear button stays as a sibling so VO users can still focus it.
- **DOLL-165** — Multichannel warning + shortcut error captions are `Label { Text } icon: { Image }`; system glyphs are `.accessibilityHidden(true)` to defend against macOS 14 locales that announce SF Symbol names.

## CI / build

- **DOLL-131** — Long benchmark / perf tests are `#[ignore]`'d; weekly `ignored-tests.yml` workflow runs them.
- **DOLL-138** — All third-party GitHub Action references pinned to commit SHAs.
- **DOLL-153** — CI is macOS-only; Ubuntu lanes dropped (Mac App Store is the shipped product).
- **DOLL-154** — Fastlane derives next build number from `latest_testflight_build_number` instead of just the committed pbxproj — kills CFBundleVersion-collision drift.
- **DOLL-160** — Pbxproj/project.yml drift CI check (parked pending design call on fastlane's release-time pbxproj mutations).
- **DOLL-166** — Cargo cache keys consolidated across CI lanes (one shared `cargo-` key for all stable-toolchain jobs; MSRV stays separate).
- **DOLL-180** — Cyberclaw-review aggregate fixes (a11y regression on shortcut Clear button + 12 minors).

## Cleanup / drift fixes

- **DOLL-170** — Two duplicate inline tests dropped from `src/lib.rs` (`test_silence_deletion`, `test_channel_parsing`).
- **DOLL-189** — `BlackboxError::Config(_)` variant deleted; zero production producers. Config validation is forgiving.
- **DOLL-191** — ARCHITECTURE.md silence-worker "never blocks" claim corrected (the channel is bounded at 8; back-pressure is possible under sustained pressure).

This list is intentionally non-exhaustive — only the load-bearing decisions are here. If you add a comment that references a DOLL number not on this list, consider whether the decision is worth surfacing here too.
