use crate::audio_processor::AudioProcessor;
use crate::constants::{
    DEFAULT_CHANNELS, DEFAULT_DEBUG, DEFAULT_DURATION, DEFAULT_OUTPUT_MODE,
    DEFAULT_SILENCE_THRESHOLD,
};
use crate::utils::parse_channel_string;
use std::env;

/// The main struct responsible for coordinating audio recording.
///
/// It takes an implementation of the AudioProcessor trait, configures it
/// based on environment variables, and manages the recording process.
pub struct AudioRecorder<P: AudioProcessor> {
    pub processor: P,
}

impl<P: AudioProcessor> AudioRecorder<P> {
    /// Create a new AudioRecorder with the given processor.
    pub fn new(processor: P) -> Self {
        AudioRecorder { processor }
    }

    /// Get a reference to the processor.
    pub fn get_processor(&self) -> &P {
        &self.processor
    }

    /// Start the recording process using environment variables for configuration.
    ///
    /// This method reads configuration from environment variables, initializes
    /// the audio processor, and starts recording.
    pub fn start_recording(&mut self) -> Result<String, String> {
        // Read environment variables
        let channels_str =
            env::var("AUDIO_CHANNELS").unwrap_or_else(|_| DEFAULT_CHANNELS.to_string());

        // Parse channels, which can now include ranges
        let channels = parse_channel_string(&channels_str)?;

        let debug: bool = env::var("DEBUG")
            .unwrap_or_else(|_| DEFAULT_DEBUG.to_string())
            .parse()
            .expect("Invalid debug flag");

        let record_duration: u64 = env::var("RECORD_DURATION")
            .unwrap_or_else(|_| DEFAULT_DURATION.to_string())
            .parse()
            .expect("Invalid record duration");

        let output_mode: String =
            env::var("OUTPUT_MODE").unwrap_or_else(|_| DEFAULT_OUTPUT_MODE.to_string());

        let silence_threshold: i32 = env::var("SILENCE_THRESHOLD")
            .unwrap_or_else(|_| DEFAULT_SILENCE_THRESHOLD.to_string())
            .parse()
            .expect("Invalid silence threshold");

        // Print recording information
        println!("Starting recording:");
        println!("  Channels: {:?}", channels);
        println!("  Debug: {}", debug);
        println!("  Duration: {} seconds", record_duration);
        println!("  Output Mode: {}", output_mode);
        if silence_threshold > 0 {
            println!(
                "  Silence Threshold: {} (files below this will be deleted)",
                silence_threshold
            );
        } else {
            println!("  Silence Detection: Disabled");
        }

        // Process audio based on channels and config
        self.processor.process_audio(&channels, &output_mode, debug);

        // Return a success message
        Ok("Recording in progress. Press Ctrl+C to stop.".to_string())
    }
}
