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
}
