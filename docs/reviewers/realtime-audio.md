# Real-time audio reviewer

Runs when: the branch touches the capture path: `src/cpal_processor.rs`, `src/writer_thread.rs`,
`src/raw_wav_writer.rs`, `src/silence_check_worker.rs`, `src/constants.rs`,
`src/audio_processor.rs`, `src/audio_recorder.rs`, `src/macos_sample_rate_listener.rs`, or
`src/tests/alloc_tests.rs`. Routing lives in `.claude/workflows/ship-ticket.js`.

## In your lane
- The cpal input callback (the closures passed to `build_input_stream` in `cpal_processor.rs`)
  does only three things: push `f32` samples through `push_samples_with_overflow_count` into the
  `rtrb` ring, store status atomics, and return (§4). Flag any allocation (`Vec::push`, `format!`,
  `String`, `Box::new`, a heap-capturing closure), lock (including `try_lock` without a bounded
  fallback), logging through the `log` facade, syscall, file IO, or sleep reachable from it.
- Buffers and state used by the callback are built when the stream is built, never in the callback.
- Atomic ordering follows `ARCHITECTURE.md` "Atomic ordering" (DOLL-101): a flag whose set state
  implies other state written by the same writer uses Release on store and Acquire on load
  (`recording_active` / `monitoring_active` with `sample_rate`). Status-only flags (`gate_idle`,
  `disk_space_low`, `stream_error`, `sample_rate_changed`, `rotation_needed`) use Relaxed. A new
  flag gets the right flavor, and a changed store keeps its matching load.
- Writer thread (`writer_thread_main`): drains the ring, converts to the configured bit depth,
  writes through `RawWavWriter`, rotates on `rotation_needed` with `swap`, publishes peaks into
  `Arc<[CacheAlignedPeak]>` slots, feeds the silence-check worker over the bounded channel. Check
  that every exit path finalizes or flushes open writers and that join and send results are
  handled, not dropped (§1.5).
- `RawWavWriter`: header sizes are rewritten on `flush` and `finalize`, capped at 4 GiB instead of
  wrapping, and odd-length data chunks get the word-align pad. A change that skips the header
  rewrite on an error path leaves an unreadable file.
- Silence gate (`GateState` in `writer_thread.rs`) and pre-roll (DOLL-465): opening the gate
  replays the retained batch so the transient is not clipped; closing it finalizes the file.
- Rotation and file-per-channel (split) mode keep channel order and frame alignment; no partial
  frame is written across a rotation boundary.
- Thread lifecycle (§1.5): every spawned thread has an owned `JoinHandle`, a stop signal, and a
  join on stop or drop. `SilenceCheckWorker`'s `Drop` joins so queued files finish before
  `finalize()` returns; `mem::forget` on it is a bug.
- `src/tests/alloc_tests.rs` is the proof of zero hot-path allocations. A new hot-path feature
  extends it; a change that weakens or bypasses it is high.

## Not your lane
- Generic correctness of non-hot-path code: `baseline`. `unsafe` soundness and the listener's
  C callback contract: `ffi-unsafe`. Casts and error design: `rust-core`. Test style: `tests`.
- Anything `scripts/check.sh` fails on mechanically (fmt, clippy -D warnings, swiftlint --strict,
  header parity, catalog sync). The gate catches those; do not report them.

## Severity in this lane
- critical: samples silently lost or reordered, a WAV left with a wrong header after a normal stop
  or rotation, a use-after-free or data race on the capture path, a deadlock on stop.
- high: an allocation, lock, log call, or syscall in the callback; a thread with no shutdown path;
  a payload flag switched to Relaxed; `alloc_tests` weakened.
- medium: an error path that skips flush, join, or worker hand-off under a narrow trigger; a new
  atomic with no documented ordering rationale; unbounded growth of writer-side buffers.
- low: cache locality, loop shape, naming.

## Facts that prevent false positives
- The writer thread is not real-time. Allocation, logging, and IO are allowed there; they are
  banned only in the callback.
- The writer's 1 to 5 ms idle backoff uses `thread::sleep` on purpose, with an `#[expect]` citing
  DOLL-270: the producer stays lock- and syscall-free, so there is nothing to block on.
- `rotation_needed` is Relaxed on purpose (DOLL-391): the samples it implies already travel through
  the ring, so no Acquire/Release pairing is needed.
- The sample-rate listener's `Drop` leaks one `Arc<AtomicBool>` on purpose. CoreAudio does not
  promise that removing the listener waits for in-flight callbacks; reclaiming the Arc
  reintroduces a use-after-free (`ARCHITECTURE.md` "Sample-rate listener").
- The silence-check worker's bounded `sync_channel` back-pressures the writer by design.
- `RING_BUFFER_SECONDS = 5` sizes the ring for writer stalls; overflow is counted in
  `write_errors`, not silently dropped.
