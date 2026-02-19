use log::{debug, error};

use crate::audio_processor::AudioProcessor;
use crate::config::AppConfig;
use crate::error::BlackboxError;
use std::fs;
use std::path::Path;
use std::path::PathBuf;

#[cfg(test)]
use crate::constants::DEFAULT_OUTPUT_MODE;

/// MockAudioProcessor simulates audio processing for testing purposes
/// without requiring actual audio hardware.
pub struct MockAudioProcessor {
    pub channels: Vec<usize>,
    pub output_mode: String,
    pub debug: bool,
    pub audio_processed: bool,
    pub finalized: bool,
    pub file_name: String,
    pub created_files: Vec<String>,
    /// When true, creates files with very low amplitude samples that will be
    /// detected as silent by the silence detection algorithm. Used for testing
    /// the automatic deletion of silent recordings.
    pub create_silent_file: bool,
    /// When true, finalize will return an error. Used for testing error handling.
    pub should_fail_finalize: bool,
}

impl MockAudioProcessor {
    #[cfg(test)]
    pub fn new(file_name: &str) -> Self {
        MockAudioProcessor {
            channels: Vec::new(),
            output_mode: DEFAULT_OUTPUT_MODE.to_string(),
            debug: false,
            audio_processed: false,
            finalized: false,
            file_name: file_name.to_string(),
            created_files: Vec::new(),
            create_silent_file: false,
            should_fail_finalize: false,
        }
    }
}

impl AudioProcessor for MockAudioProcessor {
    fn process_audio(
        &mut self,
        channels: &[usize],
        output_mode: &str,
        debug: bool,
    ) -> Result<(), BlackboxError> {
        self.channels = channels.to_vec();
        self.output_mode = output_mode.to_string();
        self.debug = debug;
        self.audio_processed = true;
        self.created_files.clear();

        // Choose amplitude based on silence flag
        let amplitude = if self.create_silent_file { 0 } else { 50 };

        // Make sure the output directory exists
        if let Some(dir) = Path::new(&self.file_name).parent() {
            if !dir.exists() {
                fs::create_dir_all(dir)?;
            }
        }

        // Always create the main file
        let spec = hound::WavSpec {
            channels: if output_mode == "split" { 1 } else { 2 },
            sample_rate: 44100,
            bits_per_sample: 16,
            sample_format: hound::SampleFormat::Int,
        };

        match hound::WavWriter::create(&self.file_name, spec) {
            Ok(mut writer) => {
                for i in 0..1000 {
                    let sample = (i % 100) * amplitude;
                    let _ = writer.write_sample(sample);
                    if output_mode != "split" {
                        let _ = writer.write_sample(sample);
                    }
                }
                let _ = writer.finalize();
            }
            Err(e) => {
                error!("Error creating test WAV file: {}", e);
            }
        }

        self.created_files.push(self.file_name.clone());

        if output_mode == "split" {
            // Create an empty WAV file for each channel
            for &channel in channels {
                let base_path = Path::new(&self.file_name);
                let file_name = if let Some(stem) = base_path.file_stem().and_then(|s| s.to_str()) {
                    if let Some(ext) = base_path.extension().and_then(|s| s.to_str()) {
                        format!("{}-ch{}.{}", stem, channel, ext)
                    } else {
                        format!("{}-ch{}", stem, channel)
                    }
                } else {
                    format!("{}-ch{}", self.file_name, channel)
                };

                let file_path = if let Some(parent) = base_path.parent() {
                    parent.join(file_name)
                } else {
                    PathBuf::from(file_name)
                };

                self.created_files
                    .push(file_path.to_string_lossy().into_owned());

                let spec = hound::WavSpec {
                    channels: 1,
                    sample_rate: 44100,
                    bits_per_sample: 16,
                    sample_format: hound::SampleFormat::Int,
                };

                match hound::WavWriter::create(&file_path, spec) {
                    Ok(mut writer) => {
                        for i in 0..1000 {
                            let sample = (i % 100) * amplitude;
                            let _ = writer.write_sample(sample);
                        }
                        let _ = writer.finalize();
                    }
                    Err(e) => {
                        error!("Error creating test WAV file: {}", e);
                    }
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
            return Err(BlackboxError::Wav("Simulated finalize failure".to_string()));
        }

        // Check if we should apply the silence threshold using AppConfig
        let config = AppConfig::load();
        let silence_threshold = config.get_silence_threshold();

        // Cast to i32 for comparison - using as i32 > 0 check
        if silence_threshold > 0.0 && self.create_silent_file {
            // If we're creating silent files and threshold is set, delete the files
            // since they should be below the threshold. This allows testing the
            // silence detection and deletion functionality.
            let files_to_delete = self.created_files.clone(); // Clone to avoid borrowing issues
            for file_path in &files_to_delete {
                if let Err(e) = fs::remove_file(file_path) {
                    error!("Failed to delete silent file in test: {}", e);
                    return Err(BlackboxError::Io(e));
                }
                debug!("Deleted silent test file: {}", file_path);
            }
        }

        Ok(())
    }

    fn start_recording(&mut self) -> Result<(), BlackboxError> {
        // Clone the values to avoid borrowing self mutably and immutably
        let channels = self.channels.clone();
        let output_mode = self.output_mode.clone();
        let debug = self.debug;

        // In the mock, we'll just simulate this by immediately processing audio
        // with the stored configuration
        self.process_audio(&channels, &output_mode, debug)
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
