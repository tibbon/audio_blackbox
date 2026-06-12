# Architecture

The contract behind BlackBox's recording pipeline. Read this before changing anything in the audio path, the FFI boundary, or the writer/silence-worker threads.

## Pipeline

```
+--------------------+   +--------------+   +-------------+   +------------+
| CoreAudio device   |   | RT callback  |   |  Writer     |   | WAV files  |
| (cpal stream)      |-->| (RT thread)  |-->|  thread     |-->| on disk    |
+--------------------+   +--------------+   +-------------+   +------------+
                              |                    |
                              | rtrb SPSC ring     | bounded mpsc
                              | buffer (5 sec)     v
                              |              +-------------+
                              |              | Silence-    |
                              |              | check worker|
                              |              | (1 thread)  |
                              v              +-------------+
                         +-----------+
                         | atomic    |
                         | flags     |  recording_active, gate_idle,
                         | (status)  |  rotation_needed, sample_rate_changed
                         +-----------+
```

The RT thread never blocks on I/O, locks, or allocations. The writer thread does all WAV encoding, peak metering, file rotation, and disk-space checks. The silence-check worker is a single dedicated thread that finalize() drains via join-on-drop.

## Threading rules

### RT (audio callback) thread

`CpalAudioProcessor` registers a cpal stream callback. That callback runs on a CoreAudio dispatch queue with hard latency requirements. Only safe operations:

- **Push samples** into the `rtrb` SPSC ring buffer. The buffer is sized for `RING_BUFFER_SECONDS = 5` (`src/constants.rs`) of audio at the device's rate × channel count, providing runway for stalls in the writer.
- **Atomic loads / stores** with Relaxed or Release ordering. No allocator calls, no mutex acquisition, no syscalls.

A test-time `CountingAllocator` (`mod alloc_counter` in `src/lib.rs`) wraps the system allocator with `AtomicU64::fetch_add`, and `src/tests/alloc_tests.rs` (no top-level `tests/` directory; all tests live under `src/tests/` to share `pub(crate)` access) asserts the hot path produces zero allocations.

### Writer thread

`writer_thread::run` is spawned in `process_audio` and joined when `finalize()` is called. Responsibilities:

- Drain the ring buffer, convert f32 to the configured bit depth, write WAV via `RawWavWriter` (a hand-rolled writer; we don't drag `hound` into the hot path).
- Maintain per-channel peak levels in cache-aligned `AtomicI32` slots — read by the FFI 30 Hz meter poll.
- Rotate files when the RT thread sets the `rotation_needed` flag (a Relaxed status flag — DOLL-391; the samples it implies are already synchronized through the rtrb ring, so no Acquire/Release pairing is needed). See `CpalAudioProcessor::process_audio_impl` (the store) and `writer_thread_main` (the `swap`).
- Submit rotated files to the silence-check worker over a bounded `mpsc::sync_channel` (capacity 8). Back-pressures the writer thread if the silence checker can't keep up — acceptable trade-off for bounded memory. In practice unreachable under normal rotation cadence (rotation is ≥ 60 s; silence checks complete in milliseconds for normal-size files, so 8-deep buffering is ample).
- Monitor disk space and flip `disk_space_low` when the configured `min_disk_space_mb` precondition fails.

### Silence-check worker

`silence_check_worker::SilenceCheckWorker` is a single thread fed via a bounded `mpsc::sync_channel`. Its `Drop` impl closes the sending side and joins the worker — guaranteeing every queued file is processed before `finalize()` returns. Don't call `mem::forget` on it.

### Sample-rate listener (macOS)

`macos_sample_rate_listener::SampleRateListener` registers a CoreAudio property listener for sample-rate changes on the active device. The `client_data` is `Arc::into_raw(Arc::clone(&flag))` — the listener owns one strong refcount of an `Arc<AtomicBool>`.

`Drop` **deliberately leaks** the strong reference (skips `Arc::from_raw`) rather than reclaiming it. Apple's docs do not guarantee that `AudioObjectRemovePropertyListener` blocks until in-flight callbacks on other threads have returned — only that no *new* callbacks will start. Leaking eliminates the race entirely; the cost is one `AtomicBool` (1 byte) per recording session for the process lifetime, bounded.

If you "fix" this by adding `Arc::from_raw` to Drop, you reintroduce a use-after-free that only fires under sample-rate-change-during-listener-removal — extremely rare, hard to reproduce, exactly the kind of bug we're refusing to ship.

## Lock acquisition order (FFI)

Copied verbatim from the canonical comment block on `BlackboxHandle` in `src/ffi.rs`. Any future code path that takes two of the inner mutexes simultaneously must add itself to this order.

1. `recorder` — outermost. Held across multi-second device probing (`CpalAudioProcessor::with_config`, `recorder.start_recording()`).
2. The remaining mutexes (`config`, `last_error`, `peak_levels`, `status`) are taken **alone**, never nested with each other. Each is acquired, mutated, and released in a brief critical section. Inside an `extern "C"` body that holds `recorder`, these inner locks are taken sequentially and dropped between acquisitions.

## FFI panic policy

Release builds set `panic = "abort"` (`Cargo.toml [profile.release]`, established in DOLL-90). Any panic in production is a bug we want to surface via crash report — *not* unwind across the FFI boundary, where stack-unwinding through `extern "C"` is undefined behavior on Apple Silicon and trips the watchdog on macOS.

Do not add `catch_unwind` wrappers in `src/ffi.rs`. Do not flip release builds back to `panic = "unwind"`. The Mac App Store crash report dashboard is the place where these surface.

## Atomic ordering

The codebase has two flavors of atomic flag:

- **Synchronizing flags** — Acquire/Release pairs that publish or observe a *payload* held in another atomic. Example: the `recording_active.store(true, Ordering::Release)` in `CpalAudioProcessor::start_recording` synchronizes-with the FFI status poll's Acquire load, so a reader that sees `recording_active = true` is also guaranteed to see the matching `sample_rate` written before the Release (DOLL-101).
- **Status-only flags** — single-bit signals with no synchronizes-with relationship. `gate_idle`, `disk_space_low`, `stream_error`, `sample_rate_changed`, `rotation_needed` (DOLL-391), the ctrlc-handler shutdown flag in `bin/main.rs`. All Relaxed pairs. The only correctness requirement is "the value is eventually visible," which Relaxed satisfies.

If you're adding a new atomic flag: ask whether a reader observing this flag's set state needs to also observe other state set by the same writer. If yes → Acquire/Release. If no → Relaxed.

## Platform support

The app is **Apple-Silicon-only by decision** (DOLL-463, 2026-06): `ARCHS` is pinned to `arm64` in `project.yml`, the Fastfile, and the Makefile's xcodebuild flags. The last Intel Macs are aging out of macOS support, and an x86_64 lane would roughly double the Rust build cost in CI (scarce Actions minutes) for a shrinking audience. If Intel support is ever wanted: build the Rust lib for both targets, `lipo` them, point `LIBRARY_SEARCH_PATHS` at the fat lib, and drop the three `ARCHS` pins. (A dead `rust-lib-universal` Makefile target that did the lipo step — but that nothing consumed — was removed as part of this decision.)

## Swift app shell

The Mac App Store-shipped product is a SwiftUI menu-bar app (`BlackBoxApp/BlackBoxApp/`). The Rust engine is consumed via the FFI surface in `src/ffi.rs`; everything below is Swift-side.

### MenuBarExtra + Window-scene termination

`AppDelegate` (in `BlackBoxApp.swift`) handles a known SwiftUI quirk: closing the last `Window` scene fires `applicationShouldTerminate`. The delegate returns `.terminateCancel` unless `explicitQuit == true`, so the app stays alive while keeping its menu bar item. Explicit Quit (menu bar, system shutdown) sets the flag then calls `terminate(nil)`.

### Sleep / wake matrix

`SleepWakePolicy` is a pure-logic enum + static methods (extracted for unit testing — the live `@MainActor` handlers are awkward to test directly). The decisions:

| Event | `isRecording` = true | Action |
|-------|--------------------|--------|
| `willSleep` (behavior=resume) | yes | `.pauseForResume` → stop + mark `wasSleepInterrupted = true` |
| `willSleep` (behavior=stop) | yes | `.stop` → stop, do not mark |
| `willSleep` | no | `.ignore` |
| `didWake` | `wasSleepInterrupted` set | deferred `Task.sleep(1500ms) → start()` |
| `sessionDidResignActive` | yes | `.pauseForResume` (always; fast-user-switch / screen-saver is recoverable) |
| `sessionDidBecomeActive` | `wasSleepInterrupted` set | deferred `start()`, same as `didWake` |
| `willPowerOff` | any | drain immediately via `recorder?.stop()` (DOLL-183) |

`wasSleepInterrupted` is cleared by `didWake`, `sessionDidBecomeActive`, AND `stop(reason: .user)` (DOLL-182 — otherwise a manual stop within the 1.5s deferred-resume window would let the deferred Task resurrect the recording). The willSleep / sessionResign handlers stop with `reason: .sleepInterruption`, which preserves the flag they just set — `stop()` clearing it unconditionally made resume-on-wake dead code (DOLL-442).

### Security-scoped bookmark lifecycle

The user-picked output directory is persisted as a security-scoped bookmark in UserDefaults. Lifecycle:

1. **Save**: in `OnboardingView` / `SettingsView` directory-picker; `URL.bookmarkData(options: .withSecurityScope)`.
2. **Restore on launch**: a deferred `Task` (`bookmarkRestoreTask`, DOLL-114) resolves the bookmark, calls `startAccessingSecurityScopedResource`, and pushes the path into the Rust engine. Auto-record waits on this Task (DOLL-181).
3. **Hold during runtime**: the URL stays scoped from restore until quit.
4. **Release on quit**: `applicationShouldTerminate` calls `releaseOutputDirAccess` after `stop()`.

A stale bookmark (folder deleted, volume unmounted) prompts the user to re-pick via `promptToReselectOutputDir`.

### Carbon hotkey lifecycle

`GlobalHotkeyManager` is a `@MainActor` singleton wrapping the Carbon Event Hot Key API. `Shortcut` is `Codable` (persisted to UserDefaults under `globalShortcut`). The C callback uses `MainActor.assumeIsolated` since Carbon delivers hotkey events on the main run loop after `InstallEventHandler` is called from main (DOLL-161). Registration surfaces failures to the user — both at Settings-time (DOLL-157) and at launch-restoration (DOLL-184).

### Notification authorization (DOLL-134, DOLL-185)

`UNUserNotificationCenter` authorization is requested eagerly at init so the very-first auto-record-on-launch notification isn't dropped. The granted bool is captured into `notificationsAuthorized` and re-checked on `NSApplication.didBecomeActiveNotification` — granting in System Settings is picked up without a relaunch.

### Meter polling cadence

`RecordingState.isMeterWindowOpen` drives the meter Task. When the window is open and the engine is recording or monitoring, a Task polls `bridge.fillPeakLevels(into:)` at ~30 Hz. The Task is paused / cancelled when the window closes — no FFI calls happen with a closed meter.

### `@Observable RecordingState` pattern

`RecordingState` is `@MainActor`-isolated and `@Observable` (Swift macro). It's passed by value into views (not via `@Environment`); SwiftUI's observation system propagates change notifications. View-model mutation off-main is a compile error because of `@MainActor`.

## Module map

| Module | Role |
|--------|------|
| `src/audio_processor.rs` | `AudioProcessor` trait — central abstraction over real (cpal) and mock processors. |
| `src/audio_recorder.rs` | High-level driver wrapping a processor + config. |
| `src/cpal_processor.rs` | Real audio I/O via cpal; spawns the writer thread. |
| `src/writer_thread.rs` | Writer-thread loop, ring-buffer consumer, WAV file management, peak metering. |
| `src/silence_check_worker.rs` | Single-thread post-rotation silence checker with join-on-drop. |
| `src/macos_sample_rate_listener.rs` | CoreAudio property listener (macOS only). |
| `src/raw_wav_writer.rs` | Hand-rolled WAV writer for the hot path. |
| `src/ffi.rs` | C ABI consumed by the SwiftUI app. Owns the canonical lock order. |
| `src/config.rs` | TOML + `BLACKBOX_*` env-var configuration; env vars take precedence. |
| `src/error.rs` | Typed error enum (`BlackboxError`) with `thiserror`. |
| `BlackBoxApp/` | SwiftUI menu-bar app; calls Rust via FFI. |

See `README.md` for the user-facing feature list and benchmark numbers.
