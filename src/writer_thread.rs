use std::fs;
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::Duration;

use log::{error, info, warn};

use crate::constants::{
    CacheAlignedPeak, MAX_CHANNELS, OutputMode, WRITER_THREAD_READ_CHUNK, WavWriterType,
};
use crate::error::BlackboxError;
use crate::utils::{available_disk_space_mb, is_silent};

use chrono::prelude::*;

// ---------------------------------------------------------------------------
// Helper functions (moved from cpal_processor.rs)
// ---------------------------------------------------------------------------

/// Returns a timestamp string like "2024-01-15-14-30-05" from the current local time.
/// Includes seconds so that file rotations within the same minute produce distinct names.
pub fn timestamp_now() -> String {
    Local::now().format("%Y-%m-%d-%H-%M-%S").to_string()
}

/// Returns a `.recording.wav` temporary path for the given final `.wav` path.
fn tmp_wav_path(final_path: &str) -> String {
    final_path.strip_suffix(".wav").map_or_else(
        || format!("{final_path}.recording"),
        |stem| format!("{stem}.recording.wav"),
    )
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

pub struct WriterThreadState {
    // --- Hot fields: accessed every write_samples() call, grouped for cache locality ---
    /// Cached scale factor for f32-to-WAV conversion (avoids match per sample).
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
    /// Iteration counter for periodic WAV flush (crash-safe headers every ~10 seconds).
    flush_counter: u16,
    /// Channel indices as a fixed inline array — no heap indirection, always in cache.
    /// Only the first `channel_count` entries are valid.
    pub channels: [u8; MAX_CHANNELS],
    /// Per-frame peak accumulator — fixed inline array, no heap pointer chase.
    /// Only the first `channel_count` entries are used.
    peak_scratch: [f32; MAX_CHANNELS],

    // --- Warm fields: accessed frequently but not per-sample ---
    pub writer: Option<WavWriterType>,
    pub multichannel_writers: Vec<Option<WavWriterType>>,
    pub write_errors: Arc<AtomicU64>,
    /// Per-channel peak levels (f32 stored as u32 bits via `to_bits()`). Shared with FFI.
    /// Each element is cache-line-aligned to prevent false sharing with the UI reader thread.
    pub peak_levels: Arc<Vec<CacheAlignedPeak>>,
    /// Partial frames carried over between ring buffer reads.
    frame_remainder: Vec<f32>,
    /// Pre-allocated buffer for combining frame_remainder + new data (avoids heap alloc).
    combined_buf: Vec<f32>,
    /// Consecutive silent frames counted while gate is Recording.
    gate_silence_frames: u64,
    /// Frame count threshold for gate timeout (`timeout_secs * sample_rate`).
    gate_timeout_frames: u64,
    /// Shared flag: true when gate is idle (no files open). Read by FFI for status.
    pub gate_idle: Arc<AtomicBool>,

    // --- Cold fields: only accessed during setup, rotation, or shutdown ---
    pub output_dir: String,
    pub sample_rate: u32,
    pub bits_per_sample: u16,
    pub current_spec: hound::WavSpec,
    pub pending_files: Vec<(String, String)>,
    pub silence_threshold: f32,
    /// Minimum free disk space in MB before stopping writes (0 = disabled).
    pub min_disk_space_mb: u64,
    /// Shared flag: set when disk space drops below threshold.
    pub disk_space_low: Arc<AtomicBool>,
}

/// Convert an f32 sample (range -1.0..1.0) to an i32 scaled for the given bit depth.
/// Used by tests; the hot path uses the pre-cached `sample_scale` field instead.
#[cfg(test)]
pub fn f32_to_wav_sample(sample: f32, bits_per_sample: u16) -> i32 {
    let scale = match bits_per_sample {
        16 => f32::from(i16::MAX), // 32767.0
        24 => 8_388_607.0_f32,     // 2^23 - 1
        _ => i32::MAX as f32,      // 2^31 - 1
    };
    (sample * scale) as i32
}

impl WriterThreadState {
    /// Create a new `WriterThreadState` with initial WAV writers set up.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        output_dir: &str,
        sample_rate: u32,
        channels: &[usize],
        output_mode: &str,
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
            disk_space_low.store(true, Ordering::Release);
            return Err(BlackboxError::Io(std::io::Error::other(format!(
                "Insufficient disk space: {}MB available, {}MB required",
                available_mb, min_disk_space_mb
            ))));
        }

        let mode = OutputMode::parse(output_mode).ok_or_else(|| {
            BlackboxError::Config(format!("Invalid output mode: '{}'", output_mode))
        })?;

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
            output_mode: mode,
            channel_count: channel_count as u8,
            total_device_channels: 0, // set by caller or process_audio
            disk_check_counter: 0,
            flush_counter: 0,
            channels: ch_arr,
            peak_scratch: [0.0_f32; MAX_CHANNELS],
            writer: None,
            multichannel_writers: Vec::new(),
            write_errors,
            peak_levels,
            frame_remainder: Vec::new(),
            combined_buf: Vec::new(),
            gate_silence_frames: 0,
            gate_timeout_frames: u64::from(sample_rate) * gate_timeout_secs,
            gate_idle,
            output_dir: output_dir.to_string(),
            sample_rate,
            bits_per_sample,
            current_spec: hound::WavSpec {
                channels: 1,
                sample_rate,
                bits_per_sample,
                sample_format: hound::SampleFormat::Int,
            },
            pending_files: Vec::new(),
            silence_threshold,
            min_disk_space_mb,
            disk_space_low,
        };

        // When gate is enabled, start idle (no files). Writers are created on first signal.
        if !gate_enabled {
            match mode {
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
        let sample_scale = match 24_u16 {
            16 => f32::from(i16::MAX),
            24 => 8_388_607.0_f32,
            _ => i32::MAX as f32,
        };

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
            flush_counter: 0,
            channels: ch_arr,
            peak_scratch: [0.0_f32; MAX_CHANNELS],
            writer: None,
            multichannel_writers: Vec::new(),
            write_errors: Arc::new(AtomicU64::new(0)),
            peak_levels,
            frame_remainder: Vec::new(),
            combined_buf: Vec::new(),
            gate_silence_frames: 0,
            gate_timeout_frames: 0,
            gate_idle: Arc::new(AtomicBool::new(false)),
            output_dir: String::new(),
            sample_rate,
            bits_per_sample: 24,
            current_spec: hound::WavSpec {
                channels: 1,
                sample_rate,
                bits_per_sample: 24,
                sample_format: hound::SampleFormat::Int,
            },
            pending_files: Vec::new(),
            silence_threshold: 0.0,
            min_disk_space_mb: 0,
            disk_space_low: Arc::new(AtomicBool::new(false)),
        }
    }

    fn setup_split_mode(&mut self) -> Result<(), BlackboxError> {
        let date_str = timestamp_now();
        let ch_count = self.channel_count as usize;

        info!("Setting up split mode with {} channels", ch_count);

        self.multichannel_writers.clear();
        for _ in 0..MAX_CHANNELS {
            self.multichannel_writers.push(None);
        }

        for (idx, &channel) in self.channels[..ch_count].iter().enumerate() {
            let final_path = format!("{}/{}-ch{}.wav", self.output_dir, date_str, channel);
            let tmp_path = tmp_wav_path(&final_path);

            let spec = hound::WavSpec {
                channels: 1,
                sample_rate: self.sample_rate,
                bits_per_sample: self.bits_per_sample,
                sample_format: hound::SampleFormat::Int,
            };

            let writer = hound::WavWriter::create(&tmp_path, spec).map_err(|e| {
                BlackboxError::Wav(format!("Failed to create channel WAV file: {}", e))
            })?;

            self.multichannel_writers[idx] = Some(writer);
            self.pending_files.push((tmp_path, final_path.clone()));
            info!("Created channel WAV file: {}", final_path);
        }

        self.current_spec = hound::WavSpec {
            channels: 1,
            sample_rate: self.sample_rate,
            bits_per_sample: self.bits_per_sample,
            sample_format: hound::SampleFormat::Int,
        };

        Ok(())
    }

    fn setup_multichannel_mode(&mut self) -> Result<(), BlackboxError> {
        let date_str = timestamp_now();
        let ch_count = self.channel_count as usize;

        let final_path = format!("{}/{}-multichannel.wav", self.output_dir, date_str);
        let tmp_path = tmp_wav_path(&final_path);

        info!("Setting up multichannel mode with {} channels", ch_count);

        let spec = hound::WavSpec {
            channels: self.channel_count as u16,
            sample_rate: self.sample_rate,
            bits_per_sample: self.bits_per_sample,
            sample_format: hound::SampleFormat::Int,
        };

        let writer = hound::WavWriter::create(&tmp_path, spec).map_err(|e| {
            BlackboxError::Wav(format!("Failed to create multichannel WAV file: {}", e))
        })?;

        self.writer = Some(writer);
        self.current_spec = spec;
        self.pending_files.push((tmp_path, final_path.clone()));

        info!("Created multichannel WAV file: {}", final_path);
        Ok(())
    }

    fn setup_standard_mode(&mut self) -> Result<(), BlackboxError> {
        let date_str = timestamp_now();
        let ch_count = self.channel_count as usize;

        info!("Setting up standard mode with {} channels", ch_count);

        let num_channels = if ch_count == 1 { 1 } else { 2 };

        let final_path = format!("{}/{}.wav", self.output_dir, date_str);
        let tmp_path = tmp_wav_path(&final_path);

        let spec = hound::WavSpec {
            channels: num_channels as u16,
            sample_rate: self.sample_rate,
            bits_per_sample: self.bits_per_sample,
            sample_format: hound::SampleFormat::Int,
        };

        let writer = hound::WavWriter::create(&tmp_path, spec)
            .map_err(|e| BlackboxError::Wav(format!("Failed to create WAV file: {}", e)))?;

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

        if let Some(available_mb) = available_disk_space_mb(&self.output_dir)
            && available_mb < self.min_disk_space_mb
        {
            warn!(
                "Disk space low: {}MB available, threshold is {}MB — stopping recording",
                available_mb, self.min_disk_space_mb
            );
            self.disk_space_low.store(true, Ordering::Release);
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
    /// `hound::WavWriter::flush()` rewrites the WAV header with the correct data
    /// size and flushes the underlying `BufWriter` to the OS. After a flush, the
    /// file is a valid WAV playable up to that point — even after a force-quit or
    /// SIGKILL. Uses a counter (~25,000 iterations ≈ 10 seconds) to amortize cost.
    pub fn flush_writers(&mut self) {
        if self.monitor_only || self.disk_stopped {
            return;
        }

        self.flush_counter += 1;
        if self.flush_counter < 25_000 {
            return;
        }
        self.flush_counter = 0;

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

        if self.monitor_only || (self.gate_enabled && self.gate_state == GateState::Idle) {
            // Monitor mode or gate idle: only track peaks, no disk writes
            for frame in frame_data.chunks(frame_size) {
                for (idx, &channel) in ch_slice.iter().enumerate() {
                    let ch = channel as usize;
                    if ch < frame.len() {
                        let abs = frame[ch].abs();
                        self.peak_scratch[idx] = self.peak_scratch[idx].max(abs);
                    }
                }
            }
        } else {
            match self.output_mode {
                OutputMode::Split => {
                    for frame in frame_data.chunks(frame_size) {
                        for (idx, &channel) in ch_slice.iter().enumerate() {
                            let ch = channel as usize;
                            if ch < frame.len() {
                                let s = frame[ch];
                                self.peak_scratch[idx] = self.peak_scratch[idx].max(s.abs());
                                if let Some(w) = &mut self.multichannel_writers[idx]
                                    && w.write_sample((s * scale) as i32).is_err()
                                {
                                    self.write_errors.fetch_add(1, Ordering::Relaxed);
                                }
                            }
                        }
                    }
                }
                OutputMode::Single if ch_count > 2 => {
                    if let Some(w) = &mut self.writer {
                        for frame in frame_data.chunks(frame_size) {
                            for (idx, &channel) in ch_slice.iter().enumerate() {
                                let ch = channel as usize;
                                if ch < frame.len() {
                                    let s = frame[ch];
                                    self.peak_scratch[idx] = self.peak_scratch[idx].max(s.abs());
                                    if w.write_sample((s * scale) as i32).is_err() {
                                        self.write_errors.fetch_add(1, Ordering::Relaxed);
                                    }
                                }
                            }
                        }
                    }
                }
                OutputMode::Single => {
                    if let Some(w) = &mut self.writer {
                        for frame in frame_data.chunks(frame_size) {
                            for (idx, &channel) in ch_slice.iter().enumerate() {
                                let ch = channel as usize;
                                if ch < frame.len() {
                                    let s = frame[ch];
                                    self.peak_scratch[idx] = self.peak_scratch[idx].max(s.abs());
                                    if w.write_sample((s * scale) as i32).is_err() {
                                        self.write_errors.fetch_add(1, Ordering::Relaxed);
                                    }
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
        for (peak_slot, &peak) in self.peak_levels.iter().zip(self.peak_scratch[..ch_count].iter())
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
                        info!("Silence gate: signal detected, opening writers");
                        self.gate_silence_frames = 0;
                        if let Err(e) = self.open_writers_for_gate() {
                            error!("Silence gate: failed to open writers: {}", e);
                        } else {
                            self.gate_state = GateState::Recording;
                            self.gate_idle.store(false, Ordering::Release);
                        }
                    }
                }
                GateState::Recording => {
                    if has_signal {
                        self.gate_silence_frames = 0;
                    } else {
                        self.gate_silence_frames += full_frames as u64;
                        if self.gate_silence_frames >= self.gate_timeout_frames {
                            info!(
                                "Silence gate: timeout reached ({} frames), finalizing files",
                                self.gate_silence_frames
                            );
                            if let Err(e) = self.finalize_all() {
                                error!("Silence gate: finalize error: {}", e);
                            }
                            self.gate_state = GateState::Idle;
                            self.gate_idle.store(true, Ordering::Release);
                            self.gate_silence_frames = 0;
                        }
                    }
                }
            }
        }
    }

    /// Create WAV writers mid-session when the silence gate transitions from Idle to Recording.
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

        // Check for silence on a background thread so the writer thread can
        // immediately resume draining the ring buffer during rotation.
        if self.silence_threshold > 0.0 && !final_files.is_empty() {
            let threshold = self.silence_threshold;
            std::thread::Builder::new()
                .name("blackbox-silence".to_string())
                .spawn(move || {
                    check_and_delete_silent_files(&final_files, threshold);
                })
                .ok(); // If thread spawn fails, skip silence check rather than block
        }

        // Create new files for the next recording period
        let ch_count = self.channel_count as usize;
        match self.output_mode {
            OutputMode::Split => {
                for (idx, &channel) in self.channels[..ch_count].iter().enumerate() {
                    let final_path =
                        format!("{}/{}-ch{}.wav", self.output_dir, timestamp_now(), channel);
                    let tmp = tmp_wav_path(&final_path);
                    let spec = hound::WavSpec {
                        channels: 1,
                        sample_rate: self.sample_rate,
                        bits_per_sample: self.bits_per_sample,
                        sample_format: hound::SampleFormat::Int,
                    };
                    match hound::WavWriter::create(&tmp, spec) {
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
                let final_path =
                    format!("{}/{}-multichannel.wav", self.output_dir, timestamp_now());
                let tmp = tmp_wav_path(&final_path);
                let spec = hound::WavSpec {
                    channels: self.channel_count as u16,
                    sample_rate: self.sample_rate,
                    bits_per_sample: self.bits_per_sample,
                    sample_format: hound::SampleFormat::Int,
                };
                match hound::WavWriter::create(&tmp, spec) {
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
                let final_path = format!("{}/{}.wav", self.output_dir, timestamp_now());
                let tmp = tmp_wav_path(&final_path);
                match hound::WavWriter::create(&tmp, self.current_spec) {
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
    pub fn finalize_all(&mut self) -> Result<(), BlackboxError> {
        // Finalize the main WAV file
        if let Some(writer) = self.writer.take() {
            writer
                .finalize()
                .map_err(|e| BlackboxError::Wav(format!("Error finalizing WAV file: {}", e)))?;
        }

        // Finalize any multichannel writers
        for writer_opt in &mut self.multichannel_writers {
            if let Some(writer) = writer_opt.take() {
                writer.finalize().map_err(|e| {
                    BlackboxError::Wav(format!("Error finalizing channel WAV file: {}", e))
                })?;
            }
        }

        // Rename all pending .recording.wav files to their final .wav paths
        let pending = std::mem::take(&mut self.pending_files);
        let mut final_files = Vec::new();
        for (tmp_path, final_path) in &pending {
            if Path::new(tmp_path).exists() {
                fs::rename(tmp_path, final_path)?;
                info!("Finalized recording to {}", final_path);
                final_files.push(final_path.clone());
            }
        }

        // Check silence synchronously during finalize — recording has stopped,
        // no ring buffer pressure.
        if self.silence_threshold > 0.0 {
            check_and_delete_silent_files(&final_files, self.silence_threshold);
        }

        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Writer thread main loop
// ---------------------------------------------------------------------------

/// Read and process a chunk from the consumer. Returns the number of samples read.
fn read_available(consumer: &mut rtrb::Consumer<f32>, state: &mut WriterThreadState) -> usize {
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

        // 3. Check rotation flag (set by RT callback via AtomicBool)
        if rotation_needed.swap(false, Ordering::Acquire) {
            state.rotate_files();
        }

        // 4. Read available samples from ring buffer
        let read = read_available(&mut consumer, &mut state);

        // 5. Periodic flush — writes valid WAV headers for crash recovery
        state.flush_writers();

        if read == 0 {
            // Ring buffer empty — sleep briefly to avoid busy-wait
            std::thread::sleep(Duration::from_millis(1));
        }
    }
}
