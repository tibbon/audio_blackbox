use std::fs;
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::time::Duration;

use log::{debug, error, info, warn};

use crate::audio_processor::AudioProcessor;
use crate::config::AppConfig;
use crate::constants::{CacheAlignedPeak, OutputMode, RING_BUFFER_SECONDS};
use crate::error::BlackboxError;
use crate::utils::{check_alsa_availability, parse_channel_string};
use crate::writer_thread::{
    WriterCommand, WriterThreadHandle, WriterThreadState, writer_thread_main,
};

use cpal::SampleFormat;
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};

/// Bundle of `Arc<Atomic*>` status flags shared between `CpalAudioProcessor` and
/// the FFI status-poll path.
///
/// Cloning the bundle clones `Arc`s only — readers can lock a containing `Mutex`
/// briefly to obtain a clone, drop the lock, and then perform lock-free atomic
/// loads. This lets the FFI status query stay lock-free with respect to the
/// multi-second device probe that runs under the recorder mutex.
///
/// Crate-private and gated behind the `ffi` feature: only `ffi.rs` consumes
/// this bundle, so leaving it on the public API would lock in the exact
/// 8-field shape as a SemVer contract (DOLL-120). Adding a 9th flag stays
/// a no-op for downstream consumers.
#[cfg(feature = "ffi")]
#[derive(Clone, Default)]
pub(crate) struct ProcessorStatus {
    /// True between the end of `process_audio_impl` and the start of `finalize`.
    pub(crate) recording_active: Arc<AtomicBool>,
    /// True between the end of `start_monitoring` and the start of `stop_monitoring`.
    pub(crate) monitoring_active: Arc<AtomicBool>,
    /// Mirrors the active stream's sample rate; 0 when idle.
    pub(crate) sample_rate: Arc<AtomicU32>,
    pub(crate) write_errors: Arc<AtomicU64>,
    pub(crate) disk_space_low: Arc<AtomicBool>,
    pub(crate) write_failed: Arc<AtomicBool>,
    pub(crate) stream_error: Arc<AtomicBool>,
    pub(crate) sample_rate_changed: Arc<AtomicBool>,
    pub(crate) gate_idle: Arc<AtomicBool>,
}

#[cfg(feature = "ffi")]
impl ProcessorStatus {
    /// Construct an idle bundle (all flags false, counters zero).
    /// Thin alias for `Default::default()` — kept so the call sites in
    /// `ffi.rs` read intentionally as "reset to idle" rather than the
    /// generic "default-init."
    pub(crate) fn idle() -> Self {
        Self::default()
    }
}

/// `CpalAudioProcessor` handles recording from audio devices using the CPAL library,
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
    output_mode: OutputMode,
    debug: bool,
    /// Counts `write_sample` errors and ring buffer overflow drops (atomic for RT safety).
    write_errors: Arc<AtomicU64>,
    /// Set by the writer thread when disk space drops below threshold.
    disk_space_low: Arc<AtomicBool>,
    /// Set by the writer thread when `write_sample` keeps failing (disk full or
    /// the output directory became unwritable), distinct from a low-space
    /// pre-check — drives a precise "unable to write to disk" UI message (DOLL-437).
    write_failed: Arc<AtomicBool>,
    /// Set by the cpal error callback when the audio stream encounters an error.
    stream_error: Arc<AtomicBool>,
    /// CoreAudio listener for sample rate changes (dropped before `sample_rate_changed`).
    #[cfg(target_os = "macos")]
    rate_listener: Option<crate::macos_sample_rate_listener::SampleRateListener>,
    /// Set by the CoreAudio listener when the device's sample rate changes mid-recording.
    sample_rate_changed: Arc<AtomicBool>,
    /// Per-channel peak levels (f32 as u32 bits). Shared with writer thread.
    peak_levels: Arc<[CacheAlignedPeak]>,
    /// Shared flag: true when silence gate is idle (no files open).
    gate_idle: Arc<AtomicBool>,
    /// Mirrors `is_recording()` so external readers can check recording state via a
    /// single atomic load instead of holding the recorder mutex.
    recording_active: Arc<AtomicBool>,
    /// Mirrors `is_monitoring()`. Same rationale as `recording_active`.
    monitoring_active: Arc<AtomicBool>,
    /// Mirrors `sample_rate` for the same reason; 0 when idle.
    sample_rate_atomic: Arc<AtomicU32>,
    /// Handle to the writer thread (None when idle; set by `process_audio` or `start_monitoring`, cleared by finalize or `stop_monitoring`).
    writer_thread: Option<WriterThreadHandle>,
    /// Whether monitoring mode is active (levels without recording).
    monitoring: bool,
    /// Test-only: bypass ring buffer and writer thread, write directly.
    #[cfg(test)]
    direct_state: Option<WriterThreadState>,
}

/// Push f32 samples into the ring buffer and atomically count any rejected
/// suffix in `write_errors`. Used by the cpal audio callback (real-time)
/// and by tests that need to verify the overflow-counting contract — both
/// call this single helper so the test can't drift from production.
///
/// Only whole frames of `frame_size` interleaved samples are accepted. The
/// writer frees slots in reads of up to `WRITER_THREAD_READ_CHUNK` samples,
/// which need not be a multiple of the channel count, so on overflow the
/// free space can end mid-frame. Pushing that partial frame and dropping the
/// rest used to shift every later sample by the missing channels: split files
/// got another channel's audio for the rest of the session, and the writer's
/// `frame_remainder` carried the shift across rotations. Rounding the accepted
/// length down to a frame boundary keeps the stream aligned; the whole
/// rejected tail (including the partial frame) is counted.
///
/// RT-safe: one atomic load (`slots`), one memcpy, and at most one atomic add.
pub(crate) fn push_samples_with_overflow_count(
    producer: &mut rtrb::Producer<f32>,
    data: &[f32],
    frame_size: usize,
    write_errors: &AtomicU64,
) {
    let slots = producer.slots();
    let accepted = if data.len() <= slots {
        data.len()
    } else {
        slots - slots % frame_size.max(1)
    };
    // push_partial_slice uses memcpy internally for Copy types. `accepted`
    // fits by construction (this is the only producer), so `unpushed` is
    // empty; it is still counted rather than assumed.
    let (head, _) = data.split_at(accepted);
    let (_, unpushed) = producer.push_partial_slice(head);
    let rejected = data.len() - accepted + unpushed.len();
    if rejected > 0 {
        write_errors.fetch_add(rejected as u64, Ordering::Relaxed);
    }
}

/// Rotation threshold in interleaved samples for one cadence period:
/// `sample_rate * total_channels * recording_cadence_secs`. The channel factor
/// matters — the RT callback counts interleaved samples across ALL device
/// channels, so a 4-channel device sees samples 4× as fast as mono and the
/// threshold must scale with it to keep rotation on the same wall clock.
/// Pure so the arithmetic is unit-testable without a device (DOLL-453).
#[inline]
fn rotation_threshold_samples(
    sample_rate: u32,
    total_channels: usize,
    recording_cadence_secs: u64,
) -> u64 {
    u64::from(sample_rate) * total_channels as u64 * recording_cadence_secs
}

/// Advance the rotation counter by one callback batch of `batch_len`
/// interleaved samples. Returns true when the counter crossed `threshold`
/// (a rotation is due). Alloc/lock-free — called from the real-time cpal
/// callback.
///
/// The overshoot past `threshold` is carried into the next period rather
/// than dropped. Callbacks rarely land exactly on the threshold, so resetting
/// to 0 made every period up to one callback longer than the cadence and file
/// boundaries crept later over a long session (a 512-frame callback at
/// 48 kHz adds up to ~10 ms per rotation). Carrying keeps rotation `n` within
/// one callback of `n * cadence`. The remainder is taken modulo `threshold`,
/// so a callback longer than a whole period flags one rotation instead of
/// queueing catch-up rotations; a zero threshold leaves the counter at 0.
#[inline]
fn advance_rotation_counter(counter: &mut u64, batch_len: usize, threshold: u64) -> bool {
    *counter = counter.saturating_add(batch_len as u64);
    if *counter >= threshold {
        *counter = counter.checked_rem(threshold).unwrap_or(0);
        true
    } else {
        false
    }
}

impl std::fmt::Debug for CpalAudioProcessor {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // `stream` is a trait object without Debug; report presence only.
        f.debug_struct("CpalAudioProcessor")
            .field("sample_rate", &self.sample_rate)
            .field("stream_open", &self.stream.is_some())
            .field("continuous_mode", &self.continuous_mode)
            .field("recording_cadence", &self.recording_cadence)
            .field("output_dir", &self.output_dir)
            .field("channels", &self.channels)
            .field("output_mode", &self.output_mode)
            .field("debug", &self.debug)
            .finish_non_exhaustive()
    }
}

impl CpalAudioProcessor {
    /// Create a new `CpalAudioProcessor` instance, loading config from env/TOML.
    ///
    /// Probes the audio device for sample rate and stores config.
    /// WAV writers are not created until `process_audio()` is called.
    ///
    /// # Errors
    ///
    /// Fails like [`with_config`](Self::with_config).
    pub fn new() -> Result<Self, BlackboxError> {
        Self::with_config(&AppConfig::load())
    }

    /// Create a new `CpalAudioProcessor` using the provided configuration.
    ///
    /// Defers device probing to `process_audio()` / `start_monitoring()` to
    /// avoid enumerating the audio device twice on recording start.
    ///
    /// # Errors
    ///
    /// Returns [`BlackboxError::Io`] if the output directory doesn't exist and
    /// can't be created.
    pub fn with_config(config: &AppConfig) -> Result<Self, BlackboxError> {
        check_alsa_availability()?;

        let output_dir = config.get_output_dir();
        let continuous_mode = config.get_continuous_mode();
        let recording_cadence = config.get_recording_cadence();

        if !Path::new(&output_dir).exists() {
            fs::create_dir_all(&output_dir)?;
        }

        Ok(Self {
            sample_rate: 0, // Set when recording/monitoring starts
            stream: None,
            continuous_mode,
            recording_cadence,
            output_dir,
            channels: Vec::new(),
            output_mode: OutputMode::default(),
            debug: false,
            write_errors: Arc::new(AtomicU64::new(0)),
            disk_space_low: Arc::new(AtomicBool::new(false)),
            write_failed: Arc::new(AtomicBool::new(false)),
            stream_error: Arc::new(AtomicBool::new(false)),
            #[cfg(target_os = "macos")]
            rate_listener: None,
            sample_rate_changed: Arc::new(AtomicBool::new(false)),
            peak_levels: Arc::from(Vec::new()),
            gate_idle: Arc::new(AtomicBool::new(false)),
            recording_active: Arc::new(AtomicBool::new(false)),
            monitoring_active: Arc::new(AtomicBool::new(false)),
            sample_rate_atomic: Arc::new(AtomicU32::new(0)),
            writer_thread: None,
            monitoring: false,
            #[cfg(test)]
            direct_state: None,
        })
    }

    /// Return a clone of the `Arc` holding per-channel peak levels.
    ///
    /// Used by the FFI layer to read peaks without locking the recorder mutex.
    #[cfg(feature = "ffi")]
    pub(crate) fn peak_levels_arc(&self) -> Arc<[CacheAlignedPeak]> {
        Arc::clone(&self.peak_levels)
    }

    /// Build the cpal `err_fn` callback used when constructing the input
    /// stream. Extracted as a method so the SAME closure the production
    /// stream uses can be exercised by tests — reverting the body here
    /// breaks both production wiring AND the test (DOLL-106).
    pub(crate) fn build_stream_err_callback(&self) -> impl FnMut(cpal::Error) + Send + 'static {
        let stream_error = Arc::clone(&self.stream_error);
        move |err| {
            error!("an error occurred on stream: {err}");
            // status flag only; reader at stream_error() loads Relaxed.
            stream_error.store(true, Ordering::Relaxed);
        }
    }

    /// Test-only: simulate the macOS CoreAudio sample-rate-changed
    /// listener firing. The real callback at
    /// `macos_sample_rate_listener::on_rate_changed` does this exact
    /// store. Tests use it to exercise
    /// the propagation path without needing a real audio device whose
    /// rate can be changed mid-test (DOLL-123 verification).
    #[cfg(test)]
    pub fn simulate_sample_rate_changed(&self) {
        self.sample_rate_changed.store(true, Ordering::Relaxed);
    }

    /// Return a clone-able bundle of the processor's status atomics.
    ///
    /// Cloning is cheap (Arc clones); the FFI layer caches the result so the
    /// status-poll path can read flags without taking the recorder mutex.
    /// Note that `gate_idle` and `peak_levels` are re-allocated on every
    /// recording start, so callers must re-fetch this bundle after each
    /// start to avoid reading a stale gate from the previous session.
    #[cfg(feature = "ffi")]
    pub(crate) fn status_arcs(&self) -> ProcessorStatus {
        ProcessorStatus {
            recording_active: Arc::clone(&self.recording_active),
            monitoring_active: Arc::clone(&self.monitoring_active),
            sample_rate: Arc::clone(&self.sample_rate_atomic),
            write_errors: Arc::clone(&self.write_errors),
            disk_space_low: Arc::clone(&self.disk_space_low),
            write_failed: Arc::clone(&self.write_failed),
            stream_error: Arc::clone(&self.stream_error),
            sample_rate_changed: Arc::clone(&self.sample_rate_changed),
            gate_idle: Arc::clone(&self.gate_idle),
        }
    }

    /// Find an input device by name, or return the default input device.
    fn find_input_device(
        host: &cpal::Host,
        device_name: Option<&str>,
    ) -> Result<cpal::Device, BlackboxError> {
        if let Some(name) = device_name {
            let devices = host
                .input_devices()
                .map_err(|e| BlackboxError::AudioDeviceSource {
                    context: "Failed to enumerate input devices".to_owned(),
                    source: Box::new(e),
                })?;
            for device in devices {
                if let Ok(desc) = device.description()
                    && desc.name() == name
                {
                    return Ok(device);
                }
            }
            warn!("Input device '{name}' not found, falling back to default");
        }
        host.default_input_device()
            .ok_or_else(|| BlackboxError::AudioDevice("No input device available".to_owned()))
    }

    /// Return the name of the system default input device.
    ///
    /// # Returns
    ///
    /// - `Some(name)` — CoreAudio reports a default input device and its
    ///   name can be read.
    /// - `None` — either no internal input device is available (rare on a
    ///   laptop, common on a headless Mac mini) or the device's
    ///   `description()` query fails (DOLL-234).
    ///
    /// `pub(crate)` because the only consumer is the C FFI wrapper in
    /// `ffi.rs`; tightening visibility avoids committing to a stable
    /// Rust-level API that no Rust caller actually needs (DOLL-232).
    /// Gated on `feature = "ffi"` to match the caller — without the
    /// gate, `--no-default-features` clippy correctly flags it unused.
    /// Surfaced over FFI for DOLL-215 so the menu / Settings can show
    /// "System Default (MacBook Pro Microphone)" instead of an opaque
    /// literal.
    #[cfg(feature = "ffi")]
    pub(crate) fn default_input_device_name() -> Option<String> {
        let host = cpal::default_host();
        let device = host.default_input_device()?;
        device.description().ok().map(|desc| desc.name().to_owned())
    }

    /// List all available input device names.
    ///
    /// # Errors
    ///
    /// Returns [`BlackboxError::AudioDeviceSource`] if the audio host can't
    /// enumerate input devices. Devices whose description can't be read are
    /// skipped rather than reported.
    pub fn list_input_devices() -> Result<Vec<String>, BlackboxError> {
        let host = cpal::default_host();
        let devices = host
            .input_devices()
            .map_err(|e| BlackboxError::AudioDeviceSource {
                context: "Failed to enumerate input devices".to_owned(),
                source: Box::new(e),
            })?;
        let mut names = Vec::new();
        for device in devices {
            if let Ok(desc) = device.description() {
                names.push(desc.name().to_owned());
            }
        }
        Ok(names)
    }

    /// Get the input channel count for a named device.
    /// Returns the channel count from the device's default input config.
    ///
    /// # Errors
    ///
    /// Returns [`BlackboxError::AudioDevice`] if there is no default input device
    /// or no device named `device_name`, and [`BlackboxError::AudioDeviceSource`]
    /// if devices can't be enumerated or the device's default input config can't
    /// be read.
    pub fn get_device_channel_count(device_name: &str) -> Result<u16, BlackboxError> {
        let host = cpal::default_host();

        // Empty name means system default device
        let device = if device_name.is_empty() {
            host.default_input_device()
                .ok_or_else(|| BlackboxError::AudioDevice("No default input device".to_owned()))?
        } else {
            let devices = host
                .input_devices()
                .map_err(|e| BlackboxError::AudioDeviceSource {
                    context: "Failed to enumerate devices".to_owned(),
                    source: Box::new(e),
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
            .map_err(|e| BlackboxError::AudioDeviceSource {
                context: format!("Failed to get config for '{device_name}'"),
                source: Box::new(e),
            })
    }

    /// Read `device`'s current default config and publish its sample rate to
    /// `sample_rate` and the lock-free mirror.
    ///
    /// Uses the device's current default config (sample rate, channels,
    /// format). This avoids changing `kAudioDevicePropertyNominalSampleRate`
    /// on macOS, which would conflict with DAWs and other pro audio apps
    /// sharing the device.
    fn load_input_config(
        &mut self,
        device: &cpal::Device,
    ) -> Result<cpal::SupportedStreamConfig, BlackboxError> {
        let config = default_input_config(device)?;
        self.sample_rate = config.sample_rate();
        self.sample_rate_atomic
            .store(self.sample_rate, Ordering::Relaxed);
        Ok(config)
    }

    fn process_audio_impl(
        &mut self,
        channels: &[usize],
        output_mode: OutputMode,
        debug: bool,
        app_config: &AppConfig,
    ) -> Result<(), BlackboxError> {
        if self.monitoring {
            self.stop_monitoring()?;
        }

        self.channels = channels.to_vec();
        self.output_mode = output_mode;
        self.debug = debug;

        // Reset counters from any prior recording session
        self.write_errors.store(0, Ordering::Relaxed);
        self.disk_space_low.store(false, Ordering::Relaxed);
        self.write_failed.store(false, Ordering::Relaxed);
        self.stream_error.store(false, Ordering::Relaxed);
        self.sample_rate_changed.store(false, Ordering::Relaxed);

        let host = cpal::default_host();
        let device = Self::find_input_device(&host, app_config.get_input_device().as_deref())?;

        info!(
            "Using audio device: {}",
            device
                .description()
                .map_or_else(|_| "unknown".to_owned(), |d| d.name().to_owned())
        );

        let config = self.load_input_config(&device)?;
        debug!("Default input stream config: {config:?}");

        // Keep cpal's u16 for the writer state; index math uses usize.
        let device_channels = config.channels();
        let total_channels = usize::from(device_channels);
        let sample_rate = config.sample_rate();

        let actual_channels = recording_channels(channels, total_channels);
        info!("Using channels: {actual_channels:?}");

        // Create per-channel peak levels for metering
        let peak_levels = new_peak_levels(actual_channels.len());
        self.peak_levels = Arc::clone(&peak_levels);

        // Create writer thread state with initial WAV writers
        let mut state = WriterThreadState::new(
            &self.output_dir,
            sample_rate,
            &actual_channels,
            output_mode,
            app_config.get_silence_threshold(),
            Arc::clone(&self.write_errors),
            app_config.get_min_disk_space_mb(),
            Arc::clone(&self.disk_space_low),
            app_config.get_bits_per_sample(),
            peak_levels,
            app_config.get_silence_gate_enabled(),
            app_config.get_silence_gate_timeout_secs(),
        )?;
        self.gate_idle = Arc::clone(&state.gate_idle);
        state.total_device_channels = device_channels;
        // DOLL-437: wire the shared write-failure flag into the writer state so
        // a persistent-write-failure stop is visible to the FFI status poll.
        state.write_failed = Arc::clone(&self.write_failed);

        let ring_size = sample_rate as usize * total_channels * RING_BUFFER_SECONDS;
        let pipeline = spawn_writer_pipeline("blackbox-writer", "writer", ring_size, state)?;
        // Store handle (producer goes to the callback, not into the handle)
        self.writer_thread = Some(pipeline.handle);

        // Error callback — set atomic flag so Swift UI can detect device
        // disconnects. Built via a method on `self` so the same closure
        // can be exercised by tests; reverting the body of
        // `build_stream_err_callback` now fails both production wiring
        // and the propagation test (DOLL-106).
        let err_fn = self.build_stream_err_callback();
        let callback = recording_callback(
            pipeline.producer,
            Arc::clone(&self.write_errors),
            pipeline.rotation_needed,
            self.continuous_mode,
            rotation_threshold_samples(sample_rate, total_channels, self.recording_cadence),
            total_channels,
        );
        let stream = start_f32_input_stream(&device, config, callback, err_fn)?;
        self.stream = Some(Box::new(stream));

        // Register sample rate change listener (macOS only)
        #[cfg(target_os = "macos")]
        {
            self.rate_listener = crate::macos_sample_rate_listener::SampleRateListener::new(
                app_config.get_input_device().as_deref(),
                Arc::clone(&self.sample_rate_changed),
            );
        }

        // Publish the live state to lock-free external readers via a
        // Release store. Readers (FFI status poll) Acquire on the matching
        // load; this synchronizes-with `sample_rate_atomic.store(rate, Relaxed)`
        // above, so a reader observing `recording_active = true` is
        // guaranteed to also see the matching `sample_rate` (DOLL-101).
        self.recording_active.store(true, Ordering::Release);

        Ok(())
    }
}

/// The device's current default input config.
fn default_input_config(
    device: &cpal::Device,
) -> Result<cpal::SupportedStreamConfig, BlackboxError> {
    device
        .default_input_config()
        .map_err(|e| BlackboxError::AudioDeviceSource {
            context: "Failed to get default input stream config".to_owned(),
            source: Box::new(e),
        })
}

/// The channels to record: the requested ones the device has, or all of its
/// channels when it has none of them. Logs each channel that is dropped.
fn recording_channels(requested: &[usize], total_channels: usize) -> Vec<usize> {
    for &channel in requested.iter().filter(|&&ch| ch >= total_channels) {
        warn!(
            "Channel {channel} not available on device. Device only has {total_channels} channels."
        );
    }
    available_channels(requested, total_channels).unwrap_or_else(|| {
        warn!(
            "No requested channels available. Using all available channels (0 to {}).",
            total_channels - 1
        );
        (0..total_channels).collect()
    })
}

/// The requested channels that exist on a device with `total_channels`
/// inputs, in order, or `None` when none of them do.
fn available_channels(requested: &[usize], total_channels: usize) -> Option<Vec<usize>> {
    let kept: Vec<usize> = requested
        .iter()
        .copied()
        .filter(|&ch| ch < total_channels)
        .collect();
    (!kept.is_empty()).then_some(kept)
}

/// One zeroed peak-level slot per recorded channel, shared with the FFI meter.
fn new_peak_levels(count: usize) -> Arc<[CacheAlignedPeak]> {
    std::iter::repeat_with(|| CacheAlignedPeak::new(0))
        .take(count)
        .collect()
}

/// The capture side of a running writer thread.
struct WriterPipeline {
    /// Pushes captured samples into the ring the thread drains.
    producer: rtrb::Producer<f32>,
    /// Set by the capture callback when a rotation is due.
    rotation_needed: Arc<AtomicBool>,
    /// Shuts the thread down.
    handle: WriterThreadHandle,
}

/// Create the ring buffer, rotation flag and command channel, and spawn the
/// thread that drains the ring into `state`. The thread runs at the
/// user-interactive quality-of-service class to avoid ring buffer overflow.
fn spawn_writer_pipeline(
    thread_name: &str,
    what: &str,
    ring_size: usize,
    state: WriterThreadState,
) -> Result<WriterPipeline, BlackboxError> {
    let (producer, consumer) = rtrb::RingBuffer::new(ring_size);
    let rotation_needed = Arc::new(AtomicBool::new(false));
    let (command_tx, command_rx) = std::sync::mpsc::sync_channel::<WriterCommand>(1);
    let rotation_needed_writer = Arc::clone(&rotation_needed);

    let join_handle = std::thread::Builder::new()
        .name(thread_name.to_owned())
        .spawn(move || {
            #[cfg(target_os = "macos")]
            // SAFETY: macOS-only libc call. No pointer args; affects
            // only the current thread's QoS attribute. The passed
            // `qos_class_t` is a valid enum variant; a different
            // (invalid) value could in principle be unsound, but
            // every call site here passes a known-good constant.
            unsafe {
                libc::pthread_set_qos_class_self_np(
                    libc::qos_class_t::QOS_CLASS_USER_INTERACTIVE,
                    0,
                );
            }
            writer_thread_main(consumer, &rotation_needed_writer, &command_rx, state);
        })
        .map_err(|e| BlackboxError::AudioDeviceSource {
            context: format!("Failed to spawn {what} thread"),
            source: Box::new(e),
        })?;

    Ok(WriterPipeline {
        producer,
        rotation_needed,
        handle: WriterThreadHandle {
            command_tx,
            join_handle: Some(join_handle),
        },
    })
}

/// The recording stream's data callback: flag a rotation once a cadence
/// period of samples has arrived, then push the batch into the ring.
fn recording_callback(
    mut producer: rtrb::Producer<f32>,
    write_errors: Arc<AtomicU64>,
    rotation_needed: Arc<AtomicBool>,
    continuous_mode: bool,
    rotation_threshold: u64,
    frame_size: usize,
) -> impl FnMut(&[f32], &cpal::InputCallbackInfo) + Send + 'static {
    // Sample counter for rotation (avoids Instant::now() syscall in RT callback)
    let mut rotation_sample_counter: u64 = 0;
    move |data: &[f32], _: &_| {
        // No logging on the RT capture thread (DOLL-250):
        // the `log` facade takes a lock and may do I/O, which
        // is a real-time-safety violation that causes audio
        // dropouts. Sample-count signals belong on the writer
        // thread (see `write_errors` atomic).

        // Check rotation via sample counter (zero syscalls)
        if continuous_mode
            && advance_rotation_counter(
                &mut rotation_sample_counter,
                data.len(),
                rotation_threshold,
            )
        {
            // Status flag only — the flag carries no
            // companion payload (samples travel through
            // rtrb with its own synchronization), so
            // Relaxed suffices and is marginally cheaper
            // on the RT thread (DOLL-391). Matches the
            // other RT status flags in this file.
            rotation_needed.store(true, Ordering::Relaxed);
        }

        push_samples_with_overflow_count(&mut producer, data, frame_size, &write_errors);
    }
}

/// Build an f32 input stream on `device` and start it.
///
/// cpal adds sample formats between releases; anything other than f32 is
/// rejected by name before a stream is built.
fn start_f32_input_stream<D, E>(
    device: &cpal::Device,
    config: cpal::SupportedStreamConfig,
    data_callback: D,
    err_fn: E,
) -> Result<cpal::Stream, BlackboxError>
where
    D: FnMut(&[f32], &cpal::InputCallbackInfo) + Send + 'static,
    E: FnMut(cpal::Error) + Send + 'static,
{
    if config.sample_format() != SampleFormat::F32 {
        return Err(BlackboxError::AudioDevice(format!(
            "Unsupported sample format: {:?}",
            config.sample_format()
        )));
    }

    let stream = device
        .build_input_stream(config.into(), data_callback, err_fn, None)
        .map_err(|e| BlackboxError::AudioDeviceSource {
            context: "Failed to build input stream".to_owned(),
            source: Box::new(e),
        })?;

    stream
        .play()
        .map_err(|e| BlackboxError::AudioDeviceSource {
            context: "Failed to play stream".to_owned(),
            source: Box::new(e),
        })?;

    Ok(stream)
}

impl AudioProcessor for CpalAudioProcessor {
    fn process_audio(
        &mut self,
        channels: &[usize],
        output_mode: OutputMode,
        debug: bool,
        config: &AppConfig,
    ) -> Result<(), BlackboxError> {
        self.process_audio_impl(channels, output_mode, debug, config)
    }

    fn finalize(&mut self) -> Result<(), BlackboxError> {
        // Mirror state for lock-free readers before we begin teardown.
        // Order matters: clear sample_rate_atomic Relaxed first, then
        // Release-store `recording_active = false`. Readers who
        // Acquire-load `recording_active = false` then observe
        // sample_rate = 0 — matches the symmetry of stop_monitoring and
        // the start-side ordering (DOLL-101).
        self.sample_rate_atomic.store(0, Ordering::Relaxed);
        // Also clear `sample_rate_changed` (DOLL-123): without this, a
        // rate-change flag set during a session survived stop and was
        // observed `true` by the next FFI poll between sessions.
        // process_audio_impl already resets it on the start side; clearing
        // here makes start/stop symmetric.
        self.sample_rate_changed.store(false, Ordering::Relaxed);
        self.recording_active.store(false, Ordering::Release);

        let errors = self.write_errors.load(Ordering::Relaxed);
        if errors > 0 {
            warn!("{errors} sample write/overflow errors occurred during recording");
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
            //
            // DOLL-352: the detach is a deliberate tradeoff with two costs.
            // (1) The detached thread still owns its `WriterThreadState` — so
            //     if the thread is wedged (e.g. slow/hung disk I/O in
            //     `finalize_all`/`drain_remaining`), it lingers with its open
            //     files, and a new writer thread may be spawned on the next
            //     `process_audio_impl`. Under repeated wedged-shutdown cycles
            //     this is unbounded thread growth.
            //
            // The join below is prompt once the thread has replied: dropping
            // its state no longer waits for queued silence scans (the silence
            // worker finishes them in the background).
            // (2) The 30 s `recv_timeout` above stalls the user-visible stop.
            // Both are accepted because hanging the app on quit is worse, and a
            // wedged writer is expected only on pathological disk failures. If
            // this becomes a real problem, bound the finalize/rename disk work
            // (so the thread can't wedge indefinitely) or refuse to spawn a new
            // writer while a prior detached one is still in flight.
            if got_reply {
                if let Some(jh) = handle.join_handle.take()
                    && jh.join().is_err()
                {
                    warn!("writer thread panicked before join");
                }
            } else {
                warn!(
                    "Writer thread did not respond within 30s — detaching to avoid hang \
                     (see DOLL-352)"
                );
            }
        }

        #[cfg(test)]
        if let Some(mut state) = self.direct_state.take() {
            return state.finalize_all();
        }

        // sample_rate_atomic was already cleared at the top of finalize
        // alongside the Release store on recording_active.
        Ok(())
    }

    fn start_recording(&mut self, config: &AppConfig) -> Result<(), BlackboxError> {
        let channels_str = config.get_audio_channels();
        let channels = parse_channel_string(&channels_str)?;
        let output_mode = config.output_mode_parsed();
        let debug = config.get_debug();

        self.process_audio_impl(&channels, output_mode, debug, config)
    }

    fn stop_recording(&mut self) -> Result<(), BlackboxError> {
        self.finalize()
    }

    fn is_recording(&self) -> bool {
        // Reads the lifted atomic mirror; the FFI status poll uses the same
        // flag via `status_arcs()` without needing the recorder mutex.
        // Acquire to synchronize-with the matching Release store; readers
        // who see `true` here also see the prior `sample_rate_atomic` write
        // (DOLL-101).
        self.recording_active.load(Ordering::Acquire)
    }

    fn write_error_count(&self) -> u64 {
        self.write_errors.load(Ordering::Relaxed)
    }

    fn disk_space_low(&self) -> bool {
        self.disk_space_low.load(Ordering::Relaxed)
    }

    fn write_failed(&self) -> bool {
        self.write_failed.load(Ordering::Relaxed)
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
            .map(CacheAlignedPeak::take)
            .collect()
    }

    fn fill_peak_levels(&self, buf: &mut [f32]) -> usize {
        let count = self.peak_levels.len().min(buf.len());
        for (dst, src) in buf[..count].iter_mut().zip(self.peak_levels.iter()) {
            *dst = src.take();
        }
        count
    }

    fn sample_rate(&self) -> u32 {
        self.sample_rate_atomic.load(Ordering::Relaxed)
    }

    fn start_monitoring(&mut self, config: &AppConfig) -> Result<(), BlackboxError> {
        // Recording and monitoring are mutually exclusive — each opens an
        // exclusive input stream on the device. If a recording is active,
        // do nothing rather than overwrite the writer-thread handle and
        // orphan the in-flight `.recording.wav` (DOLL-249). The recording
        // path already publishes per-channel peak levels, so the meter keeps
        // working without a separate monitor stream.
        if self.is_recording() {
            return Ok(());
        }

        // If already monitoring, nothing to do
        if self.monitoring {
            return Ok(());
        }

        // Reset counters
        self.write_errors.store(0, Ordering::Relaxed);
        self.stream_error.store(false, Ordering::Relaxed);

        let host = cpal::default_host();
        let device = Self::find_input_device(&host, config.get_input_device().as_deref())?;
        let stream_config = self.load_input_config(&device)?;

        let device_channels = stream_config.channels();
        let total_channels = usize::from(device_channels);
        let sample_rate = stream_config.sample_rate();

        // Determine which channels to monitor
        let requested_channels = parse_channel_string(&config.get_audio_channels())?;
        let actual_channels = available_channels(&requested_channels, total_channels)
            .unwrap_or_else(|| (0..total_channels).collect());

        info!("Starting audio monitoring on channels: {actual_channels:?}");

        // Create per-channel peak levels for metering
        let peak_levels = new_peak_levels(actual_channels.len());
        self.peak_levels = Arc::clone(&peak_levels);

        // Create monitor-only writer thread state (no file I/O)
        let mut state = WriterThreadState::new_monitor(sample_rate, &actual_channels, peak_levels);
        state.total_device_channels = device_channels;

        // Monitor mode doesn't need rotation, but writer_thread_main expects the flag.
        let ring_size = sample_rate as usize * total_channels * RING_BUFFER_SECONDS;
        let pipeline = spawn_writer_pipeline("blackbox-monitor", "monitor", ring_size, state)?;
        self.writer_thread = Some(pipeline.handle);
        let mut producer = pipeline.producer;

        let write_errors = Arc::clone(&self.write_errors);
        // Error callback — same shared method as the recording path.
        let err_fn = self.build_stream_err_callback();
        let callback = move |data: &[f32], _: &cpal::InputCallbackInfo| {
            // DOLL-353: use the single audited RT-safe push helper
            // (same as the recording callback) so the monitoring
            // producer can't drift from the overflow-counting
            // contract covered by push_samples_counts_rejected_suffix.
            push_samples_with_overflow_count(&mut producer, data, total_channels, &write_errors);
        };
        let stream = start_f32_input_stream(&device, stream_config, callback, err_fn)?;

        self.stream = Some(Box::new(stream));
        self.monitoring = true;
        // Release store synchronizes-with the Acquire load in
        // `is_monitoring`; readers seeing `true` also observe the prior
        // `sample_rate_atomic.store(rate, Relaxed)` (DOLL-101).
        self.monitoring_active.store(true, Ordering::Release);

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
                // Wait for writer thread shutdown (5s timeout; silently skipped on timeout)
                if let Ok(_result) = reply_rx.recv_timeout(Duration::from_secs(5))
                    && let Some(jh) = handle.join_handle.take()
                    && jh.join().is_err()
                {
                    warn!("writer thread panicked before join");
                }
            }
        }

        self.monitoring = false;
        // Order matters: Relaxed clear of sample_rate first, then Release
        // store of `false` to monitoring_active. Readers Acquire-loading
        // `monitoring_active = false` then see sample_rate_atomic = 0.
        self.sample_rate_atomic.store(0, Ordering::Relaxed);
        // Also clear `sample_rate_changed` (DOLL-123) — symmetry with
        // finalize / start-side reset.
        self.sample_rate_changed.store(false, Ordering::Relaxed);
        self.monitoring_active.store(false, Ordering::Release);
        self.peak_levels = Arc::from(Vec::new());

        Ok(())
    }

    fn is_monitoring(&self) -> bool {
        self.monitoring_active.load(Ordering::Acquire)
    }

    fn gate_idle(&self) -> bool {
        self.gate_idle.load(Ordering::Relaxed)
    }
}

impl Drop for CpalAudioProcessor {
    fn drop(&mut self) {
        if self.monitoring {
            if let Err(e) = self.stop_monitoring() {
                error!("Error stopping monitoring during cleanup: {e}");
            }
        } else if self.is_recording()
            && let Err(e) = self.finalize()
        {
            error!("Error during cleanup: {e}");
        }
    }
}

#[cfg(test)]
impl CpalAudioProcessor {
    /// Create a `CpalAudioProcessor` for testing without requiring audio hardware.
    ///
    /// Uses `WriterThreadState` directly (no ring buffer or writer thread).
    ///
    /// # Errors
    ///
    /// Fails like [`new_for_test_with_bits`](Self::new_for_test_with_bits).
    pub fn new_for_test(
        output_dir: &str,
        sample_rate: u32,
        channels: &[usize],
        output_mode: OutputMode,
    ) -> Result<Self, BlackboxError> {
        Self::new_for_test_with_bits(output_dir, sample_rate, channels, output_mode, 16)
    }

    /// Like `new_for_test` but with configurable bit depth.
    ///
    /// # Errors
    ///
    /// Returns [`BlackboxError::Io`] if the output directory can't be created,
    /// or a WAV error if an output file can't be opened. The disk-space check
    /// is disabled here.
    pub fn new_for_test_with_bits(
        output_dir: &str,
        sample_rate: u32,
        channels: &[usize],
        output_mode: OutputMode,
        bits_per_sample: u16,
    ) -> Result<Self, BlackboxError> {
        if !Path::new(output_dir).exists() {
            fs::create_dir_all(output_dir)?;
        }

        let write_errors = Arc::new(AtomicU64::new(0));

        let disk_space_low = Arc::new(AtomicBool::new(false));

        let peak_levels: Arc<[CacheAlignedPeak]> =
            std::iter::repeat_with(|| CacheAlignedPeak::new(0))
                .take(channels.len())
                .collect();

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
            false, // gate disabled in default test helper
            0,
        )?;
        // For tests, total_device_channels is set per feed_test_data call
        state.total_device_channels = 0;

        Ok(Self {
            sample_rate,
            stream: None,
            continuous_mode: false,
            recording_cadence: 0,
            output_dir: output_dir.to_owned(),
            channels: channels.to_vec(),
            output_mode,
            debug: false,
            write_errors,
            disk_space_low,
            write_failed: Arc::new(AtomicBool::new(false)),
            stream_error: Arc::new(AtomicBool::new(false)),
            #[cfg(target_os = "macos")]
            rate_listener: None,
            sample_rate_changed: Arc::new(AtomicBool::new(false)),
            peak_levels,
            gate_idle: Arc::new(AtomicBool::new(false)),
            // Tests call `feed_test_data` then `finalize` directly, never going
            // through `process_audio_impl`. Match the prior behaviour where
            // `is_recording()` was false for `new_for_test` processors —
            // letting `Drop` skip `finalize()` since each test owns its own
            // teardown sequence.
            recording_active: Arc::new(AtomicBool::new(false)),
            monitoring_active: Arc::new(AtomicBool::new(false)),
            sample_rate_atomic: Arc::new(AtomicU32::new(sample_rate)),
            writer_thread: None,
            monitoring: false,
            direct_state: Some(state),
        })
    }

    /// Feed interleaved f32 audio data as if it came from a cpal callback.
    ///
    /// # Panics
    ///
    /// If `total_device_channels` exceeds `u16::MAX`, which no device reports.
    pub fn feed_test_data(&mut self, data: &[f32], total_device_channels: usize) {
        if let Some(ref mut state) = self.direct_state {
            state.total_device_channels =
                u16::try_from(total_device_channels).expect("test channel count fits in u16");
            state.write_samples(data);
        }
    }

    /// Return the current write-error count.
    #[must_use]
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

#[cfg(test)]
mod rotation_tests {
    use super::{advance_rotation_counter, rotation_threshold_samples};

    /// DOLL-453: the threshold must scale with the device's TOTAL channel
    /// count, because the RT callback counts interleaved samples across all
    /// channels. The historic risk: forgetting the channel factor makes a
    /// 4-channel device rotate every `cadence * 4` seconds (30 min instead
    /// of 7.5) — every prior rotation test set `rotation_needed` manually
    /// and would never catch it.
    #[test]
    fn threshold_scales_with_channel_count_and_cadence() {
        // mono, 48 kHz, 5-minute cadence
        assert_eq!(rotation_threshold_samples(48_000, 1, 300), 14_400_000);
        // 4-channel device sees interleaved samples 4× as fast
        assert_eq!(rotation_threshold_samples(48_000, 4, 300), 57_600_000);
        // cadence factor
        assert_eq!(rotation_threshold_samples(44_100, 2, 60), 5_292_000);
    }

    /// Counter accumulates across batches, returns false below the threshold,
    /// fires exactly when crossing it, and carries the overshoot into the
    /// next period.
    #[test]
    fn counter_accumulates_fires_and_carries_overshoot() {
        let threshold = 1_000_u64;
        let mut counter = 0_u64;

        assert!(!advance_rotation_counter(&mut counter, 400, threshold));
        assert!(!advance_rotation_counter(&mut counter, 400, threshold));
        assert_eq!(counter, 800);

        assert!(
            advance_rotation_counter(&mut counter, 400, threshold),
            "crossing the threshold must signal a rotation"
        );
        assert_eq!(counter, 200, "the 200-sample overshoot must carry over");

        // The next period is shortened by the carried overshoot.
        assert!(!advance_rotation_counter(&mut counter, 799, threshold));
        assert!(advance_rotation_counter(&mut counter, 1, threshold));
        assert_eq!(counter, 0, "landing exactly on the threshold carries 0");
    }

    /// Rotation `n` must land within one callback of `n * threshold`
    /// samples, however many periods have passed. Resetting to 0 on fire
    /// dropped each period's overshoot: with 300-sample callbacks and a
    /// 1000-sample threshold every period became 1200 samples, so the 100th
    /// boundary sat 20,000 samples (67 callbacks) late.
    #[test]
    fn rotation_boundaries_do_not_drift() {
        let threshold = 1_000_u64;
        let batch = 300_usize;
        let mut counter = 0_u64;
        let mut consumed = 0_u64;
        let mut rotations = 0_u64;
        while rotations < 100 {
            consumed += batch as u64;
            if advance_rotation_counter(&mut counter, batch, threshold) {
                rotations += 1;
                let ideal = rotations * threshold;
                assert!(
                    consumed >= ideal && consumed - ideal < batch as u64,
                    "rotation {rotations} fired after {consumed} samples, \
                     more than one callback past {ideal}"
                );
            }
        }
    }

    /// A callback longer than a whole period flags one rotation and keeps
    /// only the sub-period remainder, instead of a backlog of rotations.
    #[test]
    fn oversized_batch_fires_once_and_keeps_phase() {
        let mut counter = 0_u64;
        assert!(advance_rotation_counter(&mut counter, 2_500, 1_000));
        assert_eq!(counter, 500);
        assert!(!advance_rotation_counter(&mut counter, 400, 1_000));
    }

    /// Wall-clock equivalence: with the threshold derived from the channel
    /// count, a mono and a 4-channel device must both rotate after the same
    /// number of CALLBACKS (i.e., the same elapsed time), even though the
    /// 4-channel batches carry 4× the samples. This is the end-to-end form
    /// of the off-by-channel-count regression.
    #[test]
    fn rotation_fires_at_same_wall_clock_across_channel_counts() {
        let sample_rate = 48_000_u32;
        let cadence_secs = 10_u64;
        let frames_per_callback = 480_usize; // 10 ms of frames per callback

        let callbacks_to_fire = |channels: usize| -> u32 {
            let threshold = rotation_threshold_samples(sample_rate, channels, cadence_secs);
            let batch = frames_per_callback * channels; // interleaved samples
            let mut counter = 0_u64;
            let mut n = 0_u32;
            loop {
                n += 1;
                if advance_rotation_counter(&mut counter, batch, threshold) {
                    return n;
                }
            }
        };

        let mono = callbacks_to_fire(1);
        let quad = callbacks_to_fire(4);
        assert_eq!(
            mono, quad,
            "rotation must fire after the same elapsed time regardless of channel count"
        );
        // 10 s cadence / 10 ms callbacks = 1000 callbacks.
        assert_eq!(mono, 1_000);
    }

    /// `recording_cadence` = 0 → threshold 0 → EVERY callback signals a
    /// rotation (the DOLL-458 rotation storm). `get_recording_cadence` now
    /// rejects 0 at the config boundary, so production can't reach this —
    /// the test documents why that guard exists and what the raw arithmetic
    /// does without it.
    #[test]
    fn zero_cadence_fires_every_batch() {
        let threshold = rotation_threshold_samples(48_000, 2, 0);
        assert_eq!(threshold, 0);
        let mut counter = 0_u64;
        assert!(advance_rotation_counter(&mut counter, 512, threshold));
        assert!(advance_rotation_counter(&mut counter, 512, threshold));
        assert_eq!(counter, 0, "a zero threshold must not accumulate");
    }
}

#[cfg(test)]
mod push_samples_tests {
    use super::push_samples_with_overflow_count;
    use std::sync::atomic::{AtomicU64, Ordering};

    /// Direct unit test for `push_samples_with_overflow_count` (DOLL-130).
    /// The integration test (`test_ring_buffer_overflow_counted`) spins up
    /// a writer thread, two channels, and a temp dir to exercise four
    /// arithmetic lines; this catches drift in those lines faster.
    ///
    /// Killer-question: revert the `fetch_add(remainder.len() as u64, ...)`
    /// branch (or change the count to a fixed constant) and this test
    /// fails immediately.
    #[test]
    fn push_samples_counts_rejected_suffix() {
        let (mut producer, mut consumer) = rtrb::RingBuffer::<f32>::new(16);
        let write_errors = AtomicU64::new(0);

        // Push fewer than capacity → no overflow.
        let small = vec![0.0_f32; 8];
        push_samples_with_overflow_count(&mut producer, &small, 1, &write_errors);
        assert_eq!(
            write_errors.load(Ordering::Relaxed),
            0,
            "no overflow expected when pushing within capacity"
        );

        // Push exactly the remaining capacity → fits, still no overflow.
        let fill = vec![0.0_f32; 8];
        push_samples_with_overflow_count(&mut producer, &fill, 1, &write_errors);
        assert_eq!(
            write_errors.load(Ordering::Relaxed),
            0,
            "no overflow when filling to exact capacity"
        );

        // Now the ring is full; the next push has zero capacity, so the
        // entire batch is rejected. The counter must reflect the full
        // remainder length.
        let overflow = vec![0.0_f32; 100];
        push_samples_with_overflow_count(&mut producer, &overflow, 1, &write_errors);
        assert_eq!(
            write_errors.load(Ordering::Relaxed),
            100,
            "all 100 samples should have been rejected when ring is full"
        );

        // Asymmetric case: drain 4 slots, then push 10 — 4 fit, 6 are rejected.
        if let Ok(chunk) = consumer.read_chunk(4) {
            chunk.commit_all();
        }
        let asymmetric = vec![0.0_f32; 10];
        push_samples_with_overflow_count(&mut producer, &asymmetric, 1, &write_errors);
        assert_eq!(
            write_errors.load(Ordering::Relaxed),
            106,
            "expected 100 + 6 rejected samples after asymmetric push"
        );
    }

    /// On overflow only whole frames are pushed. With 5 free slots and a
    /// 2-channel stream, 4 samples (2 frames) go in and the partial frame is
    /// counted with the rest of the rejected tail. Pushing the 5th sample
    /// (the old behavior) would leave half a frame in the ring and shift
    /// every later sample onto the wrong channel.
    #[test]
    fn overflow_push_keeps_frames_aligned() {
        let (mut producer, mut consumer) = rtrb::RingBuffer::<f32>::new(16);
        let write_errors = AtomicU64::new(0);

        // Fill 11 of 16 slots, leaving 5 free (an odd count).
        push_samples_with_overflow_count(&mut producer, &[0.0_f32; 11], 1, &write_errors);
        assert_eq!(producer.slots(), 5);

        // Stereo batch of 4 frames: L = 1.0, R = -1.0.
        let batch: Vec<f32> = [1.0_f32, -1.0].repeat(4);
        push_samples_with_overflow_count(&mut producer, &batch, 2, &write_errors);
        assert_eq!(
            write_errors.load(Ordering::Relaxed),
            4,
            "2 whole frames fit; the partial frame and the last frame are rejected"
        );
        assert_eq!(producer.slots(), 1, "exactly 2 frames were pushed");

        // Drop the filler, then check that the ring ends on a frame boundary.
        consumer.read_chunk(11).unwrap().commit_all();
        let pushed: Vec<f32> = std::iter::from_fn(|| consumer.pop().ok()).collect();
        assert_eq!(pushed.len(), 4);
        for &[left, right] in pushed.as_chunks::<2>().0 {
            assert!(
                left > 0.0 && right < 0.0,
                "frame must stay L/R: [{left}, {right}]"
            );
        }
    }
}
