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

A test-time `CountingAllocator` (`mod alloc_counter` in `src/lib.rs`) wraps the system allocator with `AtomicU64::fetch_add`, and `tests/alloc_tests.rs` asserts the hot path produces zero allocations.

### Writer thread

`writer_thread::run` is spawned in `process_audio` and joined when `finalize()` is called. Responsibilities:

- Drain the ring buffer, convert f32 to the configured bit depth, write WAV via `RawWavWriter` (a hand-rolled writer; we don't drag `hound` into the hot path).
- Maintain per-channel peak levels in cache-aligned `AtomicI32` slots — read by the FFI 30 Hz meter poll.
- Rotate files on `rotation_needed.swap(false, Ordering::Acquire)` (Release-paired with the RT thread's store at `cpal_processor.rs:476`).
- Submit rotated files to the silence-check worker over a bounded mpsc channel. Never blocks.
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

- **Synchronizing flags** — Acquire/Release pairs that publish or observe a *payload* held in another atomic. Example: `recording_active.store(true, Ordering::Release)` at `cpal_processor.rs:519` synchronizes-with the FFI status poll's Acquire load, so a reader that sees `recording_active = true` is also guaranteed to see the matching `sample_rate` written before the Release (DOLL-101).
- **Status-only flags** — single-bit signals with no synchronizes-with relationship. `gate_idle`, `disk_space_low`, `stream_error`, `sample_rate_changed`, the ctrlc-handler shutdown flag in `bin/main.rs`. All Relaxed pairs. The only correctness requirement is "the value is eventually visible," which Relaxed satisfies.

If you're adding a new atomic flag: ask whether a reader observing this flag's set state needs to also observe other state set by the same writer. If yes → Acquire/Release. If no → Relaxed.

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
