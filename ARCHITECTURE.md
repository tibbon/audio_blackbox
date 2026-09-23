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

A test-time `CountingAllocator` (`mod alloc_counter` in `src/lib.rs`) wraps the system allocator with `AtomicU64::fetch_add`, and `src/tests/alloc_tests.rs` (no top-level `tests/` directory; all tests live under `src/tests/` to share `pub(crate)` access) asserts that the writer's steady-state `write_samples` path allocates nothing. Those tests are `#[ignore]` and run only in the weekly ignored-tests lane. Nothing measures the cpal callback itself; it is allocation-free by inspection (`push_samples_with_overflow_count` is one `push_partial_slice` plus an atomic add).

### Writer thread

`writer_thread::writer_thread_main` is spawned in `process_audio` and joined when `finalize()` is called. Responsibilities:

- Drain the ring buffer, convert f32 to the configured bit depth, write WAV via `RawWavWriter` (a hand-rolled writer; we don't drag `hound` into the hot path).
- Maintain per-channel peak levels in cache-aligned `AtomicU32` slots — read by the FFI 30 Hz meter poll.
- Rotate files when the RT thread sets the `rotation_needed` flag (a Relaxed status flag — DOLL-391; the samples it implies are already synchronized through the rtrb ring, so no Acquire/Release pairing is needed). See `CpalAudioProcessor::process_audio_impl` (the store) and `writer_thread_main` (the `swap`).
- Submit rotated files to the silence-check worker over a bounded `mpsc::sync_channel` (capacity 8). Back-pressures the writer thread if the silence checker can't keep up — acceptable trade-off for bounded memory. It can fill: rotation can be as short as 1 s, and `is_silent` decodes a silent file to the end, so checking a long silent file takes seconds to minutes.
- Monitor disk space and flip `disk_space_low` when the configured `min_disk_space_mb` precondition fails.
- Keep files crash-readable; see [Recording file lifecycle](#recording-file-lifecycle).

### Silence-check worker

`silence_check_worker::SilenceCheckWorker` is a single thread fed via a bounded `mpsc::sync_channel`. Its `Drop` impl closes the sending side and joins the worker — guaranteeing every queued file is processed before `finalize()` returns. Don't call `mem::forget` on it.

That join has no timeout. `CpalAudioProcessor::finalize` bounds the writer's *reply* at 30 s, but its `join()` then waits for the writer to drop its state, which waits for every queued scan. Stopping after a long, mostly silent session therefore blocks the caller (the Swift main thread, via `blackbox_stop_recording`) until the scans finish.

### Sample-rate listener (macOS)

`macos_sample_rate_listener::SampleRateListener` registers a CoreAudio property listener for sample-rate changes on the active device. The `client_data` is `Arc::into_raw(Arc::clone(&flag))` — the listener owns one strong refcount of an `Arc<AtomicBool>`.

`Drop` **deliberately leaks** the strong reference rather than reclaiming it: it rebuilds the `Arc` with `Arc::from_raw` inside a `ManuallyDrop`, so the refcount is never decremented. Apple's docs do not guarantee that `AudioObjectRemovePropertyListener` blocks until in-flight callbacks on other threads have returned — only that no *new* callbacks will start. Leaking eliminates the race entirely; the cost is one `Arc<AtomicBool>` allocation (about 24 bytes) per recording session for the process lifetime, bounded.

If you "fix" this by letting that `Arc` drop, you reintroduce a use-after-free that only fires under sample-rate-change-during-listener-removal — extremely rare, hard to reproduce, exactly the kind of bug we're refusing to ship.

## Recording file lifecycle

This is how the product keeps its "don't lose a take" promise. All of it runs on the writer thread.

1. **Open.** Each file is created as `<output_dir>/<YYYY-MM-DD-HH-MM-SS>[-chN].recording.wav` with a placeholder header. `-chN` appears only in split mode and is the 0-based device channel. `disambiguate_path` appends `-1`, `-2`, … when the *final* `.wav` name already exists, so a finished recording is never overwritten.
2. **Write.** `RawWavWriter` buffers through a `BufWriter`. Every 10 s of audio (`flush_writers`, `sample_rate * 10` frames) it flushes and rewrites the RIFF and data sizes, so the file on disk is always a readable WAV up to the last refresh. A hard kill or power cut loses up to about 10 s and leaves the file under its `.recording.wav` name; nothing renames or recovers those files on the next launch.
3. **Rotate or close.** Rotation (continuous mode), a silence-gate close, stop, and shutdown all call `finalize_all`. It finalizes every writer (flush, RIFF pad byte, header), then renames every pending `.recording.wav` to its `.wav` name. A failure on one file does not stop the others (DOLL-345); the first error is returned.
4. **Silence check.** When `silence_threshold > 0`, renamed files go to the silence-check worker, which deletes any file whose peak and RMS stay below the threshold. `is_silent` treats a file with zero samples as silent.
5. **Limits.** WAV sizes are `u32`, so a file past 4 GiB keeps growing on disk while its header saturates at `u32::MAX` (DOLL-204). Nothing rotates on size; only the cadence and the gate end a file.

Two consequences of steps 3 and 4 to keep in mind when changing this code: a file whose `finalize` failed (for example on a full disk) is still renamed and still submitted to the silence check, and if its header never got past the placeholder, the check sees zero samples and deletes it.

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

`AppDelegate` (in `AppDelegate.swift`) handles a known SwiftUI quirk: closing the last `Window` scene fires `applicationShouldTerminate`. The delegate returns `.terminateCancel` unless `explicitQuit == true`, so the app stays alive while keeping its menu bar item. Only the menu's Quit items (which then call `terminate(nil)`) and the `willPowerOff` observer set the flag. Any other quit request, such as an Apple Event from Activity Monitor or an installer, is cancelled.

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
| `willPowerOff` | any | set `explicitQuit`, then `recorder?.stop()` (DOLL-183) |

`wasSleepInterrupted` is cleared by `didWake`, `sessionDidBecomeActive`, AND `stop(reason: .user)` (DOLL-182). The willSleep / sessionResign handlers stop with `reason: .sleepInterruption`, which preserves the flag they just set — `stop()` clearing it unconditionally made resume-on-wake dead code (DOLL-442).

Known gaps against this intent:

- **DOLL-182 does not hold.** `handleDidWake` and `handleSessionDidBecomeActive` clear the flag *before* scheduling the 1.5 s Task, and the Task checks only `!isRecording`. A user who starts and then stops within that window still gets the recording resumed.
- **`willPowerOff` is not synchronous.** Since DOLL-652 the observers are `for await` Tasks, so the handler runs on a later main-actor turn rather than inside the notification post. DOLL-183 assumed it drained directly.

### Security-scoped bookmark lifecycle

The user-picked output directory is persisted as a security-scoped bookmark in UserDefaults. Lifecycle:

1. **Save**: `RecordingState.saveOutputDirBookmark(for:)`, called from the folder pickers in onboarding and `OutputSettingsTab`; `URL.bookmarkData(options: .withSecurityScope)`. It stops access on the previous URL before storing the new one. The in-container default folder needs no bookmark (`useDefaultOutputDir()`, DOLL-344).
2. **Restore on launch**: a deferred `Task` (`bookmarkRestoreTask`, DOLL-114) resolves the bookmark, calls `startAccessingSecurityScopedResource`, and pushes the path into the Rust engine. Auto-record waits on this Task (DOLL-181).
3. **Hold during runtime**: the URL stays scoped until it is replaced or released.
4. **Release**: `releaseOutputDirAccess()` runs on quit (from `applicationShouldTerminate`, after `stop()`) and when switching to the default folder.

If the bookmark can't be resolved or access fails, the bookmark is dropped and the user is asked to pick again via `promptToReselectOutputDir` (DOLL-379). A bookmark that resolves but is marked stale is refreshed silently by calling `saveOutputDirBookmark(for:)` on the same URL. **Known bug:** that call stops access on the URL it was just granted, so the rest of that launch has no access to the folder. Nothing stops a recording from continuing into a folder whose access was just released by picking a new one.

### Carbon hotkey lifecycle

`GlobalHotkeyManager` is a `@MainActor` singleton wrapping the Carbon Event Hot Key API. `Shortcut` is `Codable` (persisted to UserDefaults under `globalShortcut`). The C callback uses `MainActor.assumeIsolated` since Carbon delivers hotkey events on the main run loop after `InstallEventHandler` is called from main (DOLL-161). Registration surfaces failures to the user — both at Settings-time (DOLL-157) and at launch-restoration (DOLL-184).

### Notification authorization (DOLL-134, DOLL-185)

`UNUserNotificationCenter` authorization is requested eagerly at init so the very-first auto-record-on-launch notification isn't dropped. The granted bool is captured into `notificationsAuthorized` and re-checked on `NSApplication.didBecomeActiveNotification` — granting in System Settings is picked up without a relaunch.

### Meter polling cadence

`RecordingState.isMeterWindowOpen` and `isMeterWindowOccluded` drive the meter Task. When the window is open, not fully covered, and the engine is recording or monitoring, a Task polls `bridge.fillPeakLevels(into:)` at ~30 Hz (DOLL-348, DOLL-374). The Task stops when the window closes or is covered, so no FFI calls happen for a meter nobody can see. Separately, the 1 Hz status poll reads only the lock-free status flags.

### `@Observable RecordingState` pattern

`RecordingState` is `@MainActor`-isolated and `@Observable` (Swift macro). Views hold it as a plain stored property (`var recorder: RecordingState`), not via `@Environment`; it's a class, so every view shares one instance, and SwiftUI's observation system propagates changes. View-model mutation off-main is a compile error because of `@MainActor`.

## Module map

| Module | Role |
|--------|------|
| `src/audio_processor.rs` | `AudioProcessor` trait — central abstraction over real (cpal) and mock processors. |
| `src/audio_recorder.rs` | High-level driver wrapping a processor + config. |
| `src/cpal_processor.rs` | Real audio I/O via cpal: device selection, the RT callback, spawning the writer thread, rotation counting. |
| `src/writer_thread.rs` | Writer-thread loop, ring-buffer consumer, file lifecycle, silence gate, peak metering. |
| `src/raw_wav_writer.rs` | Hand-rolled WAV writer for the hot path. |
| `src/silence_check_worker.rs` | Single-thread post-rotation silence checker with join-on-drop. |
| `src/utils.rs` | Channel-spec parsing, `is_silent`, and disk-space queries. |
| `src/macos_sample_rate_listener.rs` | CoreAudio property listener (macOS only). |
| `src/ffi.rs` | C ABI consumed by the SwiftUI app. Owns the canonical lock order. |
| `src/config.rs` | `AppConfig`: TOML + `BLACKBOX_*` env vars for the CLI (env wins); the app sets config through the FFI instead. |
| `src/constants.rs` | Defaults and tunables (`DEFAULT_*`, `MAX_CHANNELS`, `RING_BUFFER_SECONDS`, `WRITER_THREAD_READ_CHUNK`). |
| `src/numeric.rs` | Integer-to-float conversions, with the precision bound for each written down once. |
| `src/error.rs` | Typed error enum (`BlackboxError`) with `thiserror`. |
| `src/mock_processor.rs` | In-memory processor for tests. |
| `src/test_utils.rs` | Test helpers: synthetic audio, env-var fixtures, `MockClock`. |
| `src/benchmarking.rs` | Performance tracking behind the `benchmarking` feature. |
| `src/bin/main.rs` | The CLI recorder. |
| `src/bin/bench_writer.rs` | Write-throughput benchmark; only `--mode pipeline` exercises the production path. |
| `BlackBoxApp/` | SwiftUI menu-bar app; calls Rust via FFI. |

See `README.md` for the user-facing feature list and `SETUP.md` for build and release setup.
