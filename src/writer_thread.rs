use std::fs;
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::{Duration, Instant};

use log::{error, info, warn};

use crate::constants::{
    DISK_CHECK_INTERVAL_SECS, MAX_CHANNELS, WRITER_THREAD_READ_CHUNK, WavWriterType,
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
    final_path.replace(".wav", ".recording.wav")
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
    pub output_mode: String,
    pub channels: Vec<usize>,
    pub total_device_channels: usize,
    pub output_dir: String,
    pub sample_rate: u32,
    pub current_spec: hound::WavSpec,
    pub writer: Option<WavWriterType>,
    pub multichannel_writers: Vec<Option<WavWriterType>>,
    pub pending_files: Vec<(String, String)>,
    pub silence_threshold: f32,
    pub write_errors: Arc<AtomicU64>,
    /// Partial frames carried over between ring buffer reads.
    frame_remainder: Vec<f32>,
    /// Minimum free disk space in MB before stopping writes (0 = disabled).
    pub min_disk_space_mb: u64,
    /// Shared flag: set when disk space drops below threshold.
    pub disk_space_low: Arc<AtomicBool>,
    /// When true, writing is paused because disk space is low.
    disk_stopped: bool,
    /// Last time we checked disk space.
    last_disk_check: Instant,
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

        let mut state = WriterThreadState {
            output_mode: output_mode.to_string(),
            channels: channels.to_vec(),
            total_device_channels: 0, // set by caller or process_audio
            output_dir: output_dir.to_string(),
            sample_rate,
            current_spec: hound::WavSpec {
                channels: 1,
                sample_rate,
                bits_per_sample: 16,
                sample_format: hound::SampleFormat::Int,
            },
            writer: None,
            multichannel_writers: Vec::new(),
            pending_files: Vec::new(),
            silence_threshold,
            write_errors,
            frame_remainder: Vec::new(),
            min_disk_space_mb,
            disk_space_low,
            disk_stopped: false,
            last_disk_check: Instant::now(),
        };

        // Set up writers based on output mode
        match output_mode {
            "split" => state.setup_split_mode()?,
            "single" if channels.len() <= 2 => state.setup_standard_mode()?,
            "single" => state.setup_multichannel_mode()?,
            other => {
                return Err(BlackboxError::Config(format!(
                    "Invalid output mode: '{}'",
                    other
                )));
            }
        }

        Ok(state)
    }

    fn setup_split_mode(&mut self) -> Result<(), BlackboxError> {
        let date_str = timestamp_now();

        info!(
            "Setting up split mode with {} channels",
            self.channels.len()
        );

        self.multichannel_writers.clear();
        for _ in 0..MAX_CHANNELS {
            self.multichannel_writers.push(None);
        }

        for (idx, &channel) in self.channels.iter().enumerate() {
            let final_path = format!("{}/{}-ch{}.wav", self.output_dir, date_str, channel);
            let tmp_path = tmp_wav_path(&final_path);

            let spec = hound::WavSpec {
                channels: 1,
                sample_rate: self.sample_rate,
                bits_per_sample: 16,
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
            bits_per_sample: 16,
            sample_format: hound::SampleFormat::Int,
        };

        Ok(())
    }

    fn setup_multichannel_mode(&mut self) -> Result<(), BlackboxError> {
        let date_str = timestamp_now();

        let final_path = format!("{}/{}-multichannel.wav", self.output_dir, date_str);
        let tmp_path = tmp_wav_path(&final_path);

        info!(
            "Setting up multichannel mode with {} channels",
            self.channels.len()
        );

        let spec = hound::WavSpec {
            channels: self.channels.len() as u16,
            sample_rate: self.sample_rate,
            bits_per_sample: 16,
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

        info!(
            "Setting up standard mode with {} channels",
            self.channels.len()
        );

        let num_channels = if self.channels.len() == 1 { 1 } else { 2 };

        let final_path = format!("{}/{}.wav", self.output_dir, date_str);
        let tmp_path = tmp_wav_path(&final_path);

        let spec = hound::WavSpec {
            channels: num_channels as u16,
            sample_rate: self.sample_rate,
            bits_per_sample: 16,
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
    pub fn check_disk_space(&mut self) -> bool {
        if self.min_disk_space_mb == 0 || self.disk_stopped {
            return !self.disk_stopped;
        }

        let now = Instant::now();
        if now.duration_since(self.last_disk_check) < Duration::from_secs(DISK_CHECK_INTERVAL_SECS)
        {
            return true;
        }
        self.last_disk_check = now;

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

    /// Write interleaved f32 samples to WAV writers.
    ///
    /// Handles partial frames: if `data` doesn't divide evenly by `total_device_channels`,
    /// leftover samples are stored in `frame_remainder` and prepended to the next call.
    pub fn write_samples(&mut self, data: &[f32]) {
        // If total_device_channels is 0 or disk stopped, skip writing
        if self.total_device_channels == 0 || self.disk_stopped {
            return;
        }

        // Prepend any leftover samples from the previous call
        let combined: Vec<f32>;
        let work_data: &[f32] = if self.frame_remainder.is_empty() {
            data
        } else {
            combined = [self.frame_remainder.as_slice(), data].concat();
            self.frame_remainder.clear();
            &combined
        };

        let frame_size = self.total_device_channels;
        let full_frames = work_data.len() / frame_size;
        let used = full_frames * frame_size;

        // Save any partial frame for next time
        if used < work_data.len() {
            self.frame_remainder.extend_from_slice(&work_data[used..]);
        }

        let frame_data = &work_data[..used];

        match self.output_mode.as_str() {
            "split" => {
                let frames = frame_data.chunks(frame_size);
                for frame in frames {
                    for (idx, &channel) in self.channels.iter().enumerate() {
                        if channel < frame.len()
                            && let Some(w) = &mut self.multichannel_writers[idx]
                        {
                            let sample = (frame[channel] * 32767.0) as i32;
                            if w.write_sample(sample).is_err() {
                                self.write_errors.fetch_add(1, Ordering::Relaxed);
                            }
                        }
                    }
                }
            }
            "single" if self.channels.len() > 2 => {
                if let Some(w) = &mut self.writer {
                    let frames = frame_data.chunks(frame_size);
                    for frame in frames {
                        for &channel in &self.channels {
                            if channel < frame.len() {
                                let sample = (frame[channel] * 32767.0) as i32;
                                if w.write_sample(sample).is_err() {
                                    self.write_errors.fetch_add(1, Ordering::Relaxed);
                                }
                            }
                        }
                    }
                }
            }
            _ => {
                if let Some(w) = &mut self.writer {
                    let frames = frame_data.chunks(frame_size);
                    for frame in frames {
                        for &channel in &self.channels {
                            if channel < frame.len() {
                                let sample = (frame[channel] * 32767.0) as i32;
                                if w.write_sample(sample).is_err() {
                                    self.write_errors.fetch_add(1, Ordering::Relaxed);
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    /// Rotate files: finalize current writers, rename, check silence, create new writers.
    pub fn rotate_files(&mut self) {
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
        match self.output_mode.as_str() {
            "split" => {
                for (idx, &channel) in self.channels.iter().enumerate() {
                    let final_path =
                        format!("{}/{}-ch{}.wav", self.output_dir, timestamp_now(), channel);
                    let tmp = tmp_wav_path(&final_path);
                    let spec = hound::WavSpec {
                        channels: 1,
                        sample_rate: self.sample_rate,
                        bits_per_sample: 16,
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
            "single" if self.channels.len() > 2 => {
                let final_path =
                    format!("{}/{}-multichannel.wav", self.output_dir, timestamp_now());
                let tmp = tmp_wav_path(&final_path);
                let spec = hound::WavSpec {
                    channels: self.channels.len() as u16,
                    sample_rate: self.sample_rate,
                    bits_per_sample: 16,
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
            _ => {
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
        if read_available(&mut consumer, &mut state) == 0 {
            // Ring buffer empty — sleep briefly to avoid busy-wait
            std::thread::sleep(Duration::from_millis(1));
        }
    }
}
