use log::{debug, error};

use crate::audio_processor::AudioProcessor;
use crate::config::AppConfig;
use crate::constants::OutputMode;
use crate::error::BlackboxError;
use std::fs;
use std::path::Path;
use std::path::PathBuf;

/// `MockAudioProcessor` simulates audio processing for testing purposes
/// without requiring actual audio hardware.
#[expect(
    clippy::struct_excessive_bools,
    reason = "each flag is an independently observable piece of test-double state; folding them into an enum would obscure what a given test asserts"
)]
#[derive(Debug)]
pub struct MockAudioProcessor {
    /// Channel list captured from the last `process_audio` call.
    pub channels: Vec<usize>,
    /// Output mode captured from the last `process_audio` call.
    pub output_mode: OutputMode,
    /// Debug flag captured from the last `process_audio` call.
    pub debug: bool,
    /// Set once `process_audio` has run; cleared again by `stop_recording`.
    pub audio_processed: bool,
    /// Set once `finalize` has been called.
    pub finalized: bool,
    /// Path of the main WAV file the mock writes.
    pub file_name: String,
    /// Every file written by the last `process_audio` call, so tests can
    /// assert on (or clean up) the outputs.
    pub created_files: Vec<String>,
    /// When true, creates files with very low amplitude samples that will be
    /// detected as silent by the silence detection algorithm. Used for testing
    /// the automatic deletion of silent recordings.
    pub create_silent_file: bool,
    /// When true, finalize will return an error. Used for testing error handling.
    pub should_fail_finalize: bool,
}

impl MockAudioProcessor {
    /// Build a mock that writes to `file_name` with every flag off.
    #[cfg(test)]
    #[must_use]
    pub fn new(file_name: &str) -> Self {
        Self {
            channels: Vec::new(),
            output_mode: OutputMode::default(),
            debug: false,
            audio_processed: false,
            finalized: false,
            file_name: file_name.to_owned(),
            created_files: Vec::new(),
            create_silent_file: false,
            should_fail_finalize: false,
        }
    }
}

/// Write 1000 frames of a deterministic ramp scaled by `amplitude`.
///
/// `amplitude == 0` yields an all-zero file so the silence-deletion tests
/// have something the detector classifies as silent.
fn write_mock_wav(path: &Path, spec: hound::WavSpec, amplitude: i32) -> hound::Result<()> {
    let mut writer = hound::WavWriter::create(path, spec)?;
    for i in 0..1000_i32 {
        let sample = i.rem_euclid(100) * amplitude;
        for _ in 0..spec.channels {
            writer.write_sample(sample)?;
        }
    }
    writer.finalize()
}

impl AudioProcessor for MockAudioProcessor {
    fn process_audio(
        &mut self,
        channels: &[usize],
        output_mode: OutputMode,
        debug: bool,
        _config: &AppConfig,
    ) -> Result<(), BlackboxError> {
        self.channels = channels.to_vec();
        self.output_mode = output_mode;
        self.debug = debug;
        self.audio_processed = true;
        self.created_files.clear();

        // Choose amplitude based on silence flag
        let amplitude = if self.create_silent_file { 0 } else { 50 };

        // Make sure the output directory exists
        if let Some(dir) = Path::new(&self.file_name).parent()
            && !dir.exists()
        {
            fs::create_dir_all(dir)?;
        }

        // Always create the main file
        let spec = hound::WavSpec {
            channels: if matches!(output_mode, OutputMode::Split) {
                1
            } else {
                2
            },
            sample_rate: 44100,
            bits_per_sample: 24,
            sample_format: hound::SampleFormat::Int,
        };

        if let Err(e) = write_mock_wav(Path::new(&self.file_name), spec, amplitude) {
            error!("Error creating test WAV file: {e}");
        }

        self.created_files.push(self.file_name.clone());

        if matches!(output_mode, OutputMode::Split) {
            // Create an empty WAV file for each channel
            for &channel in channels {
                let base_path = Path::new(&self.file_name);
                let file_name = base_path.file_stem().and_then(|s| s.to_str()).map_or_else(
                    || format!("{}-ch{channel}", self.file_name),
                    |stem| {
                        base_path.extension().and_then(|s| s.to_str()).map_or_else(
                            || format!("{stem}-ch{channel}"),
                            |ext| format!("{stem}-ch{channel}.{ext}"),
                        )
                    },
                );

                let file_path = base_path.parent().map_or_else(
                    || PathBuf::from(&file_name),
                    |parent| parent.join(&file_name),
                );

                self.created_files
                    .push(file_path.to_string_lossy().into_owned());

                let channel_spec = hound::WavSpec {
                    channels: 1,
                    ..spec
                };

                if let Err(e) = write_mock_wav(&file_path, channel_spec, amplitude) {
                    error!("Error creating test WAV file: {e}");
                }
            }
            debug!(
                "Created {} individual mock channel WAV files",
                channels.len()
            );
        } else {
            debug!(
                "Created mock {} WAV file",
                if self.create_silent_file {
                    "silent"
                } else {
                    "normal"
                }
            );
        }

        Ok(())
    }

    fn finalize(&mut self) -> Result<(), BlackboxError> {
        self.finalized = true;

        if self.should_fail_finalize {
            return Err(BlackboxError::Wav("Simulated finalize failure".to_owned()));
        }

        // Check if we should apply the silence threshold using AppConfig
        let config = AppConfig::load();
        let silence_threshold = config.get_silence_threshold();

        if silence_threshold > 0.0 && self.create_silent_file {
            // If we're creating silent files and threshold is set, delete the files
            // since they should be below the threshold. This allows testing the
            // silence detection and deletion functionality.
            let files_to_delete = self.created_files.clone();
            for file_path in &files_to_delete {
                if let Err(e) = fs::remove_file(file_path) {
                    error!("Failed to delete silent file in test: {e}");
                    return Err(BlackboxError::Io(e));
                }
                debug!("Deleted silent test file: {file_path}");
            }
        }

        Ok(())
    }

    fn start_recording(&mut self, config: &AppConfig) -> Result<(), BlackboxError> {
        // Clone channels to avoid borrowing self mutably and immutably; output_mode is Copy.
        let channels = self.channels.clone();
        let output_mode = self.output_mode;
        let debug = self.debug;

        // In the mock, we'll just simulate this by immediately processing audio
        // with the stored configuration
        self.process_audio(&channels, output_mode, debug, config)
    }

    fn stop_recording(&mut self) -> Result<(), BlackboxError> {
        // Just mark as stopped, don't finalize yet
        self.audio_processed = false;
        Ok(())
    }

    fn is_recording(&self) -> bool {
        // In the mock, once we've processed audio, consider it "recording"
        self.audio_processed && !self.finalized
    }
}
