use std::ffi::CString;
use std::fs;
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::Duration;

use log::{error, info, warn};

use crate::constants::{CacheAlignedPeak, MAX_CHANNELS, OutputMode, WRITER_THREAD_READ_CHUNK};
use crate::error::BlackboxError;
use crate::raw_wav_writer::RawWavWriter;
use crate::silence_check_worker::SilenceCheckWorker;
use crate::utils::{available_disk_space_mb, is_silent};

use chrono::prelude::*;

// ---------------------------------------------------------------------------
// File-rotation helpers — timestamp formatting and tmp/.wav path derivation.
// ---------------------------------------------------------------------------

/// Returns a timestamp string like "2024-01-15-14-30-05" from the current local time.
/// Includes seconds so that file rotations within the same minute produce distinct names.
pub fn timestamp_now() -> String {
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
        return final_path.to_string();
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
        "Path disambiguation exhausted (>1000 collisions) for {}; using {}",
        final_path,
        candidate
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
pub enum GateState {
    /// No audio signal — WAV files are closed, only tracking peaks.
    Idle,
    /// Audio signal present — WAV files are open and writing.
    Recording,
}

// ---------------------------------------------------------------------------
// WriterCommand — sent from the processor to the writer thread
// ---------------------------------------------------------------------------

pub enum WriterCommand {
    /// Drain remaining samples and finalize all files.
    Shutdown(std::sync::mpsc::Sender<Result<(), BlackboxError>>),
}

// ---------------------------------------------------------------------------
// WriterThreadHandle — held by CpalAudioProcessor
// ---------------------------------------------------------------------------

pub struct WriterThreadHandle {
    pub command_tx: std::sync::mpsc::SyncSender<WriterCommand>,
    pub join_handle: Option<std::thread::JoinHandle<()>>,
}

// ---------------------------------------------------------------------------
// WriterThreadState — lives entirely on the writer thread
// ---------------------------------------------------------------------------

#[allow(clippy::struct_excessive_bools)]
pub struct WriterThreadState {
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

    // --- Warm fields: accessed frequently but not per-sample ---
    pub writer: Option<RawWavWriter>,
    pub multichannel_writers: Vec<Option<RawWavWriter>>,
    pub write_errors: Arc<AtomicU64>,
    /// Per-channel peak levels (f32 stored as u32 bits via `to_bits()`). Shared with FFI.
    /// Each element is cache-line-aligned to prevent false sharing with the UI reader thread.
    pub peak_levels: Arc<Vec<CacheAlignedPeak>>,
    /// Partial frames carried over between ring buffer reads.
    frame_remainder: Vec<f32>,
    /// Pre-allocated buffer for combining frame_remainder + new data (avoids heap alloc).
    combined_buf: Vec<f32>,
    /// Set by `write_samples` when signal is detected in Idle mode.
    /// The main loop opens writers before the next read, keeping write_samples I/O-free.
    pub gate_pending_open: bool,
    /// Set by `write_samples` when silence timeout is reached in Recording mode.
    /// The main loop finalizes writers, keeping write_samples free of file I/O.
    pub gate_pending_close: bool,
    /// Consecutive silent frames counted while gate is Recording.
    gate_silence_frames: u64,
    /// Frame count threshold for gate timeout (`timeout_secs * sample_rate`).
    gate_timeout_frames: u64,
    /// Shared flag: true when gate is idle (no files open). Read by FFI for status.
    pub gate_idle: Arc<AtomicBool>,

    // --- Cold fields: only accessed during setup, rotation, or shutdown ---
    pub output_dir: String,
    /// Pre-allocated CString of `output_dir` for `statvfs` calls (avoids heap alloc per check).
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
    /// Pluggable timestamp source for filename stamps. Production passes
    /// `Arc::new(timestamp_now)`; tests pass a `MockClock` so rotations
    /// produce distinct filenames without sleeping past a wall-clock second.
    pub(crate) timestamp_fn: TimestampFn,
    /// Single dedicated worker that scans recently-rotated files for
    /// silence and deletes them. `Some` when `silence_threshold > 0`,
    /// `None` otherwise. Joined when `WriterThreadState` is dropped.
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
pub fn f32_to_wav_sample(sample: f32, bits_per_sample: u16) -> i32 {
    let scale = match bits_per_sample {
        16 => f32::from(i16::MAX), // 32767.0
        24 => 8_388_607.0_f32,     // 2^23 - 1
        _ => i32::MAX as f32,      // 2^31 - 1
    };
    (sample.clamp(-1.0, 1.0) * scale).round() as i32
}

impl WriterThreadState {
    /// Create a new `WriterThreadState` with initial WAV writers set up.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        output_dir: &str,
        sample_rate: u32,
        channels: &[usize],
        output_mode: OutputMode,
        silence_threshold: f32,
        write_errors: Arc<AtomicU64>,
        min_disk_space_mb: u64,
        disk_space_low: Arc<AtomicBool>,
        bits_per_sample: u16,
        peak_levels: Arc<Vec<CacheAlignedPeak>>,
        gate_enabled: bool,
        gate_timeout_secs: u64,
    ) -> Result<Self, BlackboxError> {
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

        let sample_scale = match bits_per_sample {
            16 => f32::from(i16::MAX),
            24 => 8_388_607.0_f32,
            _ => i32::MAX as f32,
        };

        // Pack channel indices into a fixed inline array (u8 fits MAX_CHANNELS=255)
        let mut ch_arr = [0_u8; MAX_CHANNELS];
        let channel_count = channels.len().min(MAX_CHANNELS);
        for (i, &ch) in channels.iter().take(MAX_CHANNELS).enumerate() {
            ch_arr[i] = ch as u8;
        }

        let gate_idle = Arc::new(AtomicBool::new(gate_enabled));
        let initial_gate_state = if gate_enabled {
            GateState::Idle
        } else {
            GateState::Recording
        };

        let mut state = WriterThreadState {
            sample_scale,
            monitor_only: false,
            disk_stopped: false,
            gate_enabled,
            gate_state: initial_gate_state,
            output_mode,
            channel_count: channel_count as u8,
            total_device_channels: 0, // set by caller or process_audio
            disk_check_counter: 0,
            flush_frame_counter: 0,
            channels: ch_arr,
            peak_scratch: [0.0_f32; MAX_CHANNELS],
            active_indices: [0_u8; MAX_CHANNELS],
            active_count: 0,
            active_cache_frame_size: usize::MAX,
            writer: None,
            multichannel_writers: Vec::new(),
            write_errors,
            peak_levels,
            frame_remainder: Vec::new(),
            combined_buf: Vec::new(),
            gate_pending_open: false,
            gate_pending_close: false,
            gate_silence_frames: 0,
            gate_timeout_frames: u64::from(sample_rate) * gate_timeout_secs,
            gate_idle,
            output_dir: output_dir.to_string(),
            #[cfg(unix)]
            output_dir_cstr: CString::new(output_dir).ok(),
            sample_rate,
            bits_per_sample,
            current_spec: crate::raw_wav_writer::WavSpec {
                channels: 1,
                sample_rate,
                bits_per_sample,
            },
            pending_files: Vec::new(),
            silence_threshold,
            min_disk_space_mb,
            disk_space_low,
            timestamp_fn: Arc::new(timestamp_now),
            // SilenceCheckWorker::new returns Option (DOLL-122) — spawn
            // failures degrade to "no silence checks this session"
            // rather than crashing the recording.
            silence_worker: if silence_threshold > 0.0 {
                SilenceCheckWorker::new(silence_threshold)
            } else {
                None
            },
            #[cfg(test)]
            samples_consumed_total: Arc::new(AtomicU64::new(0)),
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
    pub fn new_monitor(
        sample_rate: u32,
        channels: &[usize],
        peak_levels: Arc<Vec<CacheAlignedPeak>>,
    ) -> Self {
        let sample_scale = 8_388_607.0_f32; // 24-bit max (monitor mode is always 24-bit)

        // Pack channel indices into inline array
        let mut ch_arr = [0_u8; MAX_CHANNELS];
        let channel_count = channels.len().min(MAX_CHANNELS);
        for (i, &ch) in channels.iter().take(MAX_CHANNELS).enumerate() {
            ch_arr[i] = ch as u8;
        }

        WriterThreadState {
            sample_scale,
            monitor_only: true,
            disk_stopped: false,
            gate_enabled: false,
            gate_state: GateState::Recording,
            output_mode: OutputMode::Single, // unused in monitor mode
            channel_count: channel_count as u8,
            total_device_channels: 0,
            disk_check_counter: 0,
            flush_frame_counter: 0,
            channels: ch_arr,
            peak_scratch: [0.0_f32; MAX_CHANNELS],
            active_indices: [0_u8; MAX_CHANNELS],
            active_count: 0,
            active_cache_frame_size: usize::MAX,
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

        info!("Setting up split mode with {} channels", ch_count);

        self.multichannel_writers.clear();
        for _ in 0..ch_count {
            self.multichannel_writers.push(None);
        }

        for (idx, &channel) in self.channels[..ch_count].iter().enumerate() {
            let final_path = disambiguate_path(&format!(
                "{}/{}-ch{}.wav",
                self.output_dir, date_str, channel
            ));
            let tmp_path = tmp_wav_path(&final_path);

            let spec = crate::raw_wav_writer::WavSpec {
                channels: 1,
                sample_rate: self.sample_rate,
                bits_per_sample: self.bits_per_sample,
            };

            let writer = create_wav_writer(&tmp_path, spec)?;

            self.multichannel_writers[idx] = Some(writer);
            self.pending_files.push((tmp_path, final_path.clone()));
            info!("Created channel WAV file: {}", final_path);
        }

        self.current_spec = crate::raw_wav_writer::WavSpec {
            channels: 1,
            sample_rate: self.sample_rate,
            bits_per_sample: self.bits_per_sample,
        };

        Ok(())
    }

    fn setup_multichannel_mode(&mut self) -> Result<(), BlackboxError> {
        let date_str = (self.timestamp_fn)();
        let ch_count = self.channel_count as usize;

        let final_path = disambiguate_path(&format!(
            "{}/{}-multichannel.wav",
            self.output_dir, date_str
        ));
        let tmp_path = tmp_wav_path(&final_path);

        info!("Setting up multichannel mode with {} channels", ch_count);

        let spec = crate::raw_wav_writer::WavSpec {
            channels: self.channel_count as u16,
            sample_rate: self.sample_rate,
            bits_per_sample: self.bits_per_sample,
        };

        let writer = create_wav_writer(&tmp_path, spec)?;

        self.writer = Some(writer);
        self.current_spec = spec;
        self.pending_files.push((tmp_path, final_path.clone()));

        info!("Created multichannel WAV file: {}", final_path);
        Ok(())
    }

    fn setup_standard_mode(&mut self) -> Result<(), BlackboxError> {
        let date_str = (self.timestamp_fn)();
        let ch_count = self.channel_count as usize;

        info!("Setting up standard mode with {} channels", ch_count);

        let num_channels = if ch_count == 1 { 1 } else { 2 };

        let final_path = disambiguate_path(&format!("{}/{}.wav", self.output_dir, date_str));
        let tmp_path = tmp_wav_path(&final_path);

        let spec = crate::raw_wav_writer::WavSpec {
            channels: num_channels as u16,
            sample_rate: self.sample_rate,
            bits_per_sample: self.bits_per_sample,
        };

        let writer = create_wav_writer(&tmp_path, spec)?;

        self.writer = Some(writer);
        self.current_spec = spec;
        self.pending_files.push((tmp_path, final_path.clone()));

        info!("Created WAV file: {}", final_path);
        Ok(())
    }

    /// Check available disk space and stop writing if below threshold.
    /// Returns true if writing should continue, false if disk is low.
    ///
    /// Uses an iteration counter to amortize the cost: only performs the actual
    /// `statvfs` syscall every 10,000 calls (~4 seconds at typical throughput),
    /// avoiding a `clock_gettime` syscall on every writer thread loop iteration.
    pub fn check_disk_space(&mut self) -> bool {
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
                "Disk space low: {}MB available, threshold is {}MB — stopping recording",
                available_mb, self.min_disk_space_mb
            );
            // status flag only; reader at disk_space_low() loads Relaxed.
            self.disk_space_low.store(true, Ordering::Relaxed);
            self.disk_stopped = true;
            // Finalize current files so data written so far is safe
            if let Err(e) = self.finalize_all() {
                error!("Error finalizing files after disk space warning: {}", e);
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
    pub fn flush_writers(&mut self, samples_consumed: usize) {
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
            error!("Error flushing WAV writer: {}", e);
        }
        for w in self.multichannel_writers.iter_mut().flatten() {
            if let Err(e) = w.flush() {
                error!("Error flushing channel WAV writer: {}", e);
            }
        }
    }

    /// Write interleaved f32 samples to WAV writers.
    ///
    /// Handles partial frames: if `data` doesn't divide evenly by `total_device_channels`,
    /// leftover samples are stored in `frame_remainder` and prepended to the next call.
    /// Also tracks per-channel peak levels for metering.
    pub fn write_samples(&mut self, data: &[f32]) {
        // If total_device_channels is 0 or disk stopped, skip writing
        if self.total_device_channels == 0 || self.disk_stopped {
            return;
        }

        // Prepend any leftover samples from the previous call using a pre-allocated buffer
        let work_data: &[f32] = if self.frame_remainder.is_empty() {
            data
        } else {
            self.combined_buf.clear();
            self.combined_buf.extend_from_slice(&self.frame_remainder);
            self.combined_buf.extend_from_slice(data);
            self.frame_remainder.clear();
            &self.combined_buf
        };

        let frame_size = self.total_device_channels as usize;
        let full_frames = work_data.len() / frame_size;
        let used = full_frames * frame_size;

        // Save any partial frame for next time
        if used < work_data.len() {
            self.frame_remainder.extend_from_slice(&work_data[used..]);
        }

        let frame_data = &work_data[..used];

        // Cache scale factor on the stack for the inner loop
        let scale = self.sample_scale;

        let ch_count = self.channel_count as usize;
        let ch_slice = &self.channels[..ch_count];

        // Reset only active channels in peak scratch buffer (no heap alloc)
        for p in &mut self.peak_scratch[..ch_count] {
            *p = 0.0;
        }

        // Pre-filter `ch_slice` to only contain channels in range for this
        // device's frame size. This hoists the bounds check OUT of the
        // per-frame loop so `frame.get_unchecked` is sound in the hot path
        // (DOLL-126). Out-of-range channels (e.g. a config that requested
        // ch5 on a 2-channel device) are skipped for the batch — same
        // graceful-skip behavior the prior `frame.get()` Option-match
        // produced.
        //
        // DOLL-375: the active set is a pure function of the (immutable)
        // channel list and `frame_size`, so cache it and rebuild only when
        // `frame_size` changes — instead of zero-initialising a 255-byte array
        // and re-scanning every channel on every `write_samples` call.
        if self.active_cache_frame_size != frame_size {
            let mut count = 0_usize;
            for (idx, &channel) in ch_slice.iter().enumerate() {
                if (channel as usize) < frame_size {
                    self.active_indices[count] = idx as u8;
                    count += 1;
                }
            }
            self.active_count = count;
            self.active_cache_frame_size = frame_size;
        }
        let active_idx_slice = &self.active_indices[..self.active_count];

        if self.monitor_only || (self.gate_enabled && self.gate_state == GateState::Idle) {
            // Monitor mode or gate idle: only track peaks, no disk writes
            for frame in frame_data.chunks_exact(frame_size) {
                for &active_idx in active_idx_slice {
                    let idx = active_idx as usize;
                    let channel = ch_slice[idx] as usize;
                    // SAFETY: pre-filter above guaranteed `channel < frame_size`,
                    // and `chunks_exact` yields frames of exactly `frame_size`
                    // samples — so `channel` is in bounds.
                    let s = unsafe { *frame.get_unchecked(channel) };
                    if s.is_finite() {
                        self.peak_scratch[idx] = self.peak_scratch[idx].max(s.abs());
                    }
                }
            }
        } else {
            match self.output_mode {
                OutputMode::Split => {
                    for frame in frame_data.chunks_exact(frame_size) {
                        for &active_idx in active_idx_slice {
                            let idx = active_idx as usize;
                            let channel = ch_slice[idx] as usize;
                            // SAFETY: see comment above.
                            let s = unsafe { *frame.get_unchecked(channel) };
                            if s.is_finite() {
                                self.peak_scratch[idx] = self.peak_scratch[idx].max(s.abs());
                            }
                            if let Some(w) = &mut self.multichannel_writers[idx]
                                && w.write_sample((s.clamp(-1.0, 1.0) * scale).round() as i32)
                                    .is_err()
                            {
                                self.write_errors.fetch_add(1, Ordering::Relaxed);
                            }
                        }
                    }
                }
                OutputMode::Single => {
                    if let Some(w) = &mut self.writer {
                        for frame in frame_data.chunks_exact(frame_size) {
                            for &active_idx in active_idx_slice {
                                let idx = active_idx as usize;
                                let channel = ch_slice[idx] as usize;
                                // SAFETY: see comment above.
                                let s = unsafe { *frame.get_unchecked(channel) };
                                if s.is_finite() {
                                    self.peak_scratch[idx] = self.peak_scratch[idx].max(s.abs());
                                }
                                if w.write_sample((s.clamp(-1.0, 1.0) * scale).round() as i32)
                                    .is_err()
                                {
                                    self.write_errors.fetch_add(1, Ordering::Relaxed);
                                }
                            }
                        }
                    }
                }
            }
        }

        // Publish peaks to shared atomics (only active channels, not full array).
        // peak_levels.len() == ch_count (both derived from the same channel list at construction),
        // so zip is always exhaustive over the active channels with no bounds check per iteration.
        for (peak_slot, &peak) in self
            .peak_levels
            .iter()
            .zip(self.peak_scratch[..ch_count].iter())
        {
            peak_slot.value.store(peak.to_bits(), Ordering::Relaxed);
        }

        // Silence gate transitions
        if self.gate_enabled && !self.monitor_only {
            let max_peak = self.peak_scratch[..ch_count]
                .iter()
                .copied()
                .fold(0.0_f32, f32::max);
            let has_signal = max_peak > self.silence_threshold;

            match self.gate_state {
                GateState::Idle => {
                    if has_signal {
                        // Flag for the main loop to open writers before the next read.
                        // Keeps write_samples() free of file I/O.
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
                            // Flag for the main loop to finalize writers.
                            // Keeps write_samples() free of file I/O.
                            self.gate_pending_close = true;
                        }
                    }
                }
            }
        }
    }

    /// Process a pending gate open: create WAV files and transition to Recording.
    /// Called from the main loop (or tests) after `write_samples` sets `gate_pending_open`.
    pub fn process_gate_open(&mut self) {
        if !self.gate_pending_open {
            return;
        }
        self.gate_pending_open = false;
        info!("Silence gate: signal detected, opening writers");
        if let Err(e) = self.open_writers_for_gate() {
            error!("Silence gate: failed to open writers: {}", e);
        } else {
            self.gate_state = GateState::Recording;
            // Status flag only; no synchronizes-with relationship.
            self.gate_idle.store(false, Ordering::Relaxed);
        }
    }

    /// Process a pending gate close: finalize WAV files and transition to Idle.
    /// Called from the main loop (or tests) after `write_samples` sets `gate_pending_close`.
    pub fn process_gate_close(&mut self) {
        if !self.gate_pending_close {
            return;
        }
        self.gate_pending_close = false;
        info!(
            "Silence gate: timeout reached ({} frames), finalizing files",
            self.gate_silence_frames
        );
        if let Err(e) = self.finalize_all() {
            error!("Silence gate: finalize error: {}", e);
        }
        self.gate_state = GateState::Idle;
        // Status flag only; no synchronizes-with relationship.
        self.gate_idle.store(true, Ordering::Relaxed);
        self.gate_silence_frames = 0;
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

    /// Rotate files: finalize current writers, rename, check silence, create new writers.
    pub fn rotate_files(&mut self) {
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

        // Take all pending (tmp → final) pairs from the previous period
        let old_pending: Vec<(String, String)> = std::mem::take(&mut self.pending_files);

        // Finalize the main WAV file if it exists
        if let Some(writer) = self.writer.take()
            && let Err(e) = writer.finalize()
        {
            error!("Error finalizing WAV file during rotation: {}", e);
        }

        // Finalize any multichannel writers
        for writer_opt in &mut self.multichannel_writers {
            if let Some(writer) = writer_opt.take()
                && let Err(e) = writer.finalize()
            {
                error!("Error finalizing channel WAV file during rotation: {}", e);
            }
        }

        // Rename tmp files to final paths
        let mut final_files = Vec::new();
        for (tmp_path, final_path) in &old_pending {
            if Path::new(tmp_path).exists() {
                if let Err(e) = fs::rename(tmp_path, final_path) {
                    error!("Error renaming {} to {}: {}", tmp_path, final_path, e);
                } else {
                    info!("Finalized recording to {}", final_path);
                    final_files.push(final_path.clone());
                }
            }
        }

        // Hand the recently-rotated files to the dedicated silence-check
        // worker. The writer thread immediately resumes draining the ring
        // buffer during rotation; silence detection happens off-thread.
        if !final_files.is_empty()
            && let Some(worker) = self.silence_worker.as_ref()
        {
            worker.submit(final_files);
        }

        // Create new files for the next recording period
        let ch_count = self.channel_count as usize;
        let date_str = (self.timestamp_fn)();
        match self.output_mode {
            OutputMode::Split => {
                for (idx, &channel) in self.channels[..ch_count].iter().enumerate() {
                    let final_path = disambiguate_path(&format!(
                        "{}/{}-ch{}.wav",
                        self.output_dir, date_str, channel
                    ));
                    let tmp = tmp_wav_path(&final_path);
                    let spec = crate::raw_wav_writer::WavSpec {
                        channels: 1,
                        sample_rate: self.sample_rate,
                        bits_per_sample: self.bits_per_sample,
                    };
                    match create_wav_writer(&tmp, spec) {
                        Ok(w) => {
                            self.multichannel_writers[idx] = Some(w);
                            self.pending_files.push((tmp, final_path.clone()));
                            info!("Created channel WAV file: {}", final_path);
                        }
                        Err(e) => {
                            error!("Failed to create channel WAV file: {}", e);
                        }
                    }
                }
            }
            OutputMode::Single if ch_count > 2 => {
                let final_path = disambiguate_path(&format!(
                    "{}/{}-multichannel.wav",
                    self.output_dir, date_str
                ));
                let tmp = tmp_wav_path(&final_path);
                let spec = crate::raw_wav_writer::WavSpec {
                    channels: self.channel_count as u16,
                    sample_rate: self.sample_rate,
                    bits_per_sample: self.bits_per_sample,
                };
                match create_wav_writer(&tmp, spec) {
                    Ok(w) => {
                        self.writer = Some(w);
                        self.pending_files.push((tmp, final_path.clone()));
                        info!("Created multichannel WAV file: {}", final_path);
                    }
                    Err(e) => {
                        error!("Failed to create multichannel WAV file: {}", e);
                    }
                }
            }
            OutputMode::Single => {
                let final_path =
                    disambiguate_path(&format!("{}/{}.wav", self.output_dir, date_str));
                let tmp = tmp_wav_path(&final_path);
                match create_wav_writer(&tmp, self.current_spec) {
                    Ok(w) => {
                        self.writer = Some(w);
                        self.pending_files.push((tmp, final_path.clone()));
                        info!("Created new recording file: {}", final_path);
                    }
                    Err(e) => {
                        error!("Failed to create new WAV file: {}", e);
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
    pub fn finalize_all(&mut self) -> Result<(), BlackboxError> {
        let mut first_err: Option<BlackboxError> = None;

        // Finalize the main WAV file
        if let Some(writer) = self.writer.take()
            && let Err(e) = writer.finalize()
        {
            let err = BlackboxError::Wav(format!("Error finalizing WAV file: {}", e));
            error!("{}", err);
            first_err.get_or_insert(err);
        }

        // Finalize any multichannel writers — attempt all even if one fails
        for writer_opt in &mut self.multichannel_writers {
            if let Some(writer) = writer_opt.take()
                && let Err(e) = writer.finalize()
            {
                let err = BlackboxError::Wav(format!("Error finalizing channel WAV file: {}", e));
                error!("{}", err);
                first_err.get_or_insert(err);
            }
        }

        // Rename all pending .recording.wav files to their final .wav paths —
        // attempt every one; a single rename failure must not strand the rest.
        let pending = std::mem::take(&mut self.pending_files);
        let mut final_files = Vec::new();
        for (tmp_path, final_path) in &pending {
            if Path::new(tmp_path).exists() {
                match fs::rename(tmp_path, final_path) {
                    Ok(()) => {
                        info!("Finalized recording to {}", final_path);
                        final_files.push(final_path.clone());
                    }
                    Err(e) => {
                        error!("Error renaming {} to {}: {}", tmp_path, final_path, e);
                        first_err.get_or_insert_with(|| BlackboxError::from(e));
                    }
                }
            }
        }

        // Hand finalized files to the silence-check worker. Drop of the
        // worker (when WriterThreadState is dropped) joins the worker
        // thread, so any in-flight check completes before the process
        // tears down — eliminating the race the prior detached spawn had
        // with file-system teardown on shutdown.
        if !final_files.is_empty()
            && let Some(worker) = self.silence_worker.as_ref()
        {
            worker.submit(final_files);
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
pub fn read_available(consumer: &mut rtrb::Consumer<f32>, state: &mut WriterThreadState) -> usize {
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
pub fn check_and_delete_silent_files(files: &[String], threshold: f32) {
    for file_path in files {
        match is_silent(file_path, threshold) {
            Ok(true) => {
                info!(
                    "Recording is silent (below threshold {}), deleting file: {}",
                    threshold, file_path
                );
                if let Err(e) = fs::remove_file(file_path) {
                    error!("Error deleting silent file: {}", e);
                }
            }
            Ok(false) => {
                info!(
                    "Recording is not silent (above threshold {}), keeping file: {}",
                    threshold, file_path
                );
            }
            Err(e) => {
                error!("Error checking for silence: {}", e);
            }
        }
    }
}

fn drain_remaining(consumer: &mut rtrb::Consumer<f32>, state: &mut WriterThreadState) {
    loop {
        if read_available(consumer, state) == 0 {
            break;
        }
    }
}

pub fn writer_thread_main(
    mut consumer: rtrb::Consumer<f32>,
    rotation_needed: Arc<std::sync::atomic::AtomicBool>,
    command_rx: std::sync::mpsc::Receiver<WriterCommand>,
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
        // 1. Check for shutdown command (non-blocking)
        if let Ok(WriterCommand::Shutdown(reply_tx)) = command_rx.try_recv() {
            // Drain remaining samples from ring buffer
            drain_remaining(&mut consumer, &mut state);
            let result = state.finalize_all();
            let _ = reply_tx.send(result);
            return;
        }

        // 2. Check disk space periodically
        state.check_disk_space();

        // 3. Check rotation flag (set by RT callback via AtomicBool).
        // Relaxed: status flag only, no companion data to acquire (DOLL-391).
        if rotation_needed.swap(false, Ordering::Relaxed) {
            state.rotate_files();
        }

        // 3b. Process deferred gate transitions (keeps write_samples free of file I/O)
        state.process_gate_open();
        state.process_gate_close();

        // 4. Read available samples from ring buffer
        let read = read_available(&mut consumer, &mut state);

        // 5. Periodic flush — writes valid WAV headers for crash recovery
        state.flush_writers(read);

        if read == 0 {
            // Ring buffer empty — back off gradually to reduce idle wakeups.
            // 1ms → 2ms → 3ms → 4ms → 5ms (cap). Resets on data arrival.
            consecutive_empty = consecutive_empty.saturating_add(1).min(5);
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
/// path — an `rtrb` ring buffer feeding a spawned [`writer_thread_main`],
/// which uses [`WriterThreadState`] + [`RawWavWriter`], the adaptive-sleep
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
#[cfg(feature = "benchmarking")]
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
    let peak_levels: Arc<Vec<CacheAlignedPeak>> =
        Arc::new((0..channels).map(|_| CacheAlignedPeak::new(0)).collect());

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
    state.total_device_channels = channels as u16;

    // Ring buffer sized exactly as production (see process_audio_impl).
    let ring_size = sample_rate as usize * channels * crate::RING_BUFFER_SECONDS;
    let (mut producer, consumer) = rtrb::RingBuffer::new(ring_size);
    let rotation_needed = Arc::new(AtomicBool::new(false));
    let (command_tx, command_rx) = std::sync::mpsc::sync_channel::<WriterCommand>(1);

    let writer_handle = std::thread::Builder::new()
        .name("bench-real-writer".to_string())
        .spawn(move || writer_thread_main(consumer, rotation_needed, command_rx, state))
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
    let _ = command_tx.send(WriterCommand::Shutdown(reply_tx));
    let _ = reply_rx.recv();
    let _ = writer_handle.join();
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
        let p = dir.path().join("rec.wav").to_str().unwrap().to_string();
        assert_eq!(disambiguate_path(&p), p);
    }

    #[test]
    fn appends_suffix_when_path_exists() {
        let dir = tempdir().unwrap();
        let p = dir.path().join("rec.wav").to_str().unwrap().to_string();
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
        let base = dir.path().join("rec.wav").to_str().unwrap().to_string();
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
        let peak_levels: Arc<Vec<CacheAlignedPeak>> = Arc::new(vec![CacheAlignedPeak::new(0)]);

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
        let peak_levels: Arc<Vec<CacheAlignedPeak>> = Arc::new(vec![CacheAlignedPeak::new(0)]);

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
