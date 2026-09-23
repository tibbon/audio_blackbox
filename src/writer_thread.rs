use std::ffi::CString;
use std::fs;
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::Duration;

use log::{error, info, warn};

use crate::constants::{CacheAlignedPeak, MAX_CHANNELS, OutputMode, WRITER_THREAD_READ_CHUNK};
use crate::error::BlackboxError;
use crate::numeric::saturating_i32;
use crate::raw_wav_writer::{MAX_WAV_DATA_BYTES, RawWavWriter};
use crate::silence_check_worker::SilenceCheckWorker;
use crate::utils::{available_disk_space_mb, is_silent};

use chrono::prelude::*;

// ---------------------------------------------------------------------------
// File-rotation helpers — timestamp formatting and tmp/.wav path derivation.
// ---------------------------------------------------------------------------

/// Returns a timestamp string like "2024-01-15-14-30-05" from the current local time.
/// Includes seconds so that file rotations within the same minute produce distinct names.
pub(crate) fn timestamp_now() -> String {
    Local::now().format("%Y-%m-%d-%H-%M-%S").to_string()
}

/// Pluggable source of timestamp strings used by `WriterThreadState` for filename
/// stamps. Production uses `timestamp_now`; tests inject a deterministic source
/// (see `crate::test_utils::MockClock`) so two rotations don't collide on the
/// wall clock and don't need a real second to elapse between them.
type TimestampFn = Arc<dyn Fn() -> String + Send + Sync>;

/// Returns a `.recording.wav` temporary path for the given final `.wav` path.
fn tmp_wav_path(final_path: &str) -> String {
    final_path.strip_suffix(".wav").map_or_else(
        || format!("{final_path}.recording"),
        |stem| format!("{stem}.recording.wav"),
    )
}

/// Returns a path that doesn't collide with an existing file (DOLL-207).
///
/// During a DST backward jump (one hour rolled back), a wall-clock
/// second can repeat — if two rotations land in that second, the
/// second rotation's `fs::rename` would silently overwrite the first
/// file. Probability is roughly zero in practice (DST jumps × in-flight
/// recording × in-flight rotation × same second) but data loss is
/// unrecoverable when it does happen.
///
/// If `final_path` exists, this returns `final_path-1.wav`, `-2.wav`,
/// etc. up to `-999`. If even those are all taken (extraordinarily
/// unlikely), it appends a nanosecond suffix rather than returning the
/// colliding original — never silently overwrite an existing recording
/// (DOLL-268).
fn disambiguate_path(final_path: &str) -> String {
    if !Path::new(final_path).exists() {
        return final_path.to_owned();
    }
    // Match `.wav` only as the literal lowercase suffix our writer
    // emits — the clippy `case_sensitive_file_extension_comparisons`
    // warning is for cross-platform path APIs, not our deterministic
    // lowercase suffix.
    let (stem, ext) = final_path
        .strip_suffix(".wav")
        .map_or((final_path, ""), |stem| (stem, ".wav"));
    for n in 1..1000 {
        let candidate = format!("{stem}-{n}{ext}");
        if !Path::new(&candidate).exists() {
            return candidate;
        }
    }
    // Exhausted -1..-999 (extraordinarily unlikely). Never fall back to the
    // colliding original — finalize_all / rotate_files would then
    // `fs::rename(tmp, original)` straight over the existing file, losing it
    // (DOLL-268). Append a nanosecond suffix for a near-certainly-unique
    // name instead.
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_nanos());
    let candidate = format!("{stem}-{nanos}{ext}");
    log::warn!(
        "Path disambiguation exhausted (>1000 collisions) for {final_path}; using {candidate}"
    );
    candidate
}

/// Create a WAV writer using our direct-write `RawWavWriter`.
fn create_wav_writer(
    path: &str,
    spec: crate::raw_wav_writer::WavSpec,
) -> Result<RawWavWriter, BlackboxError> {
    RawWavWriter::create(path, spec).map_err(|e| BlackboxError::WavSource {
        context: format!("Failed to create WAV file at {path}"),
        source: Box::new(e),
    })
}

// ---------------------------------------------------------------------------
// Silence gate state machine
// ---------------------------------------------------------------------------

/// Whether the silence gate is currently idle (no files open) or recording.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum GateState {
    /// No audio signal — WAV files are closed, only tracking peaks.
    Idle,
    /// Audio signal present — WAV files are open and writing.
    Recording,
}

// ---------------------------------------------------------------------------
// WriterCommand — sent from the processor to the writer thread
// ---------------------------------------------------------------------------

pub(crate) enum WriterCommand {
    /// Drain remaining samples and finalize all files.
    Shutdown(std::sync::mpsc::Sender<Result<(), BlackboxError>>),
}

// ---------------------------------------------------------------------------
// WriterThreadHandle — held by CpalAudioProcessor
// ---------------------------------------------------------------------------

pub(crate) struct WriterThreadHandle {
    pub command_tx: std::sync::mpsc::SyncSender<WriterCommand>,
    pub join_handle: Option<std::thread::JoinHandle<()>>,
}

// ---------------------------------------------------------------------------
// WriterThreadState — lives entirely on the writer thread
// ---------------------------------------------------------------------------

#[expect(
    clippy::struct_excessive_bools,
    reason = "hot-path state flags read every write_samples() call; a state enum would add a match per call"
)]
#[expect(
    clippy::partial_pub_fields,
    reason = "the private fields are caches derived from the pub configuration fields and must not be set independently"
)]
pub(crate) struct WriterThreadState {
    // --- Hot fields: accessed every write_samples() call, grouped for cache locality ---
    /// Cached scale factor for f32-to-WAV conversion.
    sample_scale: f32,
    /// When true, only track peak levels without writing to disk.
    pub monitor_only: bool,
    /// When true, writing is paused because disk space is low.
    pub disk_stopped: bool,
    /// Whether the silence gate feature is enabled.
    pub gate_enabled: bool,
    /// Current gate state (Idle = no files open, Recording = writing to disk).
    pub gate_state: GateState,
    /// Output mode as a 1-byte enum — eliminates string comparison in the hot-path match.
    pub output_mode: OutputMode,
    /// Number of active channels (indexes into `channels` array).
    pub channel_count: u8,
    /// Total interleaved channels from the audio device.
    pub total_device_channels: u16,
    /// Iteration counter for amortizing disk space checks (avoids syscall per loop iteration).
    pub disk_check_counter: u16,
    /// Frame counter for periodic WAV flush (crash-safe headers every ~10 seconds of audio).
    flush_frame_counter: u32,
    /// Channel indices as a fixed inline array — no heap indirection, always in cache.
    /// Only the first `channel_count` entries are valid.
    pub channels: [u8; MAX_CHANNELS],
    /// Per-frame peak accumulator — fixed inline array, no heap pointer chase.
    /// Only the first `channel_count` entries are used.
    peak_scratch: [f32; MAX_CHANNELS],
    /// Cached active-channel filter (DOLL-375): the indices into
    /// `channels[..channel_count]` whose device channel is in range for the
    /// current frame size. The active set is a pure function of the (immutable)
    /// channel list and `total_device_channels`, so it's recomputed only when
    /// the frame size changes — not zero-initialised and rebuilt on every
    /// `write_samples` call. Only the first `active_count` entries are valid.
    active_indices: [u8; MAX_CHANNELS],
    active_count: usize,
    /// The frame size `active_indices` was built for; `usize::MAX` = not built.
    active_cache_frame_size: usize,
    /// xorshift32 state for TPDF dither on 16-bit output (DOLL-373). Nonzero.
    dither_rng: u32,
    /// Running count of consecutive `write_sample` failures (DOLL-349). Reset on
    /// any successful batch. When it reaches ~1s of audio, recording self-stops
    /// (disk full / unwritable) instead of silently dropping every sample
    /// forever. This is a thread-local streak counter for the stop decision
    /// only; individual failures also still bump the shared `write_errors`
    /// counter (so the stop bounds, rather than fully removes, the window where
    /// disk-full is misattributed to load — see follow-up DOLL note).
    consecutive_write_failures: u64,

    // --- Warm fields: accessed frequently but not per-sample ---
    pub writer: Option<RawWavWriter>,
    pub multichannel_writers: Vec<Option<RawWavWriter>>,
    pub write_errors: Arc<AtomicU64>,
    /// Per-channel peak levels (f32 stored as u32 bits via `to_bits()`). Shared with FFI.
    /// Each element is cache-line-aligned to prevent false sharing with the UI reader thread.
    pub peak_levels: Arc<[CacheAlignedPeak]>,
    /// Partial frames carried over between ring buffer reads.
    frame_remainder: Vec<f32>,
    /// Pre-allocated buffer for combining `frame_remainder` + new data (avoids heap alloc).
    combined_buf: Vec<f32>,
    /// Set by `write_samples` when signal is detected in Idle mode.
    /// The main loop opens writers before the next read, keeping `write_samples` I/O-free.
    pub gate_pending_open: bool,
    /// Set by `write_samples` when silence timeout is reached in Recording mode.
    /// The main loop finalizes writers, keeping `write_samples` free of file I/O.
    pub gate_pending_close: bool,
    /// Consecutive silent frames counted while gate is Recording.
    gate_silence_frames: u64,
    /// Frame count threshold for gate timeout (`timeout_secs * sample_rate`).
    gate_timeout_frames: u64,
    /// Shared flag: true when gate is idle (no files open). Read by FFI for status.
    pub gate_idle: Arc<AtomicBool>,
    /// Pre-roll retention for the silence gate (DOLL-465): the most recent
    /// frame-aligned batch seen while the gate was Idle. The batch that trips
    /// the gate open is processed in the peaks-only branch before any writer
    /// exists, so without this the signal onset — up to one
    /// `WRITER_THREAD_READ_CHUNK` (~170 ms at 48 kHz stereo) — was silently
    /// discarded. `process_gate_open` replays it into the freshly opened
    /// writers before live samples resume. Lives on the writer thread only,
    /// so the RT callback is unaffected.
    gate_preroll: Vec<f32>,
    /// Data bytes a file may hold before the writer rotates to a new one
    /// (`MAX_WAV_DATA_BYTES`; tests lower it). WAV sizes are `u32`, so a file
    /// past 4 GiB would carry a saturated header that players truncate.
    pub(crate) max_data_bytes: u64,
    /// Set when the silence gate opens, to restart the cadence clock. The
    /// RT callback's rotation counter kept running while the gate was idle,
    /// so the first file after an open used to end at whatever point of the
    /// cadence period the counter had reached. The callback sees this flag,
    /// zeroes its counter, drops any rotation it had already flagged, and
    /// clears it (see `cpal_processor::apply_rotation_restart`); until then
    /// `take_due_rotation` ignores `rotation_needed`. The first file after
    /// an open therefore runs one full cadence from the open, plus the
    /// replayed pre-roll.
    pub(crate) rotation_restart: Arc<AtomicBool>,

    // --- Cold fields: only accessed during setup, rotation, or shutdown ---
    pub output_dir: String,
    /// Pre-allocated `CString` of `output_dir` for `statvfs` calls (avoids heap alloc per check).
    #[cfg(unix)]
    output_dir_cstr: Option<CString>,
    pub sample_rate: u32,
    pub bits_per_sample: u16,
    pub current_spec: crate::raw_wav_writer::WavSpec,
    pub pending_files: Vec<(String, String)>,
    pub silence_threshold: f32,
    /// Minimum free disk space in MB before stopping writes (0 = disabled).
    pub min_disk_space_mb: u64,
    /// Shared flag: set when disk space drops below threshold.
    pub disk_space_low: Arc<AtomicBool>,
    /// Shared flag: set when `write_sample` keeps failing (DOLL-437). Defaults
    /// to a private throwaway; the FFI-exposed clone is injected post-construction
    /// by `CpalAudioProcessor`. Distinct from `disk_space_low` so the UI can
    /// show a precise "unable to write to disk" message.
    pub write_failed: Arc<AtomicBool>,
    /// Pluggable timestamp source for filename stamps. Production passes
    /// `Arc::new(timestamp_now)`; tests pass a `MockClock` so rotations
    /// produce distinct filenames without sleeping past a wall-clock second.
    pub(crate) timestamp_fn: TimestampFn,
    /// Single dedicated worker that scans recently-rotated files for
    /// silence and deletes them. `Some` when `silence_threshold > 0`,
    /// `None` otherwise. Dropping it closes its queue; the thread finishes
    /// the queued scans in the background (see `silence_check_worker`).
    silence_worker: Option<SilenceCheckWorker>,
    /// Cumulative count of samples consumed via `read_available`. Tests
    /// poll this to know when the writer thread has drained a known
    /// number of samples — replaces the prior `thread::sleep(50ms)`
    /// rendezvous with deterministic state (DOLL-127). Only the tests read
    /// it, so it's gated to test builds — the production drain path carries
    /// no extra atomic (DOLL-269).
    #[cfg(test)]
    pub(crate) samples_consumed_total: Arc<AtomicU64>,
}

/// Convert an f32 sample (range -1.0..1.0) to an i32 scaled for the given bit depth.
///
/// Clamps out-of-range inputs (and NaN, which clamps to one of the bounds)
/// before rounding, so callers can't silently emit truncated or sign-flipped
/// values. The hot path uses the pre-cached `sample_scale` field on
/// `WriterThreadState` for speed; this helper exists solely so tests can
/// assert the conversion math without going through `write_samples`. The
/// `bench-writer` binary keeps an inline copy because the lib helper is
/// not part of the public API (DOLL-129).
#[cfg(test)]
pub(crate) fn f32_to_wav_sample(sample: f32, bits_per_sample: u16) -> i32 {
    saturating_i32((sample.clamp(-1.0, 1.0) * pcm_full_scale(bits_per_sample)).round())
}

/// Create `output_dir` if needed and refuse to start below the free-space
/// threshold, raising `disk_space_low` for the status poll.
fn prepare_output_dir(
    output_dir: &str,
    min_disk_space_mb: u64,
    disk_space_low: &AtomicBool,
) -> Result<(), BlackboxError> {
    if !Path::new(output_dir).exists() {
        fs::create_dir_all(output_dir)?;
    }

    // Fail early if disk space is already below threshold
    if min_disk_space_mb > 0
        && let Some(available_mb) = available_disk_space_mb(output_dir)
        && available_mb < min_disk_space_mb
    {
        // status flag only; reader at disk_space_low() loads Relaxed.
        disk_space_low.store(true, Ordering::Relaxed);
        return Err(BlackboxError::InsufficientDiskSpace {
            available_mb,
            required_mb: min_disk_space_mb,
        });
    }
    Ok(())
}

/// Pack up to `MAX_CHANNELS` channel indices into the inline array the hot
/// path reads, returning the array and how many entries are set.
///
/// An index that doesn't fit a `u8` is skipped. None are expected:
/// `parse_channel_string` rejects indices >= `MAX_CHANNELS`, and the
/// all-device-channels fallback starts at 0, so its first 255 entries fit.
/// Before DOLL-653 such an index would have wrapped to a different channel.
fn pack_channels(channels: &[usize]) -> ([u8; MAX_CHANNELS], u8) {
    let mut packed = [0_u8; MAX_CHANNELS];
    let mut count = 0_u8;
    for (slot, ch) in packed
        .iter_mut()
        .zip(channels.iter().filter_map(|&ch| u8::try_from(ch).ok()))
    {
        *slot = ch;
        count += 1;
    }
    (packed, count)
}

/// Multiplier that maps a sample in [-1, 1] to integer PCM at `bits_per_sample`.
///
/// 16 and 24 bits use the positive maximum (2^15 - 1, 2^23 - 1). Anything else
/// is treated as 32-bit, whose maximum 2^31 - 1 has no exact `f32`: it rounds
/// to 2^31, which is what `i32::MAX as f32` always produced. The saturating
/// `as i32` in the conversion maps +1.0 back to `i32::MAX`.
pub(crate) const fn pcm_full_scale(bits_per_sample: u16) -> f32 {
    match bits_per_sample {
        16 => 32_767.0,
        24 => 8_388_607.0,
        // 2^31, spelled as a product: clippy::lossy_float_literal misreads the
        // exact literal 2_147_483_648.0 as lossy.
        _ => 32_768.0 * 65_536.0,
    }
}

/// xorshift32 PRNG step → uniform f32 in [0, 1). Cheap and alloc/lock-free, so
/// it's safe to call per-sample on the writer thread. `state` must be nonzero.
#[inline]
fn xorshift32_unit(state: &mut u32) -> f32 {
    let mut x = *state;
    x ^= x << 13;
    x ^= x >> 17;
    x ^= x << 5;
    *state = x;
    // Top 24 bits → [0, 1); ample resolution for a 1-LSB dither. Built from
    // a u16 and a u8 so each conversion to f32 is exact (DOLL-653).
    let [b0, b1, b2, _] = x.to_be_bytes();
    f32::from(u16::from_be_bytes([b0, b1])).mul_add(256.0, f32::from(b2)) / 16_777_216.0
}

/// Convert an f32 sample in [-1, 1] to an integer PCM sample scaled by `scale`.
///
/// DOLL-373: when `dither` is set (16-bit output), add TPDF dither — the sum of
/// two independent uniform [-0.5, +0.5] LSB values — before rounding, to
/// decorrelate quantization error on low-level tails/reverb/fades, then clamp
/// so the added noise can't push the result past the integer range. 24/32-bit
/// output has enough headroom that dither is unnecessary, so it stays
/// bit-identical to the plain conversion.
#[inline]
fn convert_sample(s: f32, scale: f32, dither: bool, rng: &mut u32) -> i32 {
    let v = s.clamp(-1.0, 1.0) * scale;
    if dither {
        let d = (xorshift32_unit(rng) - 0.5) + (xorshift32_unit(rng) - 0.5);
        saturating_i32((v + d).round().clamp(-(scale + 1.0), scale))
    } else {
        saturating_i32(v.round())
    }
}

impl WriterThreadState {
    /// Create a new `WriterThreadState` with initial WAV writers set up
    /// (none yet when the silence gate starts idle).
    #[expect(
        clippy::too_many_arguments,
        reason = "constructor mirrors the config fields one-to-one; a builder would add a heap round-trip per recording start"
    )]
    pub(crate) fn new(
        output_dir: &str,
        sample_rate: u32,
        channels: &[usize],
        output_mode: OutputMode,
        silence_threshold: f32,
        write_errors: Arc<AtomicU64>,
        min_disk_space_mb: u64,
        disk_space_low: Arc<AtomicBool>,
        bits_per_sample: u16,
        peak_levels: Arc<[CacheAlignedPeak]>,
        gate_enabled: bool,
        gate_timeout_secs: u64,
    ) -> Result<Self, BlackboxError> {
        prepare_output_dir(output_dir, min_disk_space_mb, &disk_space_low)?;

        let mut state = Self {
            sample_scale: pcm_full_scale(bits_per_sample),
            monitor_only: false,
            gate_enabled,
            gate_state: if gate_enabled {
                GateState::Idle
            } else {
                GateState::Recording
            },
            output_mode,
            write_errors,
            gate_timeout_frames: u64::from(sample_rate) * gate_timeout_secs,
            gate_idle: Arc::new(AtomicBool::new(gate_enabled)),
            output_dir: output_dir.to_owned(),
            #[cfg(unix)]
            output_dir_cstr: CString::new(output_dir).ok(),
            bits_per_sample,
            current_spec: crate::raw_wav_writer::WavSpec {
                channels: 1,
                sample_rate,
                bits_per_sample,
            },
            silence_threshold,
            min_disk_space_mb,
            disk_space_low,
            // SilenceCheckWorker::new returns Option (DOLL-122) — spawn
            // failures degrade to "no silence checks this session"
            // rather than crashing the recording.
            silence_worker: if silence_threshold > 0.0 {
                SilenceCheckWorker::new(silence_threshold)
            } else {
                None
            },
            ..Self::base(sample_rate, channels, peak_levels)
        };

        // When gate is enabled, start idle (no files). Writers are created on first signal.
        if !gate_enabled {
            match output_mode {
                OutputMode::Split => state.setup_split_mode()?,
                OutputMode::Single if channels.len() <= 2 => state.setup_standard_mode()?,
                OutputMode::Single => state.setup_multichannel_mode()?,
            }
        }

        Ok(state)
    }

    /// Create a monitor-only `WriterThreadState` that tracks peak levels without writing files.
    pub(crate) fn new_monitor(
        sample_rate: u32,
        channels: &[usize],
        peak_levels: Arc<[CacheAlignedPeak]>,
    ) -> Self {
        Self::base(sample_rate, channels, peak_levels)
    }

    /// Fields shared by both constructors, with every mode-specific field at
    /// its monitor-mode value: 24-bit scale, no files, gate and silence checks
    /// off. `new` overrides the recording fields with struct update syntax.
    fn base(sample_rate: u32, channels: &[usize], peak_levels: Arc<[CacheAlignedPeak]>) -> Self {
        let (ch_arr, channel_count) = pack_channels(channels);

        Self {
            sample_scale: 8_388_607.0_f32, // 24-bit max (monitor mode is always 24-bit)
            monitor_only: true,
            disk_stopped: false,
            gate_enabled: false,
            gate_state: GateState::Recording,
            output_mode: OutputMode::Single, // unused in monitor mode
            channel_count,
            total_device_channels: 0, // set by caller or process_audio
            disk_check_counter: 0,
            flush_frame_counter: 0,
            channels: ch_arr,
            peak_scratch: [0.0_f32; MAX_CHANNELS],
            active_indices: [0_u8; MAX_CHANNELS],
            active_count: 0,
            active_cache_frame_size: usize::MAX,
            dither_rng: 0x9E37_79B9, // nonzero xorshift32 seed (DOLL-373)
            consecutive_write_failures: 0,
            writer: None,
            multichannel_writers: Vec::new(),
            write_errors: Arc::new(AtomicU64::new(0)),
            peak_levels,
            frame_remainder: Vec::new(),
            combined_buf: Vec::new(),
            gate_pending_open: false,
            gate_pending_close: false,
            gate_silence_frames: 0,
            gate_timeout_frames: 0,
            gate_idle: Arc::new(AtomicBool::new(false)),
            gate_preroll: Vec::new(),
            max_data_bytes: MAX_WAV_DATA_BYTES,
            rotation_restart: Arc::new(AtomicBool::new(false)),
            output_dir: String::new(),
            #[cfg(unix)]
            output_dir_cstr: None, // Monitor mode doesn't check disk space
            sample_rate,
            bits_per_sample: 24,
            current_spec: crate::raw_wav_writer::WavSpec {
                channels: 1,
                sample_rate,
                bits_per_sample: 24,
            },
            pending_files: Vec::new(),
            silence_threshold: 0.0,
            min_disk_space_mb: 0,
            disk_space_low: Arc::new(AtomicBool::new(false)),
            write_failed: Arc::new(AtomicBool::new(false)),
            timestamp_fn: Arc::new(timestamp_now),
            silence_worker: None, // monitor mode never writes files, so no silence checks
            #[cfg(test)]
            samples_consumed_total: Arc::new(AtomicU64::new(0)),
        }
    }

    /// Replace the timestamp source used for filename stamps. Used by tests
    /// to make rotations produce deterministic, collision-free filenames
    /// without sleeping past a wall-clock second.
    #[cfg(test)]
    pub(crate) fn set_timestamp_fn(&mut self, f: TimestampFn) {
        self.timestamp_fn = f;
    }

    fn setup_split_mode(&mut self) -> Result<(), BlackboxError> {
        let date_str = (self.timestamp_fn)();
        let ch_count = self.channel_count as usize;

        info!("Setting up split mode with {ch_count} channels");

        self.multichannel_writers.clear();
        self.multichannel_writers.resize_with(ch_count, || None);

        for idx in 0..ch_count {
            let final_path = self.period_path(&date_str, &format!("-ch{}", self.channels[idx]));
            let writer =
                self.open_pending_writer(final_path, self.mono_spec(), "channel WAV file")?;
            self.multichannel_writers[idx] = Some(writer);
        }

        self.current_spec = self.mono_spec();
        Ok(())
    }

    fn setup_multichannel_mode(&mut self) -> Result<(), BlackboxError> {
        let date_str = (self.timestamp_fn)();
        let ch_count = self.channel_count as usize;

        info!("Setting up multichannel mode with {ch_count} channels");

        let spec = self.multichannel_spec();
        let final_path = self.period_path(&date_str, "-multichannel");
        self.writer = Some(self.open_pending_writer(final_path, spec, "multichannel WAV file")?);
        self.current_spec = spec;
        Ok(())
    }

    fn setup_standard_mode(&mut self) -> Result<(), BlackboxError> {
        let date_str = (self.timestamp_fn)();
        let ch_count = self.channel_count as usize;

        info!("Setting up standard mode with {ch_count} channels");

        let spec = crate::raw_wav_writer::WavSpec {
            channels: if ch_count == 1 { 1 } else { 2 },
            sample_rate: self.sample_rate,
            bits_per_sample: self.bits_per_sample,
        };
        let final_path = self.period_path(&date_str, "");
        self.writer = Some(self.open_pending_writer(final_path, spec, "WAV file")?);
        self.current_spec = spec;
        Ok(())
    }

    /// `<output_dir>/<date_str><suffix>.wav`, made unique if that file exists.
    fn period_path(&self, date_str: &str, suffix: &str) -> String {
        disambiguate_path(&format!("{}/{date_str}{suffix}.wav", self.output_dir))
    }

    /// Spec of one per-channel file in split mode.
    const fn mono_spec(&self) -> crate::raw_wav_writer::WavSpec {
        crate::raw_wav_writer::WavSpec {
            channels: 1,
            sample_rate: self.sample_rate,
            bits_per_sample: self.bits_per_sample,
        }
    }

    /// Spec of the interleaved file for more than two channels.
    fn multichannel_spec(&self) -> crate::raw_wav_writer::WavSpec {
        crate::raw_wav_writer::WavSpec {
            channels: u16::from(self.channel_count),
            sample_rate: self.sample_rate,
            bits_per_sample: self.bits_per_sample,
        }
    }

    /// Create the temp-file writer for `final_path` and record the pending
    /// tmp → final rename, logging `Created {what}: {final_path}`.
    fn open_pending_writer(
        &mut self,
        final_path: String,
        spec: crate::raw_wav_writer::WavSpec,
        what: &str,
    ) -> Result<RawWavWriter, BlackboxError> {
        let tmp_path = tmp_wav_path(&final_path);
        let writer = create_wav_writer(&tmp_path, spec)?;
        info!("Created {what}: {final_path}");
        self.pending_files.push((tmp_path, final_path));
        Ok(writer)
    }

    /// Check available disk space and stop writing if below threshold.
    /// Returns true if writing should continue, false if disk is low.
    ///
    /// Uses an iteration counter to amortize the cost: only performs the actual
    /// `statvfs` syscall every 10,000 calls (~4 seconds at typical throughput),
    /// avoiding a `clock_gettime` syscall on every writer thread loop iteration.
    pub(crate) fn check_disk_space(&mut self) -> bool {
        if self.monitor_only || self.min_disk_space_mb == 0 || self.disk_stopped {
            return !self.disk_stopped;
        }

        // Check every 10,000 iterations (~4 seconds at typical throughput)
        // instead of calling Instant::now() every iteration.
        self.disk_check_counter += 1;
        if self.disk_check_counter < 10_000 {
            return true;
        }
        self.disk_check_counter = 0;

        // Use cached CString on unix to avoid heap allocation per check.
        #[cfg(unix)]
        let available_mb = self
            .output_dir_cstr
            .as_deref()
            .and_then(crate::utils::available_disk_space_mb_cstr);
        #[cfg(not(unix))]
        let available_mb = available_disk_space_mb(&self.output_dir);

        if let Some(available_mb) = available_mb
            && available_mb < self.min_disk_space_mb
        {
            warn!(
                "Disk space low: {available_mb}MB available, threshold is {}MB — stopping recording",
                self.min_disk_space_mb
            );
            // status flag only; reader at disk_space_low() loads Relaxed.
            self.disk_space_low.store(true, Ordering::Relaxed);
            self.disk_stopped = true;
            // Finalize current files so data written so far is safe
            if let Err(e) = self.finalize_all() {
                error!("Error finalizing files after disk space warning: {e}");
            }
            return false;
        }
        true
    }

    /// Flush all active WAV writers to make files crash-recoverable.
    ///
    /// `RawWavWriter::flush()` rewrites the WAV header with the correct data
    /// size and flushes the underlying `BufWriter` to the OS. After a flush, the
    /// file is a valid WAV playable up to that point — even after a force-quit or
    /// SIGKILL. Counts audio frames (~10 seconds worth) for predictable timing
    /// regardless of channel count or loop speed.
    pub(crate) fn flush_writers(&mut self, samples_consumed: usize) {
        if self.monitor_only || self.disk_stopped || samples_consumed == 0 {
            return;
        }

        // Defense in depth (DOLL-112): a usize > u32::MAX is unreachable
        // today since WRITER_THREAD_READ_CHUNK is 16_384, but a future
        // bump could regress this to a silent zero (and skip flush).
        let samples_u32 = u32::try_from(samples_consumed).unwrap_or(u32::MAX);
        let frames = samples_u32 / u32::from(self.total_device_channels.max(1));
        self.flush_frame_counter += frames;
        if self.flush_frame_counter < self.sample_rate * 10 {
            return;
        }
        self.flush_frame_counter = 0;

        if let Some(w) = &mut self.writer
            && let Err(e) = w.flush()
        {
            error!("Error flushing WAV writer: {e}");
        }
        for w in self.multichannel_writers.iter_mut().flatten() {
            if let Err(e) = w.flush() {
                error!("Error flushing channel WAV writer: {e}");
            }
        }
    }

    /// Write interleaved f32 samples to WAV writers.
    ///
    /// Handles partial frames: if `data` doesn't divide evenly by `total_device_channels`,
    /// leftover samples are stored in `frame_remainder` and prepended to the next call.
    /// Also tracks per-channel peak levels for metering.
    ///
    /// The per-frame loops live in `track_peaks`, `write_split_frames` and
    /// `write_single_frames`. Each is called once per batch, so the split adds
    /// no call per frame (DOLL-653).
    pub(crate) fn write_samples(&mut self, data: &[f32]) {
        // If total_device_channels is 0 or disk stopped, skip writing
        if self.total_device_channels == 0 || self.disk_stopped {
            return;
        }

        // DOLL-465: detach the pre-roll buffer so the gate-idle branch can
        // fill it while `frame_data` is borrowed; reattached at the end.
        // `combined_buf` is detached the same way so the batch helpers below
        // can take `&mut self` (DOLL-653). `mem::take` doesn't allocate.
        let mut gate_preroll = std::mem::take(&mut self.gate_preroll);
        let mut combined_buf = std::mem::take(&mut self.combined_buf);

        // Prepend any leftover samples from the previous call using a pre-allocated buffer
        let work_data: &[f32] = if self.frame_remainder.is_empty() {
            data
        } else {
            combined_buf.clear();
            combined_buf.extend_from_slice(&self.frame_remainder);
            combined_buf.extend_from_slice(data);
            self.frame_remainder.clear();
            &combined_buf
        };

        let frame_size = self.total_device_channels as usize;
        let full_frames = work_data.len() / frame_size;
        let used = full_frames * frame_size;

        // Save any partial frame for next time
        if used < work_data.len() {
            self.frame_remainder.extend_from_slice(&work_data[used..]);
        }

        let frame_data = &work_data[..used];
        let ch_count = self.channel_count as usize;

        // Reset only active channels in peak scratch buffer (no heap alloc)
        for p in &mut self.peak_scratch[..ch_count] {
            *p = 0.0;
        }
        self.refresh_active_channels(frame_size);

        let write_failures =
            if self.monitor_only || (self.gate_enabled && self.gate_state == GateState::Idle) {
                // Monitor mode or gate idle: only track peaks, no disk writes
                self.track_peaks(frame_data, frame_size);
                // DOLL-465: retain this batch while the gate is idle — if it's
                // the one that trips the gate, `process_gate_open` replays it so
                // the signal onset isn't lost. Last batch wins; clear+extend
                // reuses the allocation. Bounded by the caller's batch size
                // (`WRITER_THREAD_READ_CHUNK` in production).
                //
                // Once an earlier slice has tripped the gate, append instead:
                // `read_available` calls this twice when a read wraps the
                // ring, and the gate only opens after both slices, so the
                // second slice must not replace the onset. That bounds the
                // pre-roll at one read (both slices).
                if self.gate_enabled && !self.monitor_only && self.gate_state == GateState::Idle {
                    if !self.gate_pending_open {
                        gate_preroll.clear();
                    }
                    gate_preroll.extend_from_slice(frame_data);
                }
                0
            } else {
                match self.output_mode {
                    OutputMode::Split => self.write_split_frames(frame_data, frame_size),
                    OutputMode::Single => self.write_single_frames(frame_data, frame_size),
                }
            };

        self.note_write_failures(write_failures, full_frames);
        self.publish_peaks(ch_count);
        if self.gate_enabled && !self.monitor_only {
            self.update_gate(ch_count, full_frames);
        }

        // DOLL-465: reattach the buffers detached at the top.
        self.gate_preroll = gate_preroll;
        self.combined_buf = combined_buf;
    }

    /// Rebuild the cached list of channel positions that exist on a device
    /// with `frame_size` channels.
    ///
    /// Pre-filters `channels` to those in range for this device's frame size.
    /// This hoists the bounds check OUT of the per-frame loops so
    /// `frame.get_unchecked` is sound in the hot path (DOLL-126). Out-of-range
    /// channels (e.g. a config that requested ch5 on a 2-channel device) are
    /// skipped for the batch — same graceful-skip behavior the prior
    /// `frame.get()` Option-match produced.
    ///
    /// DOLL-375: the active set is a pure function of the (immutable) channel
    /// list and `frame_size`, so it is cached and rebuilt only when
    /// `frame_size` changes — instead of zero-initialising a 255-byte array
    /// and re-scanning every channel on every `write_samples` call.
    #[inline]
    fn refresh_active_channels(&mut self, frame_size: usize) {
        if self.active_cache_frame_size == frame_size {
            return;
        }
        let mut count = 0_usize;
        // `channels` holds at most MAX_CHANNELS (255) entries, so every
        // position fits a u8 index.
        let ch_slice = &self.channels[..self.channel_count as usize];
        for (idx, &channel) in (0_u8..=u8::MAX).zip(ch_slice) {
            if (channel as usize) < frame_size {
                self.active_indices[count] = idx;
                count += 1;
            }
        }
        self.active_count = count;
        self.active_cache_frame_size = frame_size;
    }

    /// Track per-channel peaks for a batch of whole frames without writing.
    #[inline]
    fn track_peaks(&mut self, frame_data: &[f32], frame_size: usize) {
        let ch_slice = &self.channels[..self.channel_count as usize];
        let active_idx_slice = &self.active_indices[..self.active_count];
        for frame in frame_data.chunks_exact(frame_size) {
            for &active_idx in active_idx_slice {
                let idx = active_idx as usize;
                let channel = ch_slice[idx] as usize;
                // SAFETY: `refresh_active_channels` guaranteed
                // `channel < frame_size`, and `chunks_exact` yields frames of
                // exactly `frame_size` samples — so `channel` is in bounds.
                let s = unsafe { *frame.get_unchecked(channel) };
                if s.is_finite() {
                    self.peak_scratch[idx] = self.peak_scratch[idx].max(s.abs());
                }
            }
        }
    }

    /// Split mode: write each active channel of a batch to its own file and
    /// track peaks. Returns how many samples failed to write (DOLL-349).
    #[inline]
    fn write_split_frames(&mut self, frame_data: &[f32], frame_size: usize) -> u64 {
        let scale = self.sample_scale;
        // DOLL-373: dither 16-bit output. The RNG state lives in a local so the
        // per-sample draws don't borrow `self` inside the loop, and is written
        // back after it.
        let dither = self.bits_per_sample == 16;
        let mut rng = self.dither_rng;
        let mut write_failures = 0_u64;
        let ch_slice = &self.channels[..self.channel_count as usize];
        let active_idx_slice = &self.active_indices[..self.active_count];
        for frame in frame_data.chunks_exact(frame_size) {
            for &active_idx in active_idx_slice {
                let idx = active_idx as usize;
                let channel = ch_slice[idx] as usize;
                // SAFETY: see `track_peaks`.
                let s = unsafe { *frame.get_unchecked(channel) };
                if s.is_finite() {
                    self.peak_scratch[idx] = self.peak_scratch[idx].max(s.abs());
                }
                if let Some(w) = &mut self.multichannel_writers[idx]
                    && w.write_sample(convert_sample(s, scale, dither, &mut rng))
                        .is_err()
                {
                    self.write_errors.fetch_add(1, Ordering::Relaxed);
                    write_failures += 1;
                }
            }
        }
        self.dither_rng = rng;
        write_failures
    }

    /// Single mode: write a batch's active channels interleaved into one file
    /// and track peaks. Returns how many samples failed to write (DOLL-349).
    #[inline]
    fn write_single_frames(&mut self, frame_data: &[f32], frame_size: usize) -> u64 {
        let Some(w) = self.writer.as_mut() else {
            return 0;
        };
        let scale = self.sample_scale;
        // DOLL-373: see `write_split_frames`.
        let dither = self.bits_per_sample == 16;
        let mut rng = self.dither_rng;
        let mut write_failures = 0_u64;
        let ch_slice = &self.channels[..self.channel_count as usize];
        let active_idx_slice = &self.active_indices[..self.active_count];
        for frame in frame_data.chunks_exact(frame_size) {
            for &active_idx in active_idx_slice {
                let idx = active_idx as usize;
                let channel = ch_slice[idx] as usize;
                // SAFETY: see `track_peaks`.
                let s = unsafe { *frame.get_unchecked(channel) };
                if s.is_finite() {
                    self.peak_scratch[idx] = self.peak_scratch[idx].max(s.abs());
                }
                if w.write_sample(convert_sample(s, scale, dither, &mut rng))
                    .is_err()
                {
                    self.write_errors.fetch_add(1, Ordering::Relaxed);
                    write_failures += 1;
                }
            }
        }
        self.dither_rng = rng;
        write_failures
    }

    /// DOLL-349: react to persistent `write_sample` failures (disk full or
    /// otherwise unwritable).
    ///
    /// Previously every failed write just bumped the shared overflow counter
    /// and recording spun on forever, persisting no audio and misreporting the
    /// cause as CPU "heavy load". Track consecutive failures and, once they add
    /// up to ~1s of audio, stop like the disk-low path (finalize what landed,
    /// latch `disk_stopped`), and raise the distinct `write_failed` flag
    /// (DOLL-437) so the UI shows "unable to write to disk" rather than the
    /// low-space message. A batch that writes cleanly clears the streak.
    fn note_write_failures(&mut self, write_failures: u64, full_frames: usize) {
        if write_failures > 0 {
            self.consecutive_write_failures = self
                .consecutive_write_failures
                .saturating_add(write_failures);
            if !self.disk_stopped && self.consecutive_write_failures >= u64::from(self.sample_rate)
            {
                warn!(
                    "Persistent write failures ({} samples) — stopping recording \
                     (disk full or output directory unwritable)",
                    self.consecutive_write_failures
                );
                self.latch_write_failed_stop();
            }
        } else if full_frames > 0 {
            self.consecutive_write_failures = 0;
        }
    }

    /// Publish peaks to the shared atomics (only active channels, not the full
    /// array).
    ///
    /// Each slot keeps the maximum since the meter last read it (`raise`),
    /// instead of this batch's peak: the UI polls at ~30 Hz while batches
    /// arrive every few ms, so overwriting showed only the last batch before
    /// each poll and missed clips and transients (a tiny second slice at a
    /// ring wrap could even zero it). Readers reset with `take`.
    ///
    /// `peak_levels.len() == ch_count` (both derived from the same channel list
    /// at construction), so the zip is exhaustive over the active channels
    /// with no bounds check per iteration.
    #[inline]
    fn publish_peaks(&self, ch_count: usize) {
        for (peak_slot, &peak) in self
            .peak_levels
            .iter()
            .zip(self.peak_scratch[..ch_count].iter())
        {
            peak_slot.raise(peak);
        }
    }

    /// Silence gate transitions for the batch just processed.
    ///
    /// Only sets `gate_pending_open` / `gate_pending_close`; the main loop
    /// opens and finalizes writers, which keeps `write_samples` free of file
    /// I/O.
    fn update_gate(&mut self, ch_count: usize, full_frames: usize) {
        let max_peak = self.peak_scratch[..ch_count]
            .iter()
            .copied()
            .fold(0.0_f32, f32::max);
        let has_signal = max_peak > self.silence_threshold;

        match self.gate_state {
            GateState::Idle => {
                if has_signal {
                    self.gate_pending_open = true;
                    self.gate_silence_frames = 0;
                }
            }
            GateState::Recording => {
                if has_signal {
                    self.gate_silence_frames = 0;
                } else {
                    self.gate_silence_frames += full_frames as u64;
                    if self.gate_silence_frames >= self.gate_timeout_frames {
                        self.gate_pending_close = true;
                    }
                }
            }
        }
    }

    /// Process a pending gate open: create WAV files and transition to Recording.
    /// Called from the main loop (or tests) after `write_samples` sets `gate_pending_open`.
    pub(crate) fn process_gate_open(&mut self) {
        if !self.gate_pending_open {
            return;
        }
        self.gate_pending_open = false;
        info!("Silence gate: signal detected, opening writers");
        if let Err(e) = self.open_writers_for_gate() {
            // Same unwritable-output stop as a failed rotation (DOLL-444).
            // Leaving the gate Idle used to retry on every batch with signal
            // (an error log per ~ms) while the UI showed "recording" and no
            // audio reached disk; in split mode each retry also truncated
            // the channel files the previous attempt had opened.
            error!(
                "Silence gate: failed to open writers: {e} — stopping recording \
                 (disk full or output directory unwritable)"
            );
            self.discard_unwritten_files();
            self.gate_preroll.clear();
            self.latch_write_failed_stop();
        } else {
            self.gate_state = GateState::Recording;
            // Status flag only; no synchronizes-with relationship.
            self.gate_idle.store(false, Ordering::Relaxed);
            // Restart the cadence clock so this file gets a full period
            // (see `rotation_restart`). Relaxed: the RT side's Release clear
            // is what `take_due_rotation` synchronizes with.
            self.rotation_restart.store(true, Ordering::Relaxed);
            // DOLL-465: replay the retained idle batch — the signal onset
            // that tripped the gate — into the just-opened writers before
            // live samples resume. take() empties the field first so the
            // re-entrant write_samples (gate is now Recording) can't
            // re-save or double-write it.
            //
            // The pre-roll is whole frames; a trailing partial frame of the
            // idle batch sits in `frame_remainder` and comes *after* it in
            // time. Set it aside for the replay, or write_samples would
            // prepend it and write the onset shifted by a partial frame
            // (channels rotated). It is restored for the next live batch.
            let mut preroll = std::mem::take(&mut self.gate_preroll);
            if !preroll.is_empty() {
                let remainder = std::mem::take(&mut self.frame_remainder);
                self.write_samples(&preroll);
                self.frame_remainder = remainder;
                // Hand the allocation back for the next idle period.
                preroll.clear();
                self.gate_preroll = preroll;
            }
        }
    }

    /// Process a pending gate close: finalize WAV files and transition to Idle.
    /// Called from the main loop (or tests) after `write_samples` sets `gate_pending_close`.
    pub(crate) fn process_gate_close(&mut self) {
        if !self.gate_pending_close {
            return;
        }
        self.gate_pending_close = false;
        info!(
            "Silence gate: timeout reached ({} frames), finalizing files",
            self.gate_silence_frames
        );
        if let Err(e) = self.finalize_all() {
            error!("Silence gate: finalize error: {e}");
        }
        self.gate_state = GateState::Idle;
        // Status flag only; no synchronizes-with relationship.
        self.gate_idle.store(true, Ordering::Relaxed);
        self.gate_silence_frames = 0;
        // DOLL-465: drop stale pre-roll from the closed session (the next
        // idle batch overwrites it anyway; this keeps the buffer empty
        // rather than holding already-written audio).
        self.gate_preroll.clear();
    }

    /// Open WAV writers when the silence gate transitions from Idle to Recording.
    fn open_writers_for_gate(&mut self) -> Result<(), BlackboxError> {
        let ch_count = self.channel_count as usize;
        match self.output_mode {
            OutputMode::Split => self.setup_split_mode()?,
            OutputMode::Single if ch_count <= 2 => self.setup_standard_mode()?,
            OutputMode::Single => self.setup_multichannel_mode()?,
        }
        Ok(())
    }

    /// Close and delete the files a failed gate open managed to create.
    ///
    /// The gate opens files only from Idle, when nothing is pending, so every
    /// pending pair belongs to the failed open and holds no audio yet (the
    /// pre-roll is replayed only after a successful open). Renaming them
    /// would leave empty takes next to the error.
    fn discard_unwritten_files(&mut self) {
        self.writer = None;
        for writer in &mut self.multichannel_writers {
            *writer = None;
        }
        for (tmp_path, _) in std::mem::take(&mut self.pending_files) {
            if let Err(e) = fs::remove_file(&tmp_path)
                && e.kind() != std::io::ErrorKind::NotFound
            {
                warn!("Could not remove unused {tmp_path}: {e}");
            }
        }
    }

    /// Latch the unwritable-output self-stop shared by the persistent
    /// write-failure path (DOLL-349/437) and file-creation failure on
    /// rotation (DOLL-444) or silence-gate open: raise the `write_failed` status flag so the
    /// UI reports a disk error, set `disk_stopped` so `write_samples` /
    /// `rotate_files` become no-ops, and finalize so every sample that
    /// reached disk is preserved under its final name.
    fn latch_write_failed_stop(&mut self) {
        // status flag only; reader loads Relaxed. (DOLL-437)
        self.write_failed.store(true, Ordering::Relaxed);
        self.disk_stopped = true;
        if let Err(e) = self.finalize_all() {
            error!("Error finalizing after write failure: {e}");
        }
    }

    /// Rotate files: finalize current writers, rename, check silence, create new writers.
    ///
    /// DOLL-444: if creating any next-period file fails, this latches the
    /// `write_failed` self-stop instead of carrying on with a `None`
    /// writer — `write_samples` skips absent writers without even
    /// bumping `write_errors`, so the old behavior silently discarded
    /// every subsequent sample while the UI kept showing "recording".
    pub(crate) fn rotate_files(&mut self) {
        // DOLL-350: once a disk-low self-stop has fired, the writer thread keeps
        // looping to receive Shutdown. In continuous mode the RT callback still
        // sets rotation_needed, so without this guard each rotation would call
        // create_wav_writer again — recreating empty `.recording.wav` temp files
        // that write_samples (which early-returns on disk_stopped) never fills,
        // renames, or cleans up. They'd pile up on the already-full disk.
        if self.disk_stopped {
            return;
        }
        // No-op when gate is idle (no files to rotate)
        if self.gate_enabled && self.gate_state == GateState::Idle {
            return;
        }
        info!("Rotating recording files...");

        // Errors are logged inside; rotation carries on with the next period.
        let (checkable, _) = self.close_files();

        // Hand the recently-rotated files to the dedicated silence-check
        // worker. The writer thread immediately resumes draining the ring
        // buffer during rotation; silence detection happens off-thread.
        if !checkable.is_empty()
            && let Some(worker) = self.silence_worker.as_ref()
        {
            worker.submit(checkable);
        }

        // Create new files for the next recording period. Any creation
        // failure latches the write_failed self-stop (DOLL-444).
        if !self.open_next_period_files() {
            warn!(
                "Could not create next recording file(s) during rotation — \
                 stopping recording (disk full or output directory unwritable)"
            );
            self.latch_write_failed_stop();
        }
    }

    /// Rotate when an open file's audio data has reached `max_data_bytes`.
    ///
    /// Called by the main loop after every read. Without it a long take
    /// (the app records single files by default: stereo 24-bit 48 kHz
    /// reaches 4 GiB in about 4 hours, 8 channels in about 1) kept growing
    /// past what the `u32` WAV header can describe, and players read only
    /// the first 4 GiB (DOLL-204).
    pub(crate) fn rotate_if_file_full(&mut self) {
        let largest = self
            .writer
            .iter()
            .chain(self.multichannel_writers.iter().flatten())
            .map(RawWavWriter::data_bytes)
            .max()
            .unwrap_or(0);
        if largest >= self.max_data_bytes {
            info!("Recording file reached {largest} bytes of audio; continuing in a new file");
            self.rotate_files();
        }
    }

    /// Finalize every writer and rename every pending `.recording.wav` to
    /// its final name, attempting all of them even when one fails (DOLL-345).
    ///
    /// Returns the renamed files that may go to the silence check, and the
    /// first error. A file whose `finalize` failed is still renamed (its audio
    /// is kept under its final name) but is left out of the silence check:
    /// its header may not describe the audio on disk, and a header still at
    /// the placeholder reads as zero samples, which must never get a take
    /// deleted as "silent".
    fn close_files(&mut self) -> (Vec<String>, Option<BlackboxError>) {
        let mut first_err: Option<BlackboxError> = None;
        let mut unfinalized: Vec<String> = Vec::new();

        let writers = self.writer.take().into_iter().chain(
            self.multichannel_writers
                .iter_mut()
                .filter_map(Option::take),
        );
        for writer in writers {
            let path = writer.path().to_owned();
            if let Err(e) = writer.finalize() {
                let err = BlackboxError::Wav(format!("Error finalizing WAV file {path}: {e}"));
                error!("{err}");
                first_err.get_or_insert(err);
                unfinalized.push(path);
            }
        }

        // Rename all pending .recording.wav files to their final .wav paths —
        // attempt every one; a single rename failure must not strand the rest.
        let mut checkable = Vec::new();
        for (tmp_path, final_path) in std::mem::take(&mut self.pending_files) {
            if !Path::new(&tmp_path).exists() {
                continue;
            }
            match fs::rename(&tmp_path, &final_path) {
                Ok(()) => {
                    info!("Finalized recording to {final_path}");
                    if unfinalized.contains(&tmp_path) {
                        warn!("Keeping {final_path} without a silence check: its finalize failed");
                    } else {
                        checkable.push(final_path);
                    }
                }
                Err(e) => {
                    error!("Error renaming {tmp_path} to {final_path}: {e}");
                    first_err.get_or_insert_with(|| BlackboxError::from(e));
                }
            }
        }
        (checkable, first_err)
    }

    /// Open the next period's files. Returns `false` if any could not be
    /// created; the ones that did open stay open (DOLL-444).
    fn open_next_period_files(&mut self) -> bool {
        let ch_count = self.channel_count as usize;
        let date_str = (self.timestamp_fn)();
        match self.output_mode {
            OutputMode::Split => {
                let mut all_created = true;
                for idx in 0..ch_count {
                    let final_path =
                        self.period_path(&date_str, &format!("-ch{}", self.channels[idx]));
                    match self.open_pending_writer(final_path, self.mono_spec(), "channel WAV file")
                    {
                        Ok(w) => self.multichannel_writers[idx] = Some(w),
                        Err(e) => {
                            error!("Failed to create channel WAV file: {e}");
                            all_created = false;
                        }
                    }
                }
                all_created
            }
            OutputMode::Single if ch_count > 2 => {
                let final_path = self.period_path(&date_str, "-multichannel");
                let spec = self.multichannel_spec();
                match self.open_pending_writer(final_path, spec, "multichannel WAV file") {
                    Ok(w) => {
                        self.writer = Some(w);
                        true
                    }
                    Err(e) => {
                        error!("Failed to create multichannel WAV file: {e}");
                        false
                    }
                }
            }
            OutputMode::Single => {
                let final_path = self.period_path(&date_str, "");
                match self.open_pending_writer(final_path, self.current_spec, "new recording file")
                {
                    Ok(w) => {
                        self.writer = Some(w);
                        true
                    }
                    Err(e) => {
                        error!("Failed to create new WAV file: {e}");
                        false
                    }
                }
            }
        }
    }

    /// Finalize all writers — called on shutdown after draining the ring buffer.
    ///
    /// DOLL-345: attempt to finalize+rename *every* writer and pending pair
    /// before returning, mirroring `rotate_files`'s log-and-continue. The
    /// previous `?`-on-first-error version meant that in `OutputMode::Split`
    /// (the app default) a failure on channel 0 left channels 1..n unfinalized
    /// with their audio stranded under `.recording.wav` temp names. We still
    /// surface a failure (the first error) so callers know something went
    /// wrong, but only after giving every file its best chance to land.
    pub(crate) fn finalize_all(&mut self) -> Result<(), BlackboxError> {
        let (checkable, first_err) = self.close_files();

        // Hand finalized files to the silence-check worker without
        // blocking: this runs on the way to a stop, and the stop must not
        // wait behind scans already queued. The worker keeps scanning after
        // the state is dropped; a file it never gets to is simply kept.
        if !checkable.is_empty()
            && let Some(worker) = self.silence_worker.as_ref()
        {
            worker.try_submit(checkable);
        }

        first_err.map_or(Ok(()), Err)
    }
}

// ---------------------------------------------------------------------------
// Writer thread main loop
// ---------------------------------------------------------------------------

/// Read and process a chunk from the consumer. Returns the number of samples read.
///
/// Exposed (crate-internal; this module is private) so tests can drive a
/// controlled ring-buffer wraparound (DOLL-355).
pub(crate) fn read_available(
    consumer: &mut rtrb::Consumer<f32>,
    state: &mut WriterThreadState,
) -> usize {
    let available = consumer.slots();
    if available == 0 {
        return 0;
    }
    let to_read = available.min(WRITER_THREAD_READ_CHUNK);
    consumer.read_chunk(to_read).map_or(0, |chunk| {
        let n = chunk.len();
        let (first, second) = chunk.as_slices();
        state.write_samples(first);
        if !second.is_empty() {
            state.write_samples(second);
        }
        chunk.commit_all();
        // Bump cumulative-sample counter so tests can rendezvous on
        // "writer has consumed N samples" instead of `thread::sleep`
        // (DOLL-127). Test-only — gated out of production builds (DOLL-269).
        #[cfg(test)]
        state
            .samples_consumed_total
            .fetch_add(n as u64, Ordering::Relaxed);
        n
    })
}

/// Check each file for silence and delete silent ones. Used by both the
/// background silence-check thread (during rotation) and the synchronous
/// finalize path (during shutdown).
pub(crate) fn check_and_delete_silent_files(files: &[String], threshold: f32) {
    for file_path in files {
        match is_silent(file_path, threshold) {
            Ok(true) => {
                info!(
                    "Recording is silent (below threshold {threshold}), deleting file: {file_path}"
                );
                if let Err(e) = fs::remove_file(file_path) {
                    error!("Error deleting silent file: {e}");
                }
            }
            Ok(false) => {
                info!(
                    "Recording is not silent (above threshold {threshold}), keeping file: {file_path}"
                );
            }
            Err(e) => {
                error!("Error checking for silence: {e}");
            }
        }
    }
}

/// Consume the RT callback's rotation flag and say whether to rotate now.
///
/// While `restart` is still set the callback has not yet restarted its
/// cadence counter after a gate open, so any flag it raised belongs to the
/// period before the open: it is cleared and ignored. The callback clears
/// `rotation_needed` before its Release store of `restart = false`; the
/// Acquire load here means a `false` observation also sees that clear, so a
/// flag read afterwards was raised by the restarted counter.
pub(crate) fn take_due_rotation(rotation_needed: &AtomicBool, restart: &AtomicBool) -> bool {
    let restart_pending = restart.load(Ordering::Acquire);
    // Relaxed: status flag only, no companion data to acquire (DOLL-391).
    let due = rotation_needed.swap(false, Ordering::Relaxed);
    due && !restart_pending
}

fn drain_remaining(consumer: &mut rtrb::Consumer<f32>, state: &mut WriterThreadState) {
    loop {
        if read_available(consumer, state) == 0 {
            break;
        }
    }
}

pub(crate) fn writer_thread_main(
    mut consumer: rtrb::Consumer<f32>,
    rotation_needed: &AtomicBool,
    command_rx: &std::sync::mpsc::Receiver<WriterCommand>,
    mut state: WriterThreadState,
) {
    // Why poll-sleep instead of blocking on a condvar/channel (DOLL-270):
    // the only producer into the ring buffer is the cpal audio callback, which
    // runs on a real-time thread. Signalling a condvar / waking a parked thread
    // from there means taking a lock and potentially making a syscall in the RT
    // path — exactly the non-real-time work that causes audio dropouts (cf.
    // DOLL-250). So the producer stays lock/syscall-free and just pushes into
    // the lock-free rtrb queue; this thread polls it. The cost is bounded:
    //
    // - Adaptive sleep below backs off 1ms → 5ms as the queue stays empty, so
    //   worst-case drain latency is ~5ms and idle wakeups drop ~5×.
    // - The shutdown path is a non-blocking `try_recv` each iteration, so a
    //   Shutdown command is observed within one sleep interval.
    //
    // A blocking design would shave the few-ms latency but at the cost of
    // RT-safety, which is the wrong trade for an audio capture path.
    let mut consecutive_empty: u8 = 0;

    loop {
        // 1. Check for shutdown command (non-blocking). A disconnected
        //    channel is an implicit shutdown (DOLL-447): if the processor
        //    drops its WriterThreadHandle without sending Shutdown — a
        //    failed start tearing down after build_input_stream / play()
        //    errors, or a replaced handle — the old `if let Ok(..)`
        //    ignored TryRecvError::Disconnected and this thread spun
        //    forever: leaked for the process lifetime, waking every 5 ms,
        //    holding its `.recording.wav` temp files open and unrenamed.
        match command_rx.try_recv() {
            Ok(WriterCommand::Shutdown(reply_tx)) => {
                // Drain remaining samples from ring buffer
                drain_remaining(&mut consumer, &mut state);
                let result = state.finalize_all();
                if reply_tx.send(result).is_err() {
                    warn!("finalize requester hung up before the reply was sent");
                }
                return;
            }
            Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                warn!(
                    "Writer command channel disconnected without Shutdown — \
                     draining and finalizing"
                );
                drain_remaining(&mut consumer, &mut state);
                if let Err(e) = state.finalize_all() {
                    error!("Error finalizing after command channel disconnect: {e}");
                }
                return;
            }
            Err(std::sync::mpsc::TryRecvError::Empty) => {}
        }

        // 2. Check disk space periodically
        state.check_disk_space();

        // 3. Check rotation flag (set by RT callback via AtomicBool).
        // Relaxed: status flag only, no companion data to acquire (DOLL-391).
        if take_due_rotation(rotation_needed, &state.rotation_restart) {
            state.rotate_files();
        }

        // 3b. Process deferred gate transitions (keeps write_samples free of file I/O)
        state.process_gate_open();
        state.process_gate_close();

        // 4. Read available samples from ring buffer
        let read = read_available(&mut consumer, &mut state);

        // 4b. Start a new file before any open one outgrows the WAV size
        //     field. One read adds at most WRITER_THREAD_READ_CHUNK samples,
        //     well inside the margin below the limit.
        state.rotate_if_file_full();

        // 5. Periodic flush — writes valid WAV headers for crash recovery
        state.flush_writers(read);

        if read == 0 {
            // Ring buffer empty — back off gradually to reduce idle wakeups.
            // 1ms → 2ms → 3ms → 4ms → 5ms (cap). Resets on data arrival.
            consecutive_empty = consecutive_empty.saturating_add(1).min(5);
            #[expect(
                clippy::disallowed_methods,
                reason = "bounded 1-5 ms idle backoff, not a rendezvous: the RT producer must stay lock- and syscall-free (DOLL-270), so there is nothing to block on"
            )]
            std::thread::sleep(Duration::from_millis(u64::from(consecutive_empty)));
        } else {
            consecutive_empty = 0;
        }
    }
}

/// Benchmark helper: drive the REAL production write pipeline end-to-end.
///
/// Unlike the `single`/`split` bench modes (which use `hound::WavWriter` for
/// relative comparison only), this routes samples through the exact shipped
/// path — an `rtrb` ring buffer feeding a spawned `writer_thread_main`,
/// which uses `WriterThreadState` + `RawWavWriter`, the adaptive-sleep
/// drain loop, and `WRITER_THREAD_READ_CHUNK` sizing — set up identically to
/// `CpalAudioProcessor::process_audio_impl`. This is what the CI throughput
/// floor asserts on, so a regression in any of those guards shipped code
/// (DOLL-251; supersedes the divergent hand-rolled loop noted in DOLL-192).
///
/// Pushes `chunk_data` (one interleaved cpal-callback-sized chunk) repeatedly
/// until `total_frames` have been produced, retrying the whole chunk on ring
/// back-pressure (same semantics as the RT producer, minus the sample drop),
/// then sends `Shutdown` and waits for the writer to drain + finalize.
///
/// Uses the default recording configuration: 24-bit, single-file output,
/// silence detection and the silence gate off. Returns the wall-clock elapsed
/// from first push to writer-thread join, plus the total write-error count.
///
/// # Panics
///
/// If the writer state or thread cannot be created. This is a benchmark
/// harness, so a failed setup is a harness bug rather than a runtime condition.
#[cfg(feature = "benchmarking")]
#[must_use]
#[expect(
    clippy::expect_used,
    reason = "benchmark harness: a failed setup is a harness bug, not a runtime condition"
)]
pub fn bench_real_pipeline(
    output_dir: &str,
    sample_rate: u32,
    channels: usize,
    total_frames: usize,
    chunk_data: &[f32],
) -> (Duration, u64) {
    let channel_indices: Vec<usize> = (0..channels).collect();
    let write_errors = Arc::new(AtomicU64::new(0));
    let disk_space_low = Arc::new(AtomicBool::new(false));
    let peak_levels: Arc<[CacheAlignedPeak]> = std::iter::repeat_with(|| CacheAlignedPeak::new(0))
        .take(channels)
        .collect();

    let mut state = WriterThreadState::new(
        output_dir,
        sample_rate,
        &channel_indices,
        OutputMode::Single,
        0.0, // silence_threshold off
        Arc::clone(&write_errors),
        0, // min_disk_space_mb off
        disk_space_low,
        24, // bits_per_sample
        peak_levels,
        false, // gate_enabled
        0,     // gate_timeout_secs
    )
    .expect("failed to build writer state");
    state.total_device_channels = u16::try_from(channels).expect("channel count fits in u16");

    // Ring buffer sized exactly as production (see process_audio_impl).
    let ring_size = sample_rate as usize * channels * crate::RING_BUFFER_SECONDS;
    let (mut producer, consumer) = rtrb::RingBuffer::new(ring_size);
    let rotation_needed = Arc::new(AtomicBool::new(false));
    let (command_tx, command_rx) = std::sync::mpsc::sync_channel::<WriterCommand>(1);

    let writer_handle = std::thread::Builder::new()
        .name("bench-real-writer".to_owned())
        .spawn(move || writer_thread_main(consumer, &rotation_needed, &command_rx, state))
        .expect("failed to spawn writer thread");

    let chunk_frames = chunk_data.len() / channels;
    let start = std::time::Instant::now();
    let mut frames_written = 0;
    while frames_written < total_frames {
        let frames_this_chunk = chunk_frames.min(total_frames - frames_written);
        let data = &chunk_data[..frames_this_chunk * channels];
        // Retry the whole chunk on back-pressure, like the RT producer (which
        // would instead drop). The ring is multi-second, so this is rare.
        if let Ok(chunk) = producer.write_chunk_uninit(data.len()) {
            chunk.fill_from_iter(data.iter().copied());
            frames_written += frames_this_chunk;
        } else {
            std::thread::yield_now();
        }
    }

    // Drain + finalize through the real shutdown path, then join.
    let (reply_tx, reply_rx) = std::sync::mpsc::channel();
    if command_tx.send(WriterCommand::Shutdown(reply_tx)).is_err() {
        warn!("bench writer exited before the shutdown command was sent");
    }
    if reply_rx.recv().is_err() {
        warn!("bench writer exited without a shutdown reply");
    }
    if writer_handle.join().is_err() {
        warn!("bench writer thread panicked");
    }
    let elapsed = start.elapsed();

    (elapsed, write_errors.load(Ordering::Relaxed))
}

#[cfg(test)]
mod disambiguate_tests {
    use super::*;
    use std::fs;
    use tempfile::tempdir;

    #[test]
    fn returns_same_path_when_no_collision() {
        let dir = tempdir().unwrap();
        let p = dir.path().join("rec.wav").to_str().unwrap().to_owned();
        assert_eq!(disambiguate_path(&p), p);
    }

    #[test]
    fn appends_suffix_when_path_exists() {
        let dir = tempdir().unwrap();
        let p = dir.path().join("rec.wav").to_str().unwrap().to_owned();
        fs::write(&p, b"x").unwrap();

        let got = disambiguate_path(&p);
        assert_ne!(got, p, "must not return the colliding path");
        assert!(
            got.ends_with("-1.wav"),
            "first collision should be -1.wav, got {got}"
        );
        assert!(!Path::new(&got).exists(), "disambiguated path must be free");
    }

    #[test]
    fn skips_already_taken_suffixes() {
        let dir = tempdir().unwrap();
        let base = dir.path().join("rec.wav").to_str().unwrap().to_owned();
        fs::write(&base, b"x").unwrap();
        fs::write(dir.path().join("rec-1.wav"), b"x").unwrap();

        let got = disambiguate_path(&base);
        assert!(
            got.ends_with("-2.wav"),
            "should skip taken -1.wav, got {got}"
        );
    }
}

#[cfg(test)]
mod disk_space_tests {
    use super::*;
    use tempfile::tempdir;

    /// End-to-end test for the "automatically stops recording when free space
    /// drops below the threshold" feature (README) — DOLL-262. Prior tests
    /// only covered the FFI flag plumbing and the throttle counter; nothing
    /// drove `min_disk_space_mb` to actually flip `disk_space_low` and stop
    /// the writer.
    ///
    /// We construct with a tiny threshold (so `new()`'s fail-early disk check
    /// passes on a normal temp filesystem), then raise `min_disk_space_mb` to
    /// `u64::MAX` before the runtime check — guaranteeing the real `statvfs`
    /// reading of the temp dir is below threshold, so `check_disk_space` must
    /// trip. No mocking of the syscall needed.
    #[test]
    fn disk_low_flips_flag_and_stops_writer() {
        let dir = tempdir().unwrap();
        let out = dir.path().to_str().unwrap();

        let disk_low = Arc::new(AtomicBool::new(false));
        let write_errors = Arc::new(AtomicU64::new(0));
        let peak_levels: Arc<[CacheAlignedPeak]> = Arc::from(vec![CacheAlignedPeak::new(0)]);

        let mut state = WriterThreadState::new(
            out,
            48_000,
            &[0],
            OutputMode::Single,
            0.0, // silence detection off (no background worker)
            Arc::clone(&write_errors),
            1, // tiny threshold so new()'s fail-early check passes
            Arc::clone(&disk_low),
            16,
            peak_levels,
            false, // gate disabled → writer is set up immediately
            0,
        )
        .expect("WriterThreadState::new");
        state.total_device_channels = 1;

        // Gate disabled + Single mode + 1 channel → a standard writer exists.
        assert!(state.writer.is_some(), "writer should be set up on new()");
        assert!(!disk_low.load(Ordering::Relaxed));

        // Now make the runtime check trip: any real free space is below u64::MAX.
        state.min_disk_space_mb = u64::MAX;
        // Fast-forward the throttle counter so the next call performs the real
        // statvfs check instead of the every-10,000th-iteration short-circuit.
        state.disk_check_counter = 9_999;
        let keep_going = state.check_disk_space();

        assert!(
            !keep_going,
            "check_disk_space must return false when free space is below threshold"
        );
        assert!(
            disk_low.load(Ordering::Relaxed),
            "disk_space_low flag must flip so the FFI/UI sees the stop"
        );
        assert!(state.disk_stopped, "state must mark itself disk-stopped");
        assert!(
            state.writer.is_none() && state.multichannel_writers.is_empty(),
            "writers must be finalized after a disk-low stop — no further files produced"
        );

        // Idempotent: once stopped, subsequent checks stay stopped and the
        // throttle counter is irrelevant (early-return on disk_stopped).
        assert!(!state.check_disk_space());
        assert!(disk_low.load(Ordering::Relaxed));
    }

    /// Control: with `min_disk_space_mb == 0` (the disk check disabled),
    /// `check_disk_space` always reports OK and never flips the flag, even
    /// once the throttle counter would otherwise trigger a real check.
    #[test]
    fn disk_check_disabled_never_stops() {
        let dir = tempdir().unwrap();
        let out = dir.path().to_str().unwrap();

        let disk_low = Arc::new(AtomicBool::new(false));
        let write_errors = Arc::new(AtomicU64::new(0));
        let peak_levels: Arc<[CacheAlignedPeak]> = Arc::from(vec![CacheAlignedPeak::new(0)]);

        let mut state = WriterThreadState::new(
            out,
            48_000,
            &[0],
            OutputMode::Single,
            0.0,
            Arc::clone(&write_errors),
            0, // disk check disabled
            Arc::clone(&disk_low),
            16,
            peak_levels,
            false,
            0,
        )
        .expect("WriterThreadState::new");
        state.total_device_channels = 1;

        state.disk_check_counter = 9_999;
        assert!(
            state.check_disk_space(),
            "disabled check should always continue"
        );
        assert!(
            !disk_low.load(Ordering::Relaxed),
            "flag must not flip when disabled"
        );
        assert!(!state.disk_stopped);
        assert!(state.writer.is_some(), "writer should remain active");
    }
}
