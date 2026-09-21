//! High-level driver wrapping an `AudioProcessor` + an `AppConfig`.
//!
//! Used by the CLI binary (`bin/main.rs`). The FFI layer bypasses this
//! type and drives `CpalAudioProcessor` directly — `AudioRecorder` is
//! a CLI-oriented convenience.

use std::fmt;

use log::info;

use crate::audio_processor::AudioProcessor;
use crate::config::AppConfig;
use crate::error::BlackboxError;
use crate::utils::parse_channel_string;

/// The main struct responsible for coordinating audio recording.
///
/// It takes an implementation of the `AudioProcessor` trait, configures it
/// based on environment variables and config files, and manages the recording process.
pub struct AudioRecorder<P: AudioProcessor> {
    processor: P,
    config: AppConfig,
}

// `P` is not required to be `Debug` (e.g. `CpalAudioProcessor` owns a live
// stream), so only the config is printed.
impl<P: AudioProcessor> fmt::Debug for AudioRecorder<P> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("AudioRecorder")
            .field("config", &self.config)
            .finish_non_exhaustive()
    }
}

impl<P: AudioProcessor> AudioRecorder<P> {
    /// Create a new `AudioRecorder` with the given processor.
    pub fn new(processor: P) -> Self {
        Self {
            processor,
            config: AppConfig::load(),
        }
    }

    /// Create a new `AudioRecorder` with the given processor and configuration.
    pub fn with_config(processor: P, config: AppConfig) -> Self {
        Self { processor, config }
    }

    /// Get a reference to the processor.
    pub fn get_processor(&self) -> &P {
        &self.processor
    }

    /// Get a mutable reference to the processor.
    pub fn processor_mut(&mut self) -> &mut P {
        &mut self.processor
    }

    /// Get a reference to the configuration.
    pub fn config(&self) -> &AppConfig {
        &self.config
    }

    /// Start the recording process using the configuration.
    ///
    /// This method reads configuration following precedence order:
    /// 1. Environment variables
    /// 2. Configuration file
    /// 3. Default values
    ///
    /// # Errors
    ///
    /// Returns [`BlackboxError::ChannelParse`] if the configured channel list is
    /// invalid, and otherwise any error from the processor's
    /// [`process_audio`](AudioProcessor::process_audio).
    pub fn start_recording(&mut self) -> Result<String, BlackboxError> {
        let debug = self.config.get_debug();

        // Get the selected channels
        let requested_channels = self.config.get_audio_channels();
        let channels = parse_channel_string(&requested_channels)?;

        // Get the output mode
        let output_mode = self.config.output_mode_parsed();

        // Log audio configuration
        info!("Starting recording:");
        info!("  Channels: {channels:?}");
        info!("  Debug: {debug}");

        let duration = self.config.get_duration();
        info!("  Duration: {duration} seconds");

        info!("  Output Mode: {output_mode}");

        let silence_threshold = self.config.get_silence_threshold();

        if silence_threshold > 0.0 {
            info!("  Silence Detection: Enabled (threshold: {silence_threshold})");
        } else {
            info!("  Silence Detection: Disabled");
        }

        // Start the processor with the selected configuration
        self.processor
            .process_audio(&channels, output_mode, debug, &self.config)?;

        Ok(format!("Recording started with channels {channels:?}"))
    }

    /// Start monitoring audio levels without recording to disk.
    ///
    /// # Errors
    ///
    /// Returns any error from the processor's
    /// [`start_monitoring`](AudioProcessor::start_monitoring).
    pub fn start_monitoring(&mut self) -> Result<(), BlackboxError> {
        self.processor.start_monitoring(&self.config)
    }

    /// Create a default config file if one doesn't exist
    ///
    /// # Errors
    ///
    /// Fails like [`AppConfig::create_config_file`].
    pub fn create_default_config(&self, path: &str) -> Result<(), BlackboxError> {
        self.config.create_config_file(path)
    }

    /// Reload configuration from environment and config files
    pub fn reload_config(&mut self) {
        self.config = AppConfig::load();
    }
}
