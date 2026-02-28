/// The AudioProcessor trait defines the interface for processing audio data.
///
/// Implementations of this trait are responsible for handling the actual audio
/// processing, including recording from input devices and writing to WAV files.
use crate::error::BlackboxError;

pub trait AudioProcessor {
    /// Process audio from the specified channels with the given output mode and debug flag.
    fn process_audio(
        &mut self,
        channels: &[usize],
        output_mode: &str,
        debug: bool,
    ) -> Result<(), BlackboxError>;

    /// Finalize the audio processing, closing any open files or resources.
    fn finalize(&mut self) -> Result<(), BlackboxError>;

    fn start_recording(&mut self) -> Result<(), BlackboxError>;
    fn stop_recording(&mut self) -> Result<(), BlackboxError>;
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
    fn start_monitoring(&mut self) -> Result<(), BlackboxError> {
        Ok(())
    }

    /// Stop monitoring audio levels.
    fn stop_monitoring(&mut self) -> Result<(), BlackboxError> {
        Ok(())
    }

    /// Whether audio monitoring is currently active.
    fn is_monitoring(&self) -> bool {
        false
    }
}
