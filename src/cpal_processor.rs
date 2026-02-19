use std::fs;
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use log::{debug, error, info, warn};

use crate::audio_processor::AudioProcessor;
use crate::config::AppConfig;
use crate::constants::{
    INTERMEDIATE_BUFFER_SIZE, MAX_CHANNELS, MultiChannelWriters, WavWriterType,
};
use crate::error::BlackboxError;
use crate::utils::{check_alsa_availability, is_silent, parse_channel_string};

use chrono::prelude::*;
use cpal::SampleFormat;
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};

/// Returns a timestamp string like "2024-01-15-14-30" from the current local time.
fn timestamp_now() -> String {
    Local::now().format("%Y-%m-%d-%H-%M").to_string()
}

/// Returns a `.recording.wav` temporary path for the given final `.wav` path.
/// Files are written to this path during recording and renamed on finalize,
/// so a crash never leaves a corrupt `.wav` file — only `.recording.wav`.
fn tmp_wav_path(final_path: &str) -> String {
    final_path.replace(".wav", ".recording.wav")
}

/// CpalAudioProcessor handles recording from audio devices using the CPAL library,
/// and saving the audio data to WAV files.
pub struct CpalAudioProcessor {
    writer: Arc<Mutex<Option<WavWriterType>>>,
    multichannel_writers: MultiChannelWriters,
    intermediate_buffer: Arc<Mutex<Vec<i32>>>,
    multichannel_buffers: Arc<Mutex<Vec<Vec<i32>>>>,
    #[allow(dead_code)]
    sample_rate: u32, // Kept for future features that might use it
    // Add a field to keep the stream alive
    #[allow(dead_code)]
    stream: Option<Box<dyn StreamTrait>>,
    // New fields for continuous recording
    continuous_mode: bool,
    recording_cadence: u64,
    output_dir: String,
    last_rotation_time: Arc<Mutex<Instant>>,
    channels: Vec<usize>,
    output_mode: String,
    debug: bool,
    current_spec: Arc<Mutex<Option<hound::WavSpec>>>,
    /// Counts write_sample errors in the audio callback (atomic for RT safety)
    write_errors: Arc<AtomicU64>,
    /// Maps temporary recording paths to their final paths.
    /// Files are written to `.recording.wav` and renamed on finalize.
    pending_files: Arc<Mutex<Vec<(String, String)>>>,
}

impl CpalAudioProcessor {
    /// Create a new CpalAudioProcessor instance.
    ///
    /// This sets up the recording environment, including WAV file writers
    /// and audio stream configuration.
    pub fn new() -> Result<Self, BlackboxError> {
        // Load configuration
        let config = AppConfig::load();

        // Check if ALSA is available on Linux
        check_alsa_availability()?;

        // Get configuration values
        let output_dir = config.get_output_dir();
        let continuous_mode = config.get_continuous_mode();
        let recording_cadence = config.get_recording_cadence();

        // Check and create output directory
        if !Path::new(&output_dir).exists() {
            fs::create_dir_all(&output_dir)?;
        }

        let host = cpal::default_host();
        let device = host
            .default_input_device()
            .ok_or_else(|| BlackboxError::AudioDevice("No input device available".to_string()))?;

        info!(
            "Using audio device: {}",
            device
                .description()
                .map(|d| d.name().to_string())
                .map_err(|e| BlackboxError::AudioDevice(e.to_string()))?
        );

        let config_audio = device.default_input_config().map_err(|e| {
            BlackboxError::AudioDevice(format!("Failed to get default input stream config: {}", e))
        })?;

        debug!("Default input stream config: {:?}", config_audio);

        let sample_rate = config_audio.sample_rate();

        // Placeholder spec — immediately overridden by setup_*_mode() in process_audio()
        let spec = hound::WavSpec {
            channels: 1,
            sample_rate,
            bits_per_sample: 16,
            sample_format: hound::SampleFormat::Int,
        };

        // Create the initial recording file
        let full_path = format!("{}/{}.wav", output_dir, timestamp_now());
        let tmp_path = tmp_wav_path(&full_path);

        let writer = Arc::new(Mutex::new(Some(
            hound::WavWriter::create(&tmp_path, spec)
                .map_err(|e| BlackboxError::Wav(format!("Failed to create WAV file: {}", e)))?,
        )));

        let pending_files = Arc::new(Mutex::new(vec![(tmp_path, full_path)]));

        Ok(CpalAudioProcessor {
            writer,
            multichannel_writers: Arc::new(Mutex::new(Vec::new())),
            intermediate_buffer: Arc::new(Mutex::new(Vec::with_capacity(INTERMEDIATE_BUFFER_SIZE))),
            multichannel_buffers: Arc::new(Mutex::new(Vec::new())),
            sample_rate,
            stream: None,
            continuous_mode,
            recording_cadence,
            output_dir,
            last_rotation_time: Arc::new(Mutex::new(Instant::now())),
            channels: Vec::new(),
            output_mode: String::new(),
            debug: false,
            current_spec: Arc::new(Mutex::new(Some(spec))),
            write_errors: Arc::new(AtomicU64::new(0)),
            pending_files,
        })
    }

    /// Set up split mode recording where each channel is recorded to its own file.
    fn setup_split_mode(&self, channels: &[usize], sample_rate: u32) -> Result<(), BlackboxError> {
        let date_str = timestamp_now();

        info!("Setting up split mode with {} channels", channels.len());

        let mut writers = self.multichannel_writers.lock().unwrap();
        let mut buffers = self.multichannel_buffers.lock().unwrap();
        let mut pending = self.pending_files.lock().unwrap();

        // Ensure the vectors are of the correct size
        writers.clear();
        // Instead of resize, we'll use a loop to add the writers
        for _ in 0..MAX_CHANNELS {
            writers.push(None);
        }

        buffers.clear();
        buffers.resize(channels.len(), Vec::with_capacity(INTERMEDIATE_BUFFER_SIZE));

        // Create a WAV writer for each channel
        for (idx, &channel) in channels.iter().enumerate() {
            let final_path = format!("{}/{}-ch{}.wav", self.output_dir, date_str, channel);
            let tmp_path = tmp_wav_path(&final_path);

            let spec = hound::WavSpec {
                channels: 1, // Mono for each individual channel
                sample_rate,
                bits_per_sample: 16,
                sample_format: hound::SampleFormat::Int,
            };

            let writer = hound::WavWriter::create(&tmp_path, spec).map_err(|e| {
                BlackboxError::Wav(format!("Failed to create channel WAV file: {}", e))
            })?;

            writers[idx] = Some(writer);
            pending.push((tmp_path, final_path.clone()));
            info!("Created channel WAV file: {}", final_path);
        }

        // Store the current configuration
        *self.current_spec.lock().unwrap() = Some(hound::WavSpec {
            channels: 1,
            sample_rate,
            bits_per_sample: 16,
            sample_format: hound::SampleFormat::Int,
        });

        Ok(())
    }

    /// Set up multichannel mode recording where all channels are recorded to a single file.
    fn setup_multichannel_mode(
        &self,
        channels: &[usize],
        sample_rate: u32,
    ) -> Result<(), BlackboxError> {
        let date_str = timestamp_now();

        let final_path = format!("{}/{}-multichannel.wav", self.output_dir, date_str);
        let tmp_path = tmp_wav_path(&final_path);

        info!(
            "Setting up multichannel mode with {} channels",
            channels.len()
        );

        let spec = hound::WavSpec {
            channels: channels.len() as u16,
            sample_rate,
            bits_per_sample: 16,
            sample_format: hound::SampleFormat::Int,
        };

        let writer = hound::WavWriter::create(&tmp_path, spec).map_err(|e| {
            BlackboxError::Wav(format!("Failed to create multichannel WAV file: {}", e))
        })?;

        // Replace the default stereo writer with our multichannel writer
        let mut writer_guard = self.writer.lock().unwrap();
        *writer_guard = Some(writer);

        // Store the current configuration
        *self.current_spec.lock().unwrap() = Some(spec);

        // Track pending file for atomic rename on finalize
        let mut pending = self.pending_files.lock().unwrap();
        // Replace the placeholder entry from new()
        pending.clear();
        pending.push((tmp_path, final_path.clone()));

        info!("Created multichannel WAV file: {}", final_path);
        Ok(())
    }

    /// Set up standard mode recording for mono or stereo (1 or 2 channels).
    fn setup_standard_mode(
        &self,
        channels: &[usize],
        sample_rate: u32,
    ) -> Result<(), BlackboxError> {
        let date_str = timestamp_now();

        info!("Setting up standard mode with {} channels", channels.len());

        // Determine if we're recording mono or stereo
        let num_channels = if channels.len() == 1 { 1 } else { 2 };

        // Create the WAV file
        let final_path = format!("{}/{}.wav", self.output_dir, date_str);
        let tmp_path = tmp_wav_path(&final_path);

        let spec = hound::WavSpec {
            channels: num_channels as u16,
            sample_rate,
            bits_per_sample: 16,
            sample_format: hound::SampleFormat::Int,
        };

        let writer = hound::WavWriter::create(&tmp_path, spec)
            .map_err(|e| BlackboxError::Wav(format!("Failed to create WAV file: {}", e)))?;

        // Store the writer
        *self.writer.lock().unwrap() = Some(writer);
        info!("Created WAV file: {}", final_path);

        // Store the current configuration
        *self.current_spec.lock().unwrap() = Some(spec);

        // Track pending file for atomic rename on finalize
        let mut pending = self.pending_files.lock().unwrap();
        // Replace the placeholder entry from new()
        pending.clear();
        pending.push((tmp_path, final_path));

        // Initialize buffers
        let mut buffers = self.multichannel_buffers.lock().unwrap();
        buffers.clear();
        buffers.resize(channels.len(), Vec::with_capacity(INTERMEDIATE_BUFFER_SIZE));

        Ok(())
    }
}

impl AudioProcessor for CpalAudioProcessor {
    fn process_audio(
        &mut self,
        channels: &[usize],
        output_mode: &str,
        debug: bool,
    ) -> Result<(), BlackboxError> {
        // Store the configuration for later use in continuous mode and finalize
        self.channels = channels.to_vec();
        self.output_mode = output_mode.to_string();
        self.debug = debug;

        // Get CPAL host and device
        let host = cpal::default_host();
        let device = host
            .default_input_device()
            .ok_or_else(|| BlackboxError::AudioDevice("No input device available".to_string()))?;

        info!(
            "Using audio device: {}",
            device
                .description()
                .map_or_else(|_| "unknown".to_string(), |d| d.name().to_string())
        );

        let config = device.default_input_config().map_err(|e| {
            BlackboxError::AudioDevice(format!("Failed to get default input stream config: {}", e))
        })?;

        debug!("Default input stream config: {:?}", config);

        let total_channels = config.channels() as usize;
        let sample_rate = config.sample_rate();

        // Auto-adapt to available channels
        let mut actual_channels: Vec<usize> = Vec::new();

        // Validate and adapt channels
        for &channel in channels {
            if channel < total_channels {
                actual_channels.push(channel);
            } else {
                warn!(
                    "Channel {} not available on device. Device only has {} channels.",
                    channel, total_channels
                );
            }
        }

        // If no requested channels are available, use all available channels
        if actual_channels.is_empty() {
            warn!(
                "No requested channels available. Using all available channels (0 to {}).",
                total_channels - 1
            );
            actual_channels = (0..total_channels).collect();
        }

        info!("Using channels: {:?}", actual_channels);

        // Setup the appropriate output mode
        let valid_modes = ["split", "single"];
        if !valid_modes.contains(&output_mode) {
            return Err(BlackboxError::Config(format!(
                "Invalid output mode: '{}'. Valid options are: {:?}",
                output_mode, valid_modes
            )));
        }

        match output_mode {
            "split" => {
                self.setup_split_mode(&actual_channels, sample_rate)?;
            }
            "single" if actual_channels.len() <= 2 => {
                self.setup_standard_mode(&actual_channels, sample_rate)?;
            }
            "single" => {
                self.setup_multichannel_mode(&actual_channels, sample_rate)?;
            }
            _ => {
                return Err(BlackboxError::Config(format!(
                    "Unexpected output mode: '{}'",
                    output_mode
                )));
            }
        }

        // Clone channels to own them in the closure
        let channels_owned: Vec<usize> = actual_channels.clone();
        let output_mode_owned = output_mode.to_string();

        let writer_clone = Arc::clone(&self.writer);
        let buffer_clone = Arc::clone(&self.intermediate_buffer);
        let multichannel_writers_clone = Arc::clone(&self.multichannel_writers);
        let multichannel_buffers_clone = Arc::clone(&self.multichannel_buffers);

        // For continuous mode, we need to check if we should rotate files
        let continuous_mode = self.continuous_mode;
        let last_rotation_time_clone = Arc::clone(&self.last_rotation_time);
        let recording_cadence = self.recording_cadence;

        // Capture silence threshold from config before entering the closure
        // so we don't call env::var on the real-time audio thread
        let silence_threshold = AppConfig::load().get_silence_threshold();

        // Clone shared state for the audio callback's rotation logic
        let output_dir = self.output_dir.clone();
        let current_spec = Arc::clone(&self.current_spec);
        let writer_for_rotation = Arc::clone(&self.writer);
        let multichannel_writers_for_rotation = Arc::clone(&self.multichannel_writers);
        let sample_rate = self.sample_rate;
        let write_errors = Arc::clone(&self.write_errors);
        let pending_files_for_rotation = Arc::clone(&self.pending_files);

        // Error callback
        let err_fn = move |err| {
            error!("an error occurred on stream: {}", err);
        };

        // Create a stream based on the sample format
        let stream = match config.sample_format() {
            SampleFormat::F32 => {
                // Build a stream for f32 samples
                device.build_input_stream(
                    &config.into(),
                    move |data: &[f32], _: &_| {
                        if debug {
                            debug!("Processing {} samples", data.len());
                        }

                        // Check if we need to rotate files in continuous mode
                        if continuous_mode {
                            let now = Instant::now();
                            let last_rotation = *last_rotation_time_clone.lock().unwrap();

                            if now.duration_since(last_rotation) >= Duration::from_secs(recording_cadence) {
                                // Perform file rotation using thread-safe mechanisms
                                info!("Rotating recording files...");

                                // Use the configuration captured at process_audio() start
                                let output_mode = &output_mode_owned;
                                let channels = &channels_owned;

                                // Take all pending (tmp → final) pairs from the previous period
                                let old_pending: Vec<(String, String)> = {
                                    let mut pf = pending_files_for_rotation.lock().unwrap();
                                    std::mem::take(&mut *pf)
                                };

                                // Finalize the main WAV file if it exists
                                if let Some(writer) = writer_for_rotation.lock().unwrap().take()
                                    && let Err(e) = writer.finalize()
                                {
                                    error!("Error finalizing WAV file during rotation: {}", e);
                                }

                                // Finalize any multichannel writers
                                let mut writers = multichannel_writers_for_rotation.lock().unwrap();
                                for writer_opt in writers.iter_mut() {
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

                                // Check for silence and delete silent files
                                if silence_threshold > 0.0 {
                                    for file_path in final_files {
                                        match is_silent(&file_path, silence_threshold) {
                                            Ok(true) => {
                                                info!("Recording is silent (below threshold {}), deleting file", silence_threshold);
                                                if let Err(e) = fs::remove_file(&file_path) {
                                                    error!("Error deleting silent file: {}", e);
                                                }
                                            }
                                            Ok(false) => {
                                                info!("Recording is not silent (above threshold {}), keeping file", silence_threshold);
                                            }
                                            Err(e) => {
                                                error!("Error checking for silence: {}", e);
                                            }
                                        }
                                    }
                                }

                                // Create new files for the next recording period
                                let mut new_pending = Vec::new();
                                match output_mode.as_str() {
                                    "split" => {
                                        for (idx, &channel) in channels.iter().enumerate() {
                                            let final_path = format!("{}/{}-ch{}.wav", output_dir, timestamp_now(), channel);
                                            let tmp = tmp_wav_path(&final_path);
                                            let spec = hound::WavSpec {
                                                channels: 1,
                                                sample_rate,
                                                bits_per_sample: 16,
                                                sample_format: hound::SampleFormat::Int,
                                            };
                                            match hound::WavWriter::create(&tmp, spec) {
                                                Ok(w) => {
                                                    writers[idx] = Some(w);
                                                    new_pending.push((tmp, final_path.clone()));
                                                    info!("Created channel WAV file: {}", final_path);
                                                }
                                                Err(e) => {
                                                    error!("Failed to create channel WAV file: {}", e);
                                                }
                                            }
                                        }
                                    }
                                    "single" if channels.len() > 2 => {
                                        let final_path = format!("{}/{}-multichannel.wav", output_dir, timestamp_now());
                                        let tmp = tmp_wav_path(&final_path);
                                        let spec = hound::WavSpec {
                                            channels: channels.len() as u16,
                                            sample_rate,
                                            bits_per_sample: 16,
                                            sample_format: hound::SampleFormat::Int,
                                        };
                                        match hound::WavWriter::create(&tmp, spec) {
                                            Ok(w) => {
                                                *writer_for_rotation.lock().unwrap() = Some(w);
                                                new_pending.push((tmp, final_path.clone()));
                                                info!("Created multichannel WAV file: {}", final_path);
                                            }
                                            Err(e) => {
                                                error!("Failed to create multichannel WAV file: {}", e);
                                            }
                                        }
                                    }
                                    _ => {
                                        let final_path = format!("{}/{}.wav", output_dir, timestamp_now());
                                        let tmp = tmp_wav_path(&final_path);
                                        if let Some(spec) = &*current_spec.lock().unwrap() {
                                            match hound::WavWriter::create(&tmp, *spec) {
                                                Ok(w) => {
                                                    *writer_for_rotation.lock().unwrap() = Some(w);
                                                    new_pending.push((tmp, final_path.clone()));
                                                    info!("Created new recording file: {}", final_path);
                                                }
                                                Err(e) => {
                                                    error!("Failed to create new WAV file: {}", e);
                                                }
                                            }
                                        }
                                    }
                                }

                                // Update shared pending files for the new recording period
                                *pending_files_for_rotation.lock().unwrap() = new_pending;

                                // Reset the rotation timer
                                *last_rotation_time_clone.lock().unwrap() = Instant::now();
                            }
                        }

                        // Process the audio data based on the output mode
                        match output_mode_owned.as_str() {
                            "split" => {
                                // Split mode: write each channel to its own file
                                let mut writers = multichannel_writers_clone.lock().unwrap();
                                let mut buffers = multichannel_buffers_clone.lock().unwrap();

                                // Process each frame (a frame contains one sample for each channel)
                                let frame_size = total_channels;
                                let frames = data.chunks(frame_size);

                                for frame in frames {
                                    // Extract and write the selected channels
                                    for (idx, &channel) in channels_owned.iter().enumerate() {
                                        if channel < frame.len()
                                            && let Some(writer) = &mut writers[idx]
                                        {
                                            // Convert f32 to i16 range
                                            let sample = (frame[channel] * 32767.0) as i32;
                                            if writer.write_sample(sample).is_err() {
                                                write_errors.fetch_add(1, Ordering::Relaxed);
                                            }

                                            // Also store in the buffer for later processing
                                            if idx < buffers.len() {
                                                buffers[idx].push(sample);
                                                if buffers[idx].len() >= INTERMEDIATE_BUFFER_SIZE {
                                                    buffers[idx].clear();
                                                }
                                            }
                                        }
                                    }
                                }
                            }
                            "single" if channels_owned.len() > 2 => {
                                // Multichannel mode: write all selected channels to one file
                                let mut writer_guard = writer_clone.lock().unwrap();
                                if let Some(writer) = &mut *writer_guard {
                                    // Process each frame
                                    let frame_size = total_channels;
                                    let frames = data.chunks(frame_size);

                                    for frame in frames {
                                        // Write only the selected channels
                                        for &channel in &channels_owned {
                                            if channel < frame.len() {
                                                // Convert f32 to i16 range
                                                let sample = (frame[channel] * 32767.0) as i32;
                                                if writer.write_sample(sample).is_err() {
                                                    write_errors.fetch_add(1, Ordering::Relaxed);
                                                }
                                            }
                                        }
                                    }
                                }
                            }
                            _ => {
                                // Standard mode: write the requested channels
                                let mut writer_guard = writer_clone.lock().unwrap();
                                let mut buffer = buffer_clone.lock().unwrap();

                                if let Some(writer) = &mut *writer_guard {
                                    let frame_size = total_channels;
                                    let frames = data.chunks(frame_size);

                                    for frame in frames {
                                        for &channel in &channels_owned {
                                            if channel < frame.len() {
                                                let sample = (frame[channel] * 32767.0) as i32;
                                                if writer.write_sample(sample).is_err() {
                                                    write_errors.fetch_add(1, Ordering::Relaxed);
                                                }
                                                buffer.push(sample);
                                            }
                                        }

                                        if buffer.len() >= INTERMEDIATE_BUFFER_SIZE {
                                            buffer.clear();
                                        }
                                    }
                                }
                            }
                        }
                    },
                    err_fn,
                    None,
                ).map_err(|e| {
                    BlackboxError::AudioDevice(format!("Failed to build input stream: {}", e))
                })?
            }
            _ => {
                return Err(BlackboxError::AudioDevice(format!(
                    "Unsupported sample format: {:?}",
                    config.sample_format()
                )));
            }
        };

        // Start recording
        stream
            .play()
            .map_err(|e| BlackboxError::AudioDevice(format!("Failed to play stream: {}", e)))?;

        // Store the stream to keep it alive during recording
        self.stream = Some(Box::new(stream));

        // In continuous mode, initialize the rotation timer
        if self.continuous_mode {
            *self.last_rotation_time.lock().unwrap() = Instant::now();
        }

        Ok(())
    }

    fn finalize(&mut self) -> Result<(), BlackboxError> {
        let config = AppConfig::load();
        let silence_threshold = config.get_silence_threshold();

        // Report any write errors that accumulated during recording
        let errors = self.write_errors.load(Ordering::Relaxed);
        if errors > 0 {
            warn!("{} sample write errors occurred during recording", errors);
        }

        // Finalize the main WAV file
        if let Some(writer) = self.writer.lock().unwrap().take() {
            writer
                .finalize()
                .map_err(|e| BlackboxError::Wav(format!("Error finalizing WAV file: {}", e)))?;
        }

        // Finalize any multichannel writers
        {
            let mut writers = self.multichannel_writers.lock().unwrap();
            for writer_opt in writers.iter_mut() {
                if let Some(writer) = writer_opt.take() {
                    writer.finalize().map_err(|e| {
                        BlackboxError::Wav(format!("Error finalizing channel WAV file: {}", e))
                    })?;
                }
            }
        }

        // Close the stream
        self.stream = None;

        // Rename all pending .recording.wav files to their final .wav paths
        let pending = std::mem::take(&mut *self.pending_files.lock().unwrap());
        let mut final_files = Vec::new();
        for (tmp_path, final_path) in &pending {
            if Path::new(tmp_path).exists() {
                fs::rename(tmp_path, final_path)?;
                info!("Finalized recording to {}", final_path);
                final_files.push(final_path.clone());
            }
        }

        // Check silence and delete silent files
        if silence_threshold > 0.0 {
            for file_path in final_files {
                match is_silent(&file_path, silence_threshold) {
                    Ok(true) => {
                        info!(
                            "Recording is silent (below threshold {}), deleting file: {}",
                            silence_threshold, file_path
                        );
                        fs::remove_file(&file_path)?;
                    }
                    Ok(false) => {
                        info!(
                            "Recording is not silent (above threshold {}), keeping file: {}",
                            silence_threshold, file_path
                        );
                    }
                    Err(e) => {
                        error!("Error checking for silence: {}", e);
                        return Err(e);
                    }
                }
            }
        }

        Ok(())
    }

    fn start_recording(&mut self) -> Result<(), BlackboxError> {
        let config = AppConfig::load();
        let channels_str = config.get_audio_channels();
        let channels = parse_channel_string(&channels_str)?;

        let output_mode = config.get_output_mode();
        let debug = config.get_debug();

        self.process_audio(&channels, &output_mode, debug)
    }

    fn stop_recording(&mut self) -> Result<(), BlackboxError> {
        self.finalize()
    }

    fn is_recording(&self) -> bool {
        // If there's an active stream, we're recording
        self.stream.is_some()
    }
}

// Add Drop implementation to ensure cleanup
impl Drop for CpalAudioProcessor {
    fn drop(&mut self) {
        // Try to finalize if we're still recording
        if self.is_recording()
            && let Err(e) = self.finalize()
        {
            error!("Error during cleanup: {}", e);
        }
    }
}
