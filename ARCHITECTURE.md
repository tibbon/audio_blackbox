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

The RT thread never blocks on I/O, locks, or allocations. The writer thread does all WAV encoding, peak metering, file rotation, and disk-space checks. The silence-check worker is a single dedicated thread per session that keeps scanning after finalize() returns.

## Threading rules

### RT (audio callback) thread

`CpalAudioProcessor` registers a cpal stream callback. That callback runs on a CoreAudio dispatch queue with hard latency requirements. Only safe operations:

- **Push samples** into the `rtrb` SPSC ring buffer. The buffer is sized for `RING_BUFFER_SECONDS = 5` (`src/constants.rs`) of audio at the device's rate × channel count, providing runway for stalls in the writer.
- **Atomic loads / stores** with Relaxed or Release ordering. No allocator calls, no mutex acquisition, no syscalls.

A test-time `CountingAllocator` (`mod alloc_counter` in `src/lib.rs`) wraps the system allocator with `AtomicU64::fetch_add`, and `src/tests/alloc_tests.rs` (no top-level `tests/` directory; all tests live under `src/tests/` to share `pub(crate)` access) asserts that the writer's steady-state `write_samples` path and the cpal callback's `push_samples_with_overflow_count` (with and without ring-buffer overflow; one `slots` load, one `push_partial_slice` of whole frames, and an atomic add) allocate nothing. They take about a second in a debug build and run in every `cargo test`; they rely on `--test-threads=1`, because the counter is process-wide.

### Writer thread

`writer_thread::writer_thread_main` is spawned in `process_audio` and joined when `finalize()` is called. Responsibilities:

- Drain the ring buffer, convert f32 to the configured bit depth, write WAV via `RawWavWriter` (a hand-rolled writer; we don't drag `hound` into the hot path).
- Maintain per-channel peak levels in cache-aligned `AtomicU32` slots — each slot holds the maximum since the last read (`CacheAlignedPeak::raise`), and the FFI 30 Hz meter poll reads and resets it (`take`), so transients between polls are not lost.
- Rotate files when the RT thread sets the `rotation_needed` flag (a Relaxed status flag — DOLL-391; the samples it implies are already synchronized through the rtrb ring, so no Acquire/Release pairing is needed). See `CpalAudioProcessor::process_audio_impl` (the store) and `writer_thread_main` (the `swap`). The callback's counter carries each period's overshoot, so boundaries do not drift. When the silence gate opens, the writer raises `rotation_restart`; the callback zeroes its counter and drops any rotation it flagged in the idle period, so the first file after an open runs one full cadence (plus the replayed pre-roll). `take_due_rotation` ignores `rotation_needed` until the callback has done that (Release clear on the RT side, Acquire load on the writer).
- Submit rotated files to the silence-check worker over a bounded `mpsc::sync_channel` (capacity 8). Back-pressures the writer thread if the silence checker can't keep up — acceptable trade-off for bounded memory. It can fill: rotation can be as short as 1 s, and `is_silent` decodes a silent file to the end, so checking a long silent file takes seconds to minutes.
- Monitor disk space and flip `disk_space_low` when the configured `min_disk_space_mb` precondition fails.
- Keep files crash-readable; see [Recording file lifecycle](#recording-file-lifecycle).

### Silence-check worker

`silence_check_worker::SilenceCheckWorker` is a single thread fed via a bounded `mpsc::sync_channel`. Its `Drop` impl closes the sending side and does **not** join: the thread scans whatever is still queued and then exits on its own. A scan decodes a silent file to its end, which takes minutes for a long multichannel take, and the drop runs while stopping a recording — on the Swift main thread via `blackbox_stop_recording`, and before an automatic restart after a sample-rate change or stream error. Stop must not wait on it.

The same goes for queueing: `finalize_all` submits with `try_submit`, so a full queue keeps those files unchecked instead of blocking the writer's shutdown reply. Rotation still uses the blocking `submit` (the back-pressure described above).

If the process exits before a scan runs, that file is kept. The check only ever deletes, so cutting it short never loses audio. `wait_for_silence_checks(timeout)` waits for every submitted scan; the CLI calls it before exiting, and tests rendezvous on it through `test_utils::drain_silence_checks`. Don't call it from a UI thread.

### Sample-rate listener (macOS)

`macos_sample_rate_listener::SampleRateListener` registers a CoreAudio property listener for sample-rate changes on the active device. The `client_data` is `Arc::into_raw(Arc::clone(&flag))` — the listener owns one strong refcount of an `Arc<AtomicBool>`.

`Drop` **deliberately leaks** the strong reference rather than reclaiming it: it rebuilds the `Arc` with `Arc::from_raw` inside a `ManuallyDrop`, so the refcount is never decremented. Apple's docs do not guarantee that `AudioObjectRemovePropertyListener` blocks until in-flight callbacks on other threads have returned — only that no *new* callbacks will start. Leaking eliminates the race entirely; the cost is one `Arc<AtomicBool>` allocation (about 24 bytes) per recording session for the process lifetime, bounded.

If you "fix" this by letting that `Arc` drop, you reintroduce a use-after-free that only fires under sample-rate-change-during-listener-removal — extremely rare, hard to reproduce, exactly the kind of bug we're refusing to ship.

## Recording file lifecycle

This is how the product keeps its "don't lose a take" promise. All of it runs on the writer thread.

1. **Open.** Each file is created as `<output_dir>/<YYYY-MM-DD-HH-MM-SS>[-chN].recording.wav` with a placeholder header: 44-byte `WAVE_FORMAT_PCM` for 16-bit mono or stereo, 68-byte `WAVE_FORMAT_EXTENSIBLE` (PCM subformat; channel mask front-center for mono, front-left/right for stereo, 0 above two channels) for 24/32-bit or more than two channels, as strict readers expect. Code that touches the header finds the size fields from the writer's `header_len` or by walking the chunks (`parse_wav_layout`), never at a fixed offset. `-chN` appears only in split mode and is the device channel counted from 1, as the app shows it (device channel 0 is `-ch1`; `split_channel_suffix`), even though `audio_channels` and the engine are 0-based. `-1`, `-2`, … is appended when the final `.wav` name or its `.recording.wav` temp name already exists (`disambiguate_recording_path`), and the temp file is created with `create_new`, so neither a finished recording nor a crashed take awaiting recovery is ever overwritten.
2. **Write.** `RawWavWriter` buffers through a `BufWriter`. Each frame goes in with one write (`write_frame`), so a write error that clears drops whole frames and never shifts the channels of an interleaved file. In split mode a channel file that missed frames writes them as silence once it accepts writes again (`write_frame_in_step`), so the channel files stay the same length and aligned. Every 10 s of audio (`flush_writers`, `sample_rate * 10` frames) it flushes, rewrites the RIFF and data sizes, and syncs the file to stable storage (`sync_data`, which is `F_FULLFSYNC` on macOS: about 5 ms per file), so the file on disk is always a readable WAV up to the last refresh. A hard kill leaves the file under its `.recording.wav` name with a header up to about 10 s behind the audio on disk; a power cut can also lose the audio written since the last refresh, at most about 10 s. `finalize` (rotation, gate close, stop) does not sync, to keep stop fast, so a power cut just after one can lose that file's last few seconds or its rename. `recover_recordings` (`src/recovery.rs`; FFI `blackbox_recover_recordings`) repairs those on the next launch: it finds the `data` chunk by walking the header, rewrites the sizes to cover every whole frame on disk (capped at what the `u32` fields hold), cuts a torn trailing frame, and renames the file to a free `.wav` name without replacing anything. A writer holds an advisory exclusive lock (`flock`, `File::try_lock`) on each `.recording.wav` while it is open, and recovery skips any file it can't lock, so a second recorder sharing the folder (the CLI next to the app) never has a live file finalized under it; a crashed process holds no lock. The CLI calls it at startup, the app at launch.
3. **Rotate or close.** Rotation (continuous mode), a silence-gate close, stop, and shutdown all go through `close_files`. It finalizes every writer (flush, RIFF pad byte, header), then renames every pending `.recording.wav` to its `.wav` name. A failure on one file does not stop the others (DOLL-345); the first error is returned. If the final flush fails (a full disk), `finalize` still rewrites the header to cover the whole frames that reached the file.
4. **Silence check.** When `silence_threshold > 0`, renamed files go to the silence-check worker, which deletes any file whose peak and RMS stay below the threshold. A file whose `finalize` failed is renamed but never submitted: its header may not describe the audio on disk. `is_silent` also keeps a file whose header reports zero samples but which is longer than a header; only a header-only file counts as silent.
5. **Limits.** WAV sizes are `u32`. After each read the writer loop calls `rotate_if_file_full` (the shutdown drain calls it before each read), which rotates (same path as a cadence rotation) once any open file holds `MAX_WAV_DATA_BYTES` of audio, 1 MiB short of the limit. This applies in every mode, including the non-continuous default. The header still saturates at `u32::MAX` as a last resort (DOLL-204).

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

`wasSleepInterrupted` is cleared by `didWake`, `sessionDidBecomeActive`, AND `stop(reason: .user)` (DOLL-182). The wake handlers consume the flag when they schedule the deferred start, so the pending resume is held as `pendingResumeTask`, which a user `stop(reason: .user)` or `start()` cancels: a start-then-stop inside the 1.5 s window is not resumed. The willSleep / sessionResign handlers stop with `reason: .sleepInterruption`, which preserves the flag they just set — `stop()` clearing it unconditionally made resume-on-wake dead code (DOLL-442).

`willPowerOff` and `willSleep` are observed with block observers that run inside the notification post (`AppDelegate.observeSynchronously`), so `explicitQuit` is set and the recording is finalized before AppKit asks `applicationShouldTerminate` or the Mac sleeps (DOLL-183). The other observers are `for await` Tasks, which run on a later main-actor turn.

### Security-scoped bookmark lifecycle

The user-picked output directory is persisted as a security-scoped bookmark in UserDefaults. Lifecycle:

1. **Save**: `RecordingState.saveOutputDirBookmark(for:)`, reached from the folder pickers in onboarding and `OutputSettingsTab` through `switchOutputDir(to:)`; `URL.bookmarkData(options: .withSecurityScope)`. It stops access on the previous URL before storing the new one. The in-container default folder needs no bookmark (`useDefaultOutputDir()`, DOLL-344).
2. **Restore on launch**: a deferred `Task` (`bookmarkRestoreTask`, DOLL-114) resolves the bookmark, calls `startAccessingSecurityScopedResource`, and pushes the path into the Rust engine. Auto-record waits on this Task (DOLL-181), which includes the re-pick prompt below, so auto-record never starts before the user has answered it.
3. **Hold during runtime**: the URL stays scoped until it is replaced or released.
4. **Release**: `releaseOutputDirAccess()` runs on quit (from `applicationShouldTerminate`, after `stop()`) and when switching to the default folder.

If the bookmark can't be resolved or access fails, the bookmark is dropped and the user is asked to pick again via `promptToReselectOutputDir` (DOLL-379). A bookmark that resolves but is marked stale is refreshed silently by rewriting only the stored bookmark data (`storeOutputDirBookmark(for:)`); it does not go through `saveOutputDirBookmark(for:)`, which would stop access on the URL it was just granted. Switching folders mid-recording goes through `switchOutputDir(to:)`: the Settings picker first asks to restart (the bit-depth/channels prompt), then the live session is finalized, the old folder's access is released while the engine is stopped, and recording restarts in the new folder, so access is never released under a live session.

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
| `src/silence_check_worker.rs` | Single-thread post-rotation silence checker; finishes its queue in the background after stop. |
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
