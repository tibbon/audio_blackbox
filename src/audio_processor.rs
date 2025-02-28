/// The AudioProcessor trait defines the interface for processing audio data.
///
/// Implementations of this trait are responsible for handling the actual audio
/// processing, including recording from input devices and writing to WAV files.
pub trait AudioProcessor {
    /// Process audio from the specified channels with the given output mode and debug flag.
    fn process_audio(&mut self, channels: &[usize], output_mode: &str, debug: bool);

    /// Finalize the audio processing, closing any open files or resources.
    fn finalize(&mut self);
}
