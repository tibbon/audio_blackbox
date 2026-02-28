use std::fs;
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::{Duration, Instant};

use log::{debug, error, info, warn};

use crate::audio_processor::AudioProcessor;
use crate::config::AppConfig;
use crate::constants::{CacheAlignedPeak, RING_BUFFER_SECONDS};
use crate::error::BlackboxError;
use crate::utils::{check_alsa_availability, parse_channel_string};
use crate::writer_thread::{
    WriterCommand, WriterThreadHandle, WriterThreadState, writer_thread_main,
};

use cpal::SampleFormat;
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};

// ---------------------------------------------------------------------------
// macOS CoreAudio sample rate change listener
// ---------------------------------------------------------------------------

/// Registers a CoreAudio property listener on `kAudioDevicePropertyNominalSampleRate`
/// for the active input device. When the sample rate changes, sets an `AtomicBool`
/// flag that the Swift UI polling loop can detect and restart the recording with
/// the correct sample rate in the new WAV header.
#[cfg(target_os = "macos")]
mod sample_rate_listener {
    use std::ffi::c_void;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, Ordering};

    use core_foundation::base::TCFType;
    use core_foundation::string::{CFString, CFStringRef};
    use log::{info, warn};

    type AudioObjectID = u32;

    #[repr(C)]
    #[derive(Clone, Copy)]
    struct PropAddr {
        selector: u32,
        scope: u32,
        element: u32,
    }

    // CoreAudio FourCC constants
    const SYSTEM_OBJECT: AudioObjectID = 1;
    const SEL_DEVICES: u32 = u32::from_be_bytes(*b"dev#");
    const SEL_DEFAULT_INPUT: u32 = u32::from_be_bytes(*b"dIn ");
    const SEL_NAME: u32 = u32::from_be_bytes(*b"lnam");
    const SEL_NOMINAL_RATE: u32 = u32::from_be_bytes(*b"nsrt");
    const SCOPE_GLOBAL: u32 = u32::from_be_bytes(*b"glob");
    const ELEMENT_MAIN: u32 = 0;

    type ListenerProc =
        unsafe extern "C" fn(AudioObjectID, u32, *const PropAddr, *mut c_void) -> i32;

    #[link(name = "CoreAudio", kind = "framework")]
    unsafe extern "C" {
        unsafe fn AudioObjectGetPropertyDataSize(
            id: AudioObjectID,
            addr: *const PropAddr,
            qual_size: u32,
            qual: *const c_void,
            out_size: *mut u32,
        ) -> i32;

        unsafe fn AudioObjectGetPropertyData(
            id: AudioObjectID,
            addr: *const PropAddr,
            qual_size: u32,
            qual: *const c_void,
            io_size: *mut u32,
            out_data: *mut c_void,
        ) -> i32;

        unsafe fn AudioObjectAddPropertyListener(
            id: AudioObjectID,
            addr: *const PropAddr,
            listener: ListenerProc,
            client_data: *mut c_void,
        ) -> i32;

        unsafe fn AudioObjectRemovePropertyListener(
            id: AudioObjectID,
            addr: *const PropAddr,
            listener: ListenerProc,
            client_data: *mut c_void,
        ) -> i32;
    }

    /// RAII guard: registers a CoreAudio property listener on creation, removes on drop.
    pub(super) struct SampleRateListener {
        device_id: AudioObjectID,
        /// Raw pointer to an `Arc<AtomicBool>` — we own one strong reference.
        client_data: *mut c_void,
    }

    // SAFETY: `client_data` points to an `AtomicBool` (Send+Sync), `device_id` is Copy.
    unsafe impl Send for SampleRateListener {}

    impl SampleRateListener {
        /// Register a CoreAudio listener for sample rate changes on the given device.
        /// Returns `None` if the device can't be found or registration fails.
        pub fn new(device_name: Option<&str>, flag: Arc<AtomicBool>) -> Option<Self> {
            let device_id = find_device_id(device_name)?;
            let client_data = Arc::into_raw(Arc::clone(&flag)) as *mut c_void;

            let status = unsafe {
                AudioObjectAddPropertyListener(
                    device_id,
                    &rate_addr(),
                    on_rate_changed,
                    client_data,
                )
            };

            if status != 0 {
                // Reclaim the Arc since registration failed
                unsafe {
                    drop(Arc::from_raw(client_data as *const AtomicBool));
                }
                warn!(
                    "Failed to register sample rate listener (status {})",
                    status
                );
                return None;
            }

            info!("Registered sample rate listener on device {}", device_id);
            Some(Self {
                device_id,
                client_data,
            })
        }
    }

    impl Drop for SampleRateListener {
        fn drop(&mut self) {
            let status = unsafe {
                AudioObjectRemovePropertyListener(
                    self.device_id,
                    &rate_addr(),
                    on_rate_changed,
                    self.client_data,
                )
            };
            if status == 0 {
                // Successfully removed — safe to reclaim the Arc
                unsafe {
                    drop(Arc::from_raw(self.client_data as *const AtomicBool));
                }
            } else {
                // Listener still registered — leak the Arc to avoid use-after-free
                warn!(
                    "Failed to remove sample rate listener (status {})",
                    status
                );
            }
        }
    }

    fn rate_addr() -> PropAddr {
        PropAddr {
            selector: SEL_NOMINAL_RATE,
            scope: SCOPE_GLOBAL,
            element: ELEMENT_MAIN,
        }
    }

    /// CoreAudio callback — runs on an internal CoreAudio thread.
    unsafe extern "C" fn on_rate_changed(
        _id: AudioObjectID,
        _count: u32,
        _addrs: *const PropAddr,
        client_data: *mut c_void,
    ) -> i32 {
        if !client_data.is_null() {
            let flag = unsafe { &*(client_data as *const AtomicBool) };
            flag.store(true, Ordering::Release);
        }
        0
    }

    // --- Device lookup helpers ---

    fn find_device_id(name: Option<&str>) -> Option<AudioObjectID> {
        match name {
            Some(n) if !n.is_empty() => device_by_name(n).or_else(default_input_device),
            _ => default_input_device(),
        }
    }

    fn default_input_device() -> Option<AudioObjectID> {
        let addr = PropAddr {
            selector: SEL_DEFAULT_INPUT,
            scope: SCOPE_GLOBAL,
            element: ELEMENT_MAIN,
        };
        let mut device_id: AudioObjectID = 0;
        let mut size = size_of::<AudioObjectID>() as u32;

        let status = unsafe {
            AudioObjectGetPropertyData(
                SYSTEM_OBJECT,
                &raw const addr,
                0,
                std::ptr::null(),
                &raw mut size,
                (&raw mut device_id).cast::<c_void>(),
            )
        };
        (status == 0 && device_id != 0).then_some(device_id)
    }

    fn device_by_name(name: &str) -> Option<AudioObjectID> {
        let addr = PropAddr {
            selector: SEL_DEVICES,
            scope: SCOPE_GLOBAL,
            element: ELEMENT_MAIN,
        };

        let mut size: u32 = 0;
        let status = unsafe {
            AudioObjectGetPropertyDataSize(
                SYSTEM_OBJECT,
                &raw const addr,
                0,
                std::ptr::null(),
                &raw mut size,
            )
        };
        if status != 0 || size == 0 {
            return None;
        }

        let count = size as usize / size_of::<AudioObjectID>();
        let mut ids = vec![0u32; count];

        let status = unsafe {
            AudioObjectGetPropertyData(
                SYSTEM_OBJECT,
                &raw const addr,
                0,
                std::ptr::null(),
                &raw mut size,
                ids.as_mut_ptr().cast::<c_void>(),
            )
        };
        if status != 0 {
            return None;
        }

        ids.iter()
            .copied()
            .find(|&id| device_name(id).is_some_and(|n| n == name))
    }

    fn device_name(device_id: AudioObjectID) -> Option<String> {
        let addr = PropAddr {
            selector: SEL_NAME,
            scope: SCOPE_GLOBAL,
            element: ELEMENT_MAIN,
        };
        let mut name_ref: CFStringRef = std::ptr::null();
        let mut size = size_of::<CFStringRef>() as u32;

        let status = unsafe {
            AudioObjectGetPropertyData(
                device_id,
                &raw const addr,
                0,
                std::ptr::null(),
                &raw mut size,
                (&raw mut name_ref).cast::<c_void>(),
            )
        };
        if status != 0 || name_ref.is_null() {
            return None;
        }

        // Caller owns the returned CFString (AudioObject API contract).
        let cf = unsafe { CFString::wrap_under_create_rule(name_ref) };
        Some(cf.to_string())
    }
}

/// CpalAudioProcessor handles recording from audio devices using the CPAL library,
/// and saving the audio data to WAV files.
///
/// File I/O is performed on a dedicated writer thread. The cpal audio callback
/// pushes raw f32 samples into a lock-free SPSC ring buffer (via `rtrb`),
/// keeping the real-time thread free of blocking operations.
pub struct CpalAudioProcessor {
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
    /// Set by the cpal error callback when the audio stream encounters an error.
    stream_error: Arc<AtomicBool>,
    /// CoreAudio listener for sample rate changes (dropped before sample_rate_changed).
    #[cfg(target_os = "macos")]
    rate_listener: Option<sample_rate_listener::SampleRateListener>,
    /// Set by the CoreAudio listener when the device's sample rate changes mid-recording.
    sample_rate_changed: Arc<AtomicBool>,
    /// Per-channel peak levels (f32 as u32 bits). Shared with writer thread.
    peak_levels: Arc<Vec<CacheAlignedPeak>>,
    /// Handle to the writer thread (None before process_audio, None after finalize).
    writer_thread: Option<WriterThreadHandle>,
    /// Whether monitoring mode is active (levels without recording).
    monitoring: bool,
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
            stream_error: Arc::new(AtomicBool::new(false)),
            #[cfg(target_os = "macos")]
            rate_listener: None,
            sample_rate_changed: Arc::new(AtomicBool::new(false)),
            peak_levels: Arc::new(Vec::new()),
            writer_thread: None,
            monitoring: false,
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

    /// Get the input channel count for a named device.
    /// Returns the channel count from the device's default input config.
    pub fn get_device_channel_count(device_name: &str) -> Result<u16, BlackboxError> {
        let host = cpal::default_host();

        // Empty name means system default device
        let device = if device_name.is_empty() {
            host.default_input_device()
                .ok_or_else(|| BlackboxError::AudioDevice("No default input device".to_string()))?
        } else {
            let devices = host.input_devices().map_err(|e| {
                BlackboxError::AudioDevice(format!("Failed to enumerate devices: {e}"))
            })?;
            let mut found = None;
            for d in devices {
                if let Ok(desc) = d.description()
                    && desc.name() == device_name
                {
                    found = Some(d);
                    break;
                }
            }
            found.ok_or_else(|| {
                BlackboxError::AudioDevice(format!("Device '{device_name}' not found"))
            })?
        };

        device
            .default_input_config()
            .map(|cfg| cfg.channels())
            .map_err(|e| {
                BlackboxError::AudioDevice(format!("Failed to get config for '{device_name}': {e}"))
            })
    }
}

impl AudioProcessor for CpalAudioProcessor {
    fn process_audio(
        &mut self,
        channels: &[usize],
        output_mode: &str,
        debug: bool,
    ) -> Result<(), BlackboxError> {
        // Stop monitoring first if active, so recording seamlessly replaces it
        if self.monitoring {
            self.stop_monitoring()?;
        }

        self.channels = channels.to_vec();
        self.output_mode = output_mode.to_string();
        self.debug = debug;

        // Reset counters from any prior recording session
        self.write_errors.store(0, Ordering::Relaxed);
        self.disk_space_low.store(false, Ordering::Relaxed);
        self.stream_error.store(false, Ordering::Relaxed);
        self.sample_rate_changed.store(false, Ordering::Relaxed);

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
        let silence_threshold = app_config.get_silence_threshold();
        let min_disk_space_mb = app_config.get_min_disk_space_mb();
        let bits_per_sample = app_config.get_bits_per_sample();

        // Create per-channel peak levels for metering
        let peak_levels: Arc<Vec<CacheAlignedPeak>> = Arc::new(
            (0..actual_channels.len())
                .map(|_| CacheAlignedPeak::new(0))
                .collect(),
        );
        self.peak_levels = Arc::clone(&peak_levels);

        // Create writer thread state with initial WAV writers
        let mut state = WriterThreadState::new(
            &self.output_dir,
            sample_rate,
            &actual_channels,
            output_mode,
            silence_threshold,
            Arc::clone(&self.write_errors),
            min_disk_space_mb,
            Arc::clone(&self.disk_space_low),
            bits_per_sample,
            peak_levels,
        )?;
        state.total_device_channels = total_channels as u16;

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
            command_tx,
            join_handle: Some(join_handle),
        });

        // Clone write_errors for the callback
        let write_errors = Arc::clone(&self.write_errors);
        let continuous_mode = self.continuous_mode;
        let recording_cadence = self.recording_cadence;
        let rotation_needed_cb = Arc::clone(&rotation_needed);

        // Error callback — set atomic flag so Swift UI can detect device disconnects
        let stream_error = Arc::clone(&self.stream_error);
        let err_fn = move |err| {
            error!("an error occurred on stream: {}", err);
            stream_error.store(true, Ordering::Release);
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

                            // Push raw f32 to ring buffer (lock-free, wait-free, zero I/O).
                            // Try the full chunk first; on failure, write as much as fits
                            // so we only drop the true overflow instead of the entire callback.
                            if producer.write_chunk_uninit(data.len()).is_ok_and(|chunk| {
                                chunk.fill_from_iter(data.iter().copied());
                                true
                            }) {
                                // Wrote everything
                            } else {
                                let available = producer.slots();
                                if available > 0
                                    && let Ok(chunk) = producer.write_chunk_uninit(available)
                                {
                                    chunk.fill_from_iter(data[..available].iter().copied());
                                }
                                let dropped = data.len() - available;
                                if dropped > 0 {
                                    write_errors.fetch_add(dropped as u64, Ordering::Relaxed);
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

        // Register sample rate change listener (macOS only)
        #[cfg(target_os = "macos")]
        {
            self.rate_listener = sample_rate_listener::SampleRateListener::new(
                app_config.get_input_device().as_deref(),
                Arc::clone(&self.sample_rate_changed),
            );
        }

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

        // Remove sample rate listener before tearing down the stream
        #[cfg(target_os = "macos")]
        {
            self.rate_listener = None;
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
        !self.monitoring && (self.stream.is_some() || self.writer_thread.is_some())
    }

    fn write_error_count(&self) -> u64 {
        self.write_errors.load(Ordering::Relaxed)
    }

    fn disk_space_low(&self) -> bool {
        self.disk_space_low.load(Ordering::Relaxed)
    }

    fn stream_error(&self) -> bool {
        self.stream_error.load(Ordering::Relaxed)
    }

    fn sample_rate_changed(&self) -> bool {
        self.sample_rate_changed.load(Ordering::Relaxed)
    }

    fn peak_levels(&self) -> Vec<f32> {
        self.peak_levels
            .iter()
            .map(|a| f32::from_bits(a.value.load(Ordering::Relaxed)))
            .collect()
    }

    fn fill_peak_levels(&self, buf: &mut [f32]) -> usize {
        let count = self.peak_levels.len().min(buf.len());
        for (dst, src) in buf[..count].iter_mut().zip(self.peak_levels.iter()) {
            *dst = f32::from_bits(src.value.load(Ordering::Relaxed));
        }
        count
    }

    fn sample_rate(&self) -> u32 {
        self.sample_rate
    }

    fn start_monitoring(&mut self) -> Result<(), BlackboxError> {
        // If already monitoring, nothing to do
        if self.monitoring {
            return Ok(());
        }

        // Reset counters
        self.write_errors.store(0, Ordering::Relaxed);
        self.stream_error.store(false, Ordering::Relaxed);

        let host = cpal::default_host();
        let app_config = AppConfig::load();
        let device = Self::find_input_device(&host, app_config.get_input_device().as_deref())?;

        let config = device.default_input_config().map_err(|e| {
            BlackboxError::AudioDevice(format!("Failed to get default input stream config: {}", e))
        })?;

        let total_channels = config.channels() as usize;
        let sample_rate = config.sample_rate();

        // Determine which channels to monitor
        let channels_str = app_config.get_audio_channels();
        let requested_channels = parse_channel_string(&channels_str)?;
        let mut actual_channels: Vec<usize> = Vec::new();
        for &channel in &requested_channels {
            if channel < total_channels {
                actual_channels.push(channel);
            }
        }
        if actual_channels.is_empty() {
            actual_channels = (0..total_channels).collect();
        }

        info!(
            "Starting audio monitoring on channels: {:?}",
            actual_channels
        );

        // Create per-channel peak levels for metering
        let peak_levels: Arc<Vec<CacheAlignedPeak>> = Arc::new(
            (0..actual_channels.len())
                .map(|_| CacheAlignedPeak::new(0))
                .collect(),
        );
        self.peak_levels = Arc::clone(&peak_levels);

        // Create monitor-only writer thread state (no file I/O)
        let mut state = WriterThreadState::new_monitor(sample_rate, &actual_channels, peak_levels);
        state.total_device_channels = total_channels as u16;

        // Create ring buffer
        let ring_size = sample_rate as usize * total_channels * RING_BUFFER_SECONDS;
        let (mut producer, consumer) = rtrb::RingBuffer::new(ring_size);

        // Monitor mode doesn't need rotation, but writer_thread_main expects it
        let rotation_needed = Arc::new(AtomicBool::new(false));
        let (command_tx, command_rx) = std::sync::mpsc::sync_channel::<WriterCommand>(1);

        let rotation_needed_writer = Arc::clone(&rotation_needed);

        let join_handle = std::thread::Builder::new()
            .name("blackbox-monitor".to_string())
            .spawn(move || {
                writer_thread_main(consumer, rotation_needed_writer, command_rx, state);
            })
            .map_err(|e| {
                BlackboxError::AudioDevice(format!("Failed to spawn monitor thread: {}", e))
            })?;

        self.writer_thread = Some(WriterThreadHandle {
            command_tx,
            join_handle: Some(join_handle),
        });

        // Clone write_errors for the callback
        let write_errors = Arc::clone(&self.write_errors);

        let stream_error = Arc::clone(&self.stream_error);
        let err_fn = move |err| {
            error!("an error occurred on stream: {}", err);
            stream_error.store(true, Ordering::Release);
        };

        let stream = match config.sample_format() {
            SampleFormat::F32 => {
                device
                    .build_input_stream(
                        &config.into(),
                        move |data: &[f32], _: &_| {
                            if producer.write_chunk_uninit(data.len()).is_ok_and(|chunk| {
                                chunk.fill_from_iter(data.iter().copied());
                                true
                            }) {
                                // Wrote everything
                            } else {
                                let available = producer.slots();
                                if available > 0
                                    && let Ok(chunk) = producer.write_chunk_uninit(available)
                                {
                                    chunk.fill_from_iter(data[..available].iter().copied());
                                }
                                let dropped = data.len() - available;
                                if dropped > 0 {
                                    write_errors.fetch_add(dropped as u64, Ordering::Relaxed);
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

        stream
            .play()
            .map_err(|e| BlackboxError::AudioDevice(format!("Failed to play stream: {}", e)))?;

        self.stream = Some(Box::new(stream));
        self.monitoring = true;

        Ok(())
    }

    fn stop_monitoring(&mut self) -> Result<(), BlackboxError> {
        if !self.monitoring {
            return Ok(());
        }

        info!("Stopping audio monitoring");

        // Drop stream first
        self.stream = None;

        // Shut down writer thread
        if let Some(mut handle) = self.writer_thread.take() {
            let (reply_tx, reply_rx) = std::sync::mpsc::channel();
            if handle
                .command_tx
                .send(WriterCommand::Shutdown(reply_tx))
                .is_ok()
            {
                // Monitor mode finalize is a no-op (no files), but wait for thread
                if let Ok(_result) = reply_rx.recv_timeout(Duration::from_secs(5))
                    && let Some(jh) = handle.join_handle.take()
                {
                    let _ = jh.join();
                }
            }
        }

        self.monitoring = false;
        self.peak_levels = Arc::new(Vec::new());

        Ok(())
    }

    fn is_monitoring(&self) -> bool {
        self.monitoring
    }
}

impl Drop for CpalAudioProcessor {
    fn drop(&mut self) {
        if self.monitoring {
            if let Err(e) = self.stop_monitoring() {
                error!("Error stopping monitoring during cleanup: {}", e);
            }
        } else if self.is_recording()
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
        Self::new_for_test_with_bits(output_dir, sample_rate, channels, output_mode, 16)
    }

    /// Like `new_for_test` but with configurable bit depth.
    pub fn new_for_test_with_bits(
        output_dir: &str,
        sample_rate: u32,
        channels: &[usize],
        output_mode: &str,
        bits_per_sample: u16,
    ) -> Result<Self, BlackboxError> {
        if !Path::new(output_dir).exists() {
            fs::create_dir_all(output_dir)?;
        }

        let write_errors = Arc::new(AtomicU64::new(0));

        let disk_space_low = Arc::new(AtomicBool::new(false));

        let peak_levels: Arc<Vec<CacheAlignedPeak>> = Arc::new(
            (0..channels.len())
                .map(|_| CacheAlignedPeak::new(0))
                .collect(),
        );

        let mut state = WriterThreadState::new(
            output_dir,
            sample_rate,
            channels,
            output_mode,
            AppConfig::load().get_silence_threshold(),
            Arc::clone(&write_errors),
            0, // disable disk check in tests
            Arc::clone(&disk_space_low),
            bits_per_sample,
            Arc::clone(&peak_levels),
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
            stream_error: Arc::new(AtomicBool::new(false)),
            #[cfg(target_os = "macos")]
            rate_listener: None,
            sample_rate_changed: Arc::new(AtomicBool::new(false)),
            peak_levels,
            writer_thread: None,
            monitoring: false,
            direct_state: Some(state),
        })
    }

    /// Feed interleaved f32 audio data as if it came from a cpal callback.
    pub fn feed_test_data(&mut self, data: &[f32], total_device_channels: usize) {
        if let Some(ref mut state) = self.direct_state {
            state.total_device_channels = total_device_channels as u16;
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
