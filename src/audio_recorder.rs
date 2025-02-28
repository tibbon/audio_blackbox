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
        let debug = env::var("DEBUG")
            .unwrap_or_else(|_| DEFAULT_DEBUG.to_string())
            .parse::<bool>()
            .unwrap_or(false);

        // Get the selected channels from environment or default
        let requested_channels =
            env::var("AUDIO_CHANNELS").unwrap_or_else(|_| DEFAULT_CHANNELS.to_string());

        let channels = match parse_channel_string(&requested_channels) {
            Ok(chs) => chs,
            Err(e) => return Err(format!("Error parsing channels: {}", e)),
        };

        // Get the selected output mode from environment or default
        let output_mode =
            env::var("OUTPUT_MODE").unwrap_or_else(|_| DEFAULT_OUTPUT_MODE.to_string());

        // Print audio configuration
        println!("Starting recording:");
        println!("  Channels: {:?}", channels);
        println!("  Debug: {}", debug);

        // Get the recording duration from environment or default
        let duration = env::var("RECORD_DURATION")
            .unwrap_or_else(|_| DEFAULT_DURATION.to_string())
            .parse::<u64>()
            .unwrap_or(10);
        println!("  Duration: {} seconds", duration);

        println!("  Output Mode: {}", output_mode);

        // Check if silence detection is enabled
        let silence_threshold = env::var("SILENCE_THRESHOLD")
            .unwrap_or_else(|_| DEFAULT_SILENCE_THRESHOLD.to_string())
            .parse::<i32>()
            .unwrap_or(0);

        if silence_threshold > 0 {
            println!(
                "  Silence Detection: Enabled (threshold: {})",
                silence_threshold
            );
        } else {
            println!("  Silence Detection: Disabled");
        }

        // Start the processor with the selected configuration
        self.processor.process_audio(&channels, &output_mode, debug);

        Ok(format!("Recording started with channels {:?}", channels))
    }
}
