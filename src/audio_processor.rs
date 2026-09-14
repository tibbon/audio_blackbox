//! `AudioProcessor` — central trait abstracting over real (cpal) and
//! mock processors.
//!
//! Production code uses `CpalAudioProcessor`; tests use
//! `MockAudioProcessor` for deterministic behavior without hardware.
//! The trait surface is the only thing `AudioRecorder` knows about,
//! which lets the test suite swap implementations without touching the
//! recorder loop.

use crate::config::AppConfig;
use crate::constants::OutputMode;
use crate::error::BlackboxError;

/// The `AudioProcessor` trait defines the interface for processing audio data.
///
/// Implementations of this trait are responsible for handling the actual audio
/// processing, including recording from input devices and writing to WAV files.
pub trait AudioProcessor {
    /// Configure the audio pipeline for the given channel selection and
    /// output mode without starting capture. Builds the cpal stream,
    /// allocates the ring buffer, opens output files, and spawns the
    /// writer thread. Idempotent during a single recording session;
    /// callers typically pair with [`start_recording`](Self::start_recording).
    ///
    /// # Errors
    ///
    /// The cpal implementation fails when the pipeline can't be built: no usable
    /// input device, a stream that won't build or start, or an unsupported sample
    /// format ([`BlackboxError::AudioDevice`], [`BlackboxError::AudioDeviceSource`]);
    /// free disk space below `min_disk_space_mb` ([`BlackboxError::InsufficientDiskSpace`]);
    /// or an output directory or WAV file that can't be created
    /// ([`BlackboxError::Io`], [`BlackboxError::WavSource`]).
    fn process_audio(
        &mut self,
        channels: &[usize],
        output_mode: OutputMode,
        debug: bool,
        config: &AppConfig,
    ) -> Result<(), BlackboxError>;

    /// Stop the audio stream, drain the ring buffer, finalize WAV
    /// headers, and join the writer + silence-check threads. Must be
    /// called before drop to avoid losing the tail of the recording.
    ///
    /// # Errors
    ///
    /// Returns the first error from finalizing an output file
    /// ([`BlackboxError::Wav`]) or renaming it into place
    /// ([`BlackboxError::Io`]). Later files are still finalized on a
    /// best-effort basis.
    fn finalize(&mut self) -> Result<(), BlackboxError>;

    /// Begin the cpal stream so samples flow into the ring buffer.
    /// Requires [`process_audio`](Self::process_audio) to have been
    /// called first to set up the pipeline. Returns immediately —
    /// recording continues asynchronously until `stop_recording` or
    /// `finalize`.
    ///
    /// # Errors
    ///
    /// The cpal implementation parses the configured channel list first
    /// ([`BlackboxError::ChannelParse`]), then fails like
    /// [`process_audio`](Self::process_audio).
    fn start_recording(&mut self, config: &AppConfig) -> Result<(), BlackboxError>;

    /// Pause the cpal stream without finalizing files. The pipeline
    /// stays configured so a subsequent `start_recording` resumes
    /// without re-allocating the ring buffer or reopening files. Use
    /// `finalize` to fully tear down.
    ///
    /// # Errors
    ///
    /// The cpal implementation finalizes the recording, so it fails like
    /// [`finalize`](Self::finalize).
    fn stop_recording(&mut self) -> Result<(), BlackboxError>;

    /// Whether the cpal stream is currently running. False after
    /// `stop_recording` or `finalize`, before `start_recording`, or
    /// when configuration failed.
    fn is_recording(&self) -> bool;

    /// Return the number of audio samples lost due to write errors or buffer overflows.
    fn write_error_count(&self) -> u64 {
        0
    }

    /// Whether recording has been paused because available disk space is below threshold.
    fn disk_space_low(&self) -> bool {
        false
    }

    /// Whether the audio stream has encountered an error (e.g., device disconnected).
    fn stream_error(&self) -> bool {
        false
    }

    /// Whether the audio device's sample rate has changed since recording started.
    fn sample_rate_changed(&self) -> bool {
        false
    }

    /// Return per-channel peak levels (0.0..1.0) for metering.
    fn peak_levels(&self) -> Vec<f32> {
        Vec::new()
    }

    /// Write per-channel peak levels into a caller-provided buffer.
    /// Returns the number of channels written (may be less than `buf.len()`).
    /// Default implementation delegates to `peak_levels()`.
    fn fill_peak_levels(&self, buf: &mut [f32]) -> usize {
        let peaks = self.peak_levels();
        let count = peaks.len().min(buf.len());
        buf[..count].copy_from_slice(&peaks[..count]);
        count
    }

    /// Return the sample rate of the active audio stream (0 if unknown).
    fn sample_rate(&self) -> u32 {
        0
    }

    /// Start monitoring audio levels without recording to disk.
    ///
    /// # Errors
    ///
    /// The default implementation never fails. The cpal implementation returns
    /// [`BlackboxError::ChannelParse`] for an invalid channel list, and
    /// [`BlackboxError::AudioDevice`] or [`BlackboxError::AudioDeviceSource`] when
    /// the input device or its stream can't be opened.
    fn start_monitoring(&mut self, _config: &AppConfig) -> Result<(), BlackboxError> {
        Ok(())
    }

    /// Stop monitoring audio levels.
    ///
    /// # Errors
    ///
    /// Neither the default nor the cpal implementation returns an error today;
    /// the `Result` leaves room for a teardown that can fail.
    fn stop_monitoring(&mut self) -> Result<(), BlackboxError> {
        Ok(())
    }

    /// Whether audio monitoring is currently active.
    fn is_monitoring(&self) -> bool {
        false
    }

    /// Whether the silence gate is currently idle (no files open, waiting for signal).
    fn gate_idle(&self) -> bool {
        false
    }
}
