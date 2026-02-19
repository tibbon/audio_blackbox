use std::fs;
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::{Duration, Instant};

use log::{debug, error, info, warn};

use crate::audio_processor::AudioProcessor;
use crate::config::AppConfig;
use crate::constants::RING_BUFFER_SECONDS;
use crate::error::BlackboxError;
use crate::utils::{check_alsa_availability, parse_channel_string};
use crate::writer_thread::{
    WriterCommand, WriterThreadHandle, WriterThreadState, writer_thread_main,
};

use cpal::SampleFormat;
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};

/// CpalAudioProcessor handles recording from audio devices using the CPAL library,
/// and saving the audio data to WAV files.
///
/// File I/O is performed on a dedicated writer thread. The cpal audio callback
/// pushes raw f32 samples into a lock-free SPSC ring buffer (via `rtrb`),
/// keeping the real-time thread free of blocking operations.
pub struct CpalAudioProcessor {
    #[allow(dead_code)]
    sample_rate: u32,
    stream: Option<Box<dyn StreamTrait>>,
    continuous_mode: bool,
    recording_cadence: u64,
    output_dir: String,
    channels: Vec<usize>,
    output_mode: String,
    debug: bool,
    /// Counts write_sample errors and ring buffer overflow drops (atomic for RT safety).
    write_errors: Arc<AtomicU64>,
    /// Set by the writer thread when disk space drops below threshold.
    disk_space_low: Arc<AtomicBool>,
    /// Handle to the writer thread (None before process_audio, None after finalize).
    writer_thread: Option<WriterThreadHandle>,
    /// Test-only: bypass ring buffer and writer thread, write directly.
    #[cfg(test)]
    direct_state: Option<WriterThreadState>,
}

impl CpalAudioProcessor {
    /// Create a new CpalAudioProcessor instance, loading config from env/TOML.
    ///
    /// Probes the audio device for sample rate and stores config.
    /// WAV writers are not created until `process_audio()` is called.
    pub fn new() -> Result<Self, BlackboxError> {
        Self::with_config(&AppConfig::load())
    }

    /// Create a new CpalAudioProcessor using the provided configuration.
    pub fn with_config(config: &AppConfig) -> Result<Self, BlackboxError> {
        check_alsa_availability()?;

        let output_dir = config.get_output_dir();
        let continuous_mode = config.get_continuous_mode();
        let recording_cadence = config.get_recording_cadence();

        if !Path::new(&output_dir).exists() {
            fs::create_dir_all(&output_dir)?;
        }

        let host = cpal::default_host();
        let device = Self::find_input_device(&host, config.get_input_device().as_deref())?;

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

        Ok(CpalAudioProcessor {
            sample_rate,
            stream: None,
            continuous_mode,
            recording_cadence,
            output_dir,
            channels: Vec::new(),
            output_mode: String::new(),
            debug: false,
            write_errors: Arc::new(AtomicU64::new(0)),
            disk_space_low: Arc::new(AtomicBool::new(false)),
            writer_thread: None,
            #[cfg(test)]
            direct_state: None,
        })
    }

    /// Find an input device by name, or return the default input device.
    fn find_input_device(
        host: &cpal::Host,
        device_name: Option<&str>,
    ) -> Result<cpal::Device, BlackboxError> {
        if let Some(name) = device_name {
            let devices = host.input_devices().map_err(|e| {
                BlackboxError::AudioDevice(format!("Failed to enumerate input devices: {}", e))
            })?;
            for device in devices {
                if let Ok(desc) = device.description()
                    && desc.name() == name
                {
                    return Ok(device);
                }
            }
            warn!("Input device '{}' not found, falling back to default", name);
        }
        host.default_input_device()
            .ok_or_else(|| BlackboxError::AudioDevice("No input device available".to_string()))
    }

    /// List all available input device names.
    pub fn list_input_devices() -> Result<Vec<String>, BlackboxError> {
        let host = cpal::default_host();
        let devices = host.input_devices().map_err(|e| {
            BlackboxError::AudioDevice(format!("Failed to enumerate input devices: {}", e))
        })?;
        let mut names = Vec::new();
        for device in devices {
            if let Ok(desc) = device.description() {
                names.push(desc.name().to_string());
            }
        }
        Ok(names)
    }
}

impl AudioProcessor for CpalAudioProcessor {
    fn process_audio(
        &mut self,
        channels: &[usize],
        output_mode: &str,
        debug: bool,
    ) -> Result<(), BlackboxError> {
        self.channels = channels.to_vec();
        self.output_mode = output_mode.to_string();
        self.debug = debug;

        let host = cpal::default_host();
        let app_config = AppConfig::load();
        let device = Self::find_input_device(&host, app_config.get_input_device().as_deref())?;

        info!(
            "Using audio device: {}",
            device
                .description()
                .map_or_else(|_| "unknown".to_string(), |d| d.name().to_string())
        );

        // Use the device's current default config (sample rate, channels, format).
        // This avoids changing kAudioDevicePropertyNominalSampleRate on macOS,
        // which would conflict with DAWs and other pro audio apps sharing the device.
        let config = device.default_input_config().map_err(|e| {
            BlackboxError::AudioDevice(format!("Failed to get default input stream config: {}", e))
        })?;

        debug!("Default input stream config: {:?}", config);

        let total_channels = config.channels() as usize;
        let sample_rate = config.sample_rate();

        // Auto-adapt to available channels
        let mut actual_channels: Vec<usize> = Vec::new();
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

        if actual_channels.is_empty() {
            warn!(
                "No requested channels available. Using all available channels (0 to {}).",
                total_channels - 1
            );
            actual_channels = (0..total_channels).collect();
        }

        info!("Using channels: {:?}", actual_channels);

        // Validate output mode
        let valid_modes = ["split", "single"];
        if !valid_modes.contains(&output_mode) {
            return Err(BlackboxError::Config(format!(
                "Invalid output mode: '{}'. Valid options are: {:?}",
                output_mode, valid_modes
            )));
        }

        // Capture config values before entering the closure
        let loaded_config = AppConfig::load();
        let silence_threshold = loaded_config.get_silence_threshold();
        let min_disk_space_mb = loaded_config.get_min_disk_space_mb();

        // Create writer thread state with initial WAV writers
        let mut state = WriterThreadState::new(
            &self.output_dir,
            sample_rate,
            &actual_channels,
            output_mode,
            silence_threshold,
            Arc::clone(&self.write_errors),
            debug,
            min_disk_space_mb,
            Arc::clone(&self.disk_space_low),
        )?;
        state.total_device_channels = total_channels;

        // Create ring buffer
        let ring_size = sample_rate as usize * total_channels * RING_BUFFER_SECONDS;
        let (mut producer, consumer) = rtrb::RingBuffer::new(ring_size);

        // Create rotation flag and command channel
        let rotation_needed = Arc::new(AtomicBool::new(false));
        let (command_tx, command_rx) = std::sync::mpsc::sync_channel::<WriterCommand>(1);

        // Clone for the writer thread
        let rotation_needed_writer = Arc::clone(&rotation_needed);

        // Spawn writer thread
        let join_handle = std::thread::Builder::new()
            .name("blackbox-writer".to_string())
            .spawn(move || {
                writer_thread_main(consumer, rotation_needed_writer, command_rx, state);
            })
            .map_err(|e| {
                BlackboxError::AudioDevice(format!("Failed to spawn writer thread: {}", e))
            })?;

        // Store handle (producer goes to the callback, not into the handle)
        self.writer_thread = Some(WriterThreadHandle {
            rotation_needed: Arc::clone(&rotation_needed),
            command_tx,
            join_handle: Some(join_handle),
            disk_space_low: Arc::clone(&self.disk_space_low),
        });

        // Clone write_errors for the callback
        let write_errors = Arc::clone(&self.write_errors);
        let continuous_mode = self.continuous_mode;
        let recording_cadence = self.recording_cadence;
        let rotation_needed_cb = Arc::clone(&rotation_needed);

        // Error callback
        let err_fn = move |err| {
            error!("an error occurred on stream: {}", err);
        };

        // Local rotation timer for the callback (plain variable, not Arc<Mutex>)
        let mut last_rotation = Instant::now();

        // Build the input stream
        let stream = match config.sample_format() {
            SampleFormat::F32 => {
                device
                    .build_input_stream(
                        &config.into(),
                        move |data: &[f32], _: &_| {
                            if debug {
                                debug!("Processing {} samples", data.len());
                            }

                            // Check rotation timer (only reads Instant::now + comparison, no mutex)
                            if continuous_mode {
                                let now = Instant::now();
                                if now.duration_since(last_rotation)
                                    >= Duration::from_secs(recording_cadence)
                                {
                                    rotation_needed_cb.store(true, Ordering::Release);
                                    last_rotation = now;
                                }
                            }

                            // Push raw f32 to ring buffer (lock-free, wait-free, zero I/O)
                            match producer.write_chunk_uninit(data.len()) {
                                Ok(chunk) => {
                                    chunk.fill_from_iter(data.iter().copied());
                                }
                                Err(_) => {
                                    // Ring buffer full — drop samples, count errors
                                    write_errors.fetch_add(data.len() as u64, Ordering::Relaxed);
                                }
                            }
                        },
                        err_fn,
                        None,
                    )
                    .map_err(|e| {
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

        self.stream = Some(Box::new(stream));

        Ok(())
    }

    fn finalize(&mut self) -> Result<(), BlackboxError> {
        let errors = self.write_errors.load(Ordering::Relaxed);
        if errors > 0 {
            warn!(
                "{} sample write/overflow errors occurred during recording",
                errors
            );
        }

        // Drop stream first — no more data will be pushed to the ring buffer
        self.stream = None;

        // Signal writer thread to drain + shutdown
        if let Some(mut handle) = self.writer_thread.take() {
            let (reply_tx, reply_rx) = std::sync::mpsc::channel();
            let got_reply = if handle
                .command_tx
                .send(WriterCommand::Shutdown(reply_tx))
                .is_ok()
            {
                if let Ok(result) = reply_rx.recv_timeout(Duration::from_secs(30)) {
                    result?;
                    true
                } else {
                    warn!("Writer thread shutdown timed out");
                    false
                }
            } else {
                false
            };
            // Only join if the thread acknowledged shutdown; otherwise let it detach
            // to avoid hanging the app on quit.
            if got_reply {
                if let Some(jh) = handle.join_handle.take() {
                    let _ = jh.join();
                }
            } else {
                warn!("Writer thread did not respond — detaching to avoid hang");
            }
        }

        #[cfg(test)]
        if let Some(mut state) = self.direct_state.take() {
            return state.finalize_all();
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
        self.stream.is_some() || self.writer_thread.is_some()
    }

    fn write_error_count(&self) -> u64 {
        self.write_errors.load(Ordering::Relaxed)
    }

    fn disk_space_low(&self) -> bool {
        self.disk_space_low.load(Ordering::Relaxed)
    }
}

impl Drop for CpalAudioProcessor {
    fn drop(&mut self) {
        if self.is_recording()
            && let Err(e) = self.finalize()
        {
            error!("Error during cleanup: {}", e);
        }
    }
}

#[cfg(test)]
impl CpalAudioProcessor {
    /// Create a `CpalAudioProcessor` for testing without requiring audio hardware.
    ///
    /// Uses `WriterThreadState` directly (no ring buffer or writer thread).
    pub fn new_for_test(
        output_dir: &str,
        sample_rate: u32,
        channels: &[usize],
        output_mode: &str,
    ) -> Result<Self, BlackboxError> {
        if !Path::new(output_dir).exists() {
            fs::create_dir_all(output_dir)?;
        }

        let write_errors = Arc::new(AtomicU64::new(0));

        let disk_space_low = Arc::new(AtomicBool::new(false));

        let mut state = WriterThreadState::new(
            output_dir,
            sample_rate,
            channels,
            output_mode,
            AppConfig::load().get_silence_threshold(),
            Arc::clone(&write_errors),
            false,
            0, // disable disk check in tests
            Arc::clone(&disk_space_low),
        )?;
        // For tests, total_device_channels is set per feed_test_data call
        state.total_device_channels = 0;

        Ok(CpalAudioProcessor {
            sample_rate,
            stream: None,
            continuous_mode: false,
            recording_cadence: 0,
            output_dir: output_dir.to_string(),
            channels: channels.to_vec(),
            output_mode: output_mode.to_string(),
            debug: false,
            write_errors,
            disk_space_low,
            writer_thread: None,
            direct_state: Some(state),
        })
    }

    /// Feed interleaved f32 audio data as if it came from a cpal callback.
    pub fn feed_test_data(&mut self, data: &[f32], total_device_channels: usize) {
        if let Some(ref mut state) = self.direct_state {
            state.total_device_channels = total_device_channels;
            state.write_samples(data);
        }
    }

    /// Return the current write-error count.
    pub fn test_write_error_count(&self) -> u64 {
        self.write_errors.load(Ordering::Relaxed)
    }

    /// Return a clone of the pending (tmp, final) path pairs.
    pub fn test_pending_files(&self) -> Vec<(String, String)> {
        self.direct_state
            .as_ref()
            .map_or_else(Vec::new, |s| s.pending_files.clone())
    }
}
