use std::fs;
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use crate::audio_processor::AudioProcessor;
use crate::config::AppConfig;
use crate::constants::{
    MultiChannelWriters, WavWriterType, INTERMEDIATE_BUFFER_SIZE, MAX_CHANNELS,
};
use crate::utils::{check_alsa_availability, is_silent, parse_channel_string};

use chrono::prelude::*;
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::SampleFormat;
use std::env;

/// CpalAudioProcessor handles recording from audio devices using the CPAL library,
/// and saving the audio data to WAV files.
pub struct CpalAudioProcessor {
    file_name: Arc<Mutex<String>>,
    writer: Arc<Mutex<Option<WavWriterType>>>,
    multichannel_writers: MultiChannelWriters,
    intermediate_buffer: Arc<Mutex<Vec<i32>>>,
    multichannel_buffers: Arc<Mutex<Vec<Vec<i32>>>>,
    #[allow(dead_code)]
    sample_rate: u32, // Kept for future features that might use it
    // Add a field to keep the stream alive
    #[allow(dead_code)]
    stream: Option<Box<dyn StreamTrait>>,
    // New fields for continuous recording
    continuous_mode: bool,
    recording_cadence: u64,
    output_dir: String,
    last_rotation_time: Arc<Mutex<Instant>>,
    channels: Arc<Mutex<Vec<usize>>>,
    output_mode: Arc<Mutex<String>>,
    debug: Arc<Mutex<bool>>,
    current_spec: Arc<Mutex<Option<hound::WavSpec>>>,
}

impl CpalAudioProcessor {
    /// Create a new CpalAudioProcessor instance.
    ///
    /// This sets up the recording environment, including WAV file writers
    /// and audio stream configuration.
    pub fn new() -> Result<Self, String> {
        // Load configuration
        let config = AppConfig::load();

        // Check if ALSA is available on Linux
        check_alsa_availability()?;

        // Get configuration values
        let output_dir = config.get_output_dir();
        let continuous_mode = config.get_continuous_mode();
        let recording_cadence = config.get_recording_cadence();

        // Check and create output directory
        if !Path::new(&output_dir).exists() {
            fs::create_dir_all(&output_dir)
                .map_err(|e| format!("Failed to create output directory: {}", e))?;
        }

        // Generate the output file name
        let now: DateTime<Local> = Local::now();
        let file_name = format!(
            "{}-{:02}-{:02}-{:02}-{:02}.wav",
            now.year(),
            now.month(),
            now.day(),
            now.hour(),
            now.minute()
        );

        let host = cpal::default_host();
        let device = host
            .default_input_device()
            .ok_or_else(|| "No input device available".to_string())?;

        println!(
            "Using audio device: {}",
            device.name().map_err(|e| e.to_string())?
        );

        let config_audio = device
            .default_input_config()
            .map_err(|e| format!("Failed to get default input stream config: {}", e))?;

        println!("Default input stream config: {:?}", config_audio);

        let sample_rate = config_audio.sample_rate().0;

        let spec = hound::WavSpec {
            channels: 2, // Default is stereo WAV for backward compatibility
            sample_rate,
            bits_per_sample: 16,
            sample_format: hound::SampleFormat::Int,
        };

        // In continuous mode, we'll create the first file in the output directory
        let full_path = format!("{}/{}", output_dir, file_name);

        let writer = Arc::new(Mutex::new(Some(
            hound::WavWriter::create(&full_path, spec)
                .map_err(|e| format!("Failed to create WAV file: {}", e))?,
        )));

        Ok(CpalAudioProcessor {
            file_name: Arc::new(Mutex::new(full_path)),
            writer,
            multichannel_writers: Arc::new(Mutex::new(Vec::new())),
            intermediate_buffer: Arc::new(Mutex::new(Vec::with_capacity(INTERMEDIATE_BUFFER_SIZE))),
            multichannel_buffers: Arc::new(Mutex::new(Vec::new())),
            sample_rate,
            stream: None,
            continuous_mode,
            recording_cadence,
            output_dir,
            last_rotation_time: Arc::new(Mutex::new(Instant::now())),
            channels: Arc::new(Mutex::new(Vec::new())),
            output_mode: Arc::new(Mutex::new(String::new())),
            debug: Arc::new(Mutex::new(false)),
            current_spec: Arc::new(Mutex::new(Some(spec))),
        })
    }

    /// Set up split mode recording where each channel is recorded to its own file.
    fn setup_split_mode(&self, channels: &[usize], sample_rate: u32) -> Result<(), String> {
        let now: DateTime<Local> = Local::now();
        let date_str = format!(
            "{}-{:02}-{:02}-{:02}-{:02}",
            now.year(),
            now.month(),
            now.day(),
            now.hour(),
            now.minute()
        );

        println!("Setting up split mode with {} channels", channels.len());

        let mut writers = self.multichannel_writers.lock().unwrap();
        let mut buffers = self.multichannel_buffers.lock().unwrap();

        // Ensure the vectors are of the correct size
        writers.clear();
        // Instead of resize, we'll use a loop to add the writers
        for _ in 0..MAX_CHANNELS {
            writers.push(None);
        }

        buffers.clear();
        buffers.resize(channels.len(), Vec::with_capacity(INTERMEDIATE_BUFFER_SIZE));

        // Create a WAV writer for each channel
        for (idx, &channel) in channels.iter().enumerate() {
            let channel_file_name = format!("{}/{}-ch{}.wav", self.output_dir, date_str, channel);

            let spec = hound::WavSpec {
                channels: 1, // Mono for each individual channel
                sample_rate,
                bits_per_sample: 16,
                sample_format: hound::SampleFormat::Int,
            };

            let writer = hound::WavWriter::create(&channel_file_name, spec)
                .map_err(|e| format!("Failed to create channel WAV file: {}", e))?;

            writers[idx] = Some(writer);
            println!("Created channel WAV file: {}", channel_file_name);
        }

        // Store the current configuration
        *self.current_spec.lock().unwrap() = Some(hound::WavSpec {
            channels: 1,
            sample_rate,
            bits_per_sample: 16,
            sample_format: hound::SampleFormat::Int,
        });

        Ok(())
    }

    /// Set up multichannel mode recording where all channels are recorded to a single file.
    fn setup_multichannel_mode(&self, channels: &[usize], sample_rate: u32) -> Result<(), String> {
        let now: DateTime<Local> = Local::now();
        let date_str = format!(
            "{}-{:02}-{:02}-{:02}-{:02}",
            now.year(),
            now.month(),
            now.day(),
            now.hour(),
            now.minute()
        );

        let multichannel_file_name = format!("{}/{}-multichannel.wav", self.output_dir, date_str);

        println!(
            "Setting up multichannel mode with {} channels",
            channels.len()
        );

        let spec = hound::WavSpec {
            channels: channels.len() as u16,
            sample_rate,
            bits_per_sample: 16,
            sample_format: hound::SampleFormat::Int,
        };

        let writer = hound::WavWriter::create(&multichannel_file_name, spec)
            .map_err(|e| format!("Failed to create multichannel WAV file: {}", e))?;

        // Replace the default stereo writer with our multichannel writer
        let mut writer_guard = self.writer.lock().unwrap();
        *writer_guard = Some(writer);

        // Store the current configuration
        *self.current_spec.lock().unwrap() = Some(spec);

        println!("Created multichannel WAV file: {}", multichannel_file_name);
        Ok(())
    }

    /// Set up standard mode recording for mono or stereo (1 or 2 channels).
    fn setup_standard_mode(&self, channels: &[usize], sample_rate: u32) -> Result<(), String> {
        let now: DateTime<Local> = Local::now();
        let date_str = format!(
            "{}-{:02}-{:02}-{:02}-{:02}",
            now.year(),
            now.month(),
            now.day(),
            now.hour(),
            now.minute()
        );

        println!("Setting up standard mode with {} channels", channels.len());

        // Determine if we're recording mono or stereo
        let num_channels = if channels.len() == 1 { 1 } else { 2 };

        // Create the WAV file
        let file_name = format!("{}/{}.wav", self.output_dir, date_str);

        let spec = hound::WavSpec {
            channels: num_channels as u16,
            sample_rate,
            bits_per_sample: 16,
            sample_format: hound::SampleFormat::Int,
        };

        let writer = hound::WavWriter::create(&file_name, spec)
            .map_err(|e| format!("Failed to create WAV file: {}", e))?;

        // Store the writer
        *self.writer.lock().unwrap() = Some(writer);
        println!("Created WAV file: {}", file_name);

        // Store the current configuration
        *self.current_spec.lock().unwrap() = Some(spec);

        // Initialize buffers
        let mut buffers = self.multichannel_buffers.lock().unwrap();
        buffers.clear();
        buffers.resize(channels.len(), Vec::with_capacity(INTERMEDIATE_BUFFER_SIZE));

        Ok(())
    }

    /// Rotates the current recording files, finalizing the current files and creating new ones.
    /// This method is part of the continuous recording mode functionality.
    #[allow(dead_code)]
    fn rotate_files(&self) -> Result<(), String> {
        if !self.continuous_mode {
            return Ok(());
        }

        println!("Rotating recording files...");

        // Load configuration
        let config = AppConfig::load();
        let silence_threshold = config.get_silence_threshold();

        // Get current configuration for recreating files
        let output_mode = self.output_mode.lock().unwrap().clone();
        let channels = self.channels.lock().unwrap().clone();

        // Store paths of files being finalized to check for silence later
        let mut created_files = Vec::new();

        // Finalize the main WAV file if it exists
        if let Some(writer) = self.writer.lock().unwrap().take() {
            let file_path = if output_mode == "single" && channels.len() > 2 {
                let now: DateTime<Local> = Local::now();
                format!(
                    "{}/{}-{:02}-{:02}-{:02}-{:02}-multichannel.wav",
                    self.output_dir,
                    now.year(),
                    now.month(),
                    now.day(),
                    now.hour(),
                    now.minute() - (now.minute() % 5) // Round to nearest 5 minutes for better organization
                )
            } else {
                let now: DateTime<Local> = Local::now();
                format!(
                    "{}/{}-{:02}-{:02}-{:02}-{:02}.wav",
                    self.output_dir,
                    now.year(),
                    now.month(),
                    now.day(),
                    now.hour(),
                    now.minute() - (now.minute() % 5)
                )
            };

            created_files.push(file_path.clone());

            if let Err(e) = writer.finalize() {
                eprintln!("Error finalizing WAV file during rotation: {}", e);
            } else {
                println!("Finalized recording to {}", file_path);
            }
        }

        // Finalize any multichannel writers
        let mut writers = self.multichannel_writers.lock().unwrap();
        for (idx, writer_opt) in writers.iter_mut().enumerate() {
            if let Some(writer) = writer_opt.take() {
                let now: DateTime<Local> = Local::now();
                let file_path = format!(
                    "{}/{}-{:02}-{:02}-{:02}-{:02}-ch{}.wav",
                    self.output_dir,
                    now.year(),
                    now.month(),
                    now.day(),
                    now.hour(),
                    now.minute() - (now.minute() % 5),
                    channels.get(idx).unwrap_or(&idx)
                );

                created_files.push(file_path.clone());

                if let Err(e) = writer.finalize() {
                    eprintln!("Error finalizing channel WAV file during rotation: {}", e);
                } else {
                    println!("Finalized recording to {}", file_path);
                }
            }
        }

        // Check for silence and delete silent files if threshold is set
        if silence_threshold > 0.0 {
            for file_path in created_files {
                match is_silent(&file_path, silence_threshold) {
                    Ok(true) => {
                        println!(
                            "Recording is silent (below threshold {}), deleting file",
                            silence_threshold
                        );
                        if let Err(e) = fs::remove_file(&file_path) {
                            eprintln!("Error deleting silent file: {}", e);
                        }
                    }
                    Ok(false) => {
                        println!(
                            "Recording is not silent (above threshold {}), keeping file",
                            silence_threshold
                        );
                    }
                    Err(e) => {
                        eprintln!("Error checking for silence: {}", e);
                    }
                }
            }
        }

        // Create new files for the next recording period
        match output_mode.as_str() {
            "split" => {
                self.setup_split_mode(&channels, self.sample_rate)?;
            }
            "single" if channels.len() > 2 => {
                self.setup_multichannel_mode(&channels, self.sample_rate)?;
            }
            _ => {
                // Standard stereo mode
                let now: DateTime<Local> = Local::now();
                let file_name = format!(
                    "{}-{:02}-{:02}-{:02}-{:02}.wav",
                    now.year(),
                    now.month(),
                    now.day(),
                    now.hour(),
                    now.minute()
                );

                let full_path = format!("{}/{}", self.output_dir, file_name);

                if let Some(spec) = &*self.current_spec.lock().unwrap() {
                    let writer = hound::WavWriter::create(&full_path, *spec).map_err(|e| {
                        format!("Failed to create new WAV file during rotation: {}", e)
                    })?;

                    *self.writer.lock().unwrap() = Some(writer);
                    println!("Created new recording file: {}", full_path);
                }
            }
        }

        // Reset the rotation timer
        *self.last_rotation_time.lock().unwrap() = Instant::now();

        Ok(())
    }

    /// Checks if it's time to rotate files based on the configured recording cadence.
    /// This method is part of the continuous recording mode functionality.
    #[allow(dead_code)]
    fn check_and_rotate_files(&self) -> Result<(), String> {
        if !self.continuous_mode {
            return Ok(());
        }

        let now = Instant::now();
        let last_rotation = *self.last_rotation_time.lock().unwrap();

        if now.duration_since(last_rotation) >= Duration::from_secs(self.recording_cadence) {
            self.rotate_files()?;
        }

        Ok(())
    }

    /// Update the file name with a new path
    #[allow(dead_code)]
    fn update_file_name(&self, new_path: String) {
        *self.file_name.lock().unwrap() = new_path;
    }
}

impl AudioProcessor for CpalAudioProcessor {
    fn process_audio(&mut self, channels: &[usize], output_mode: &str, debug: bool) {
        // Store the configuration for later use in continuous mode
        *self.channels.lock().unwrap() = channels.to_vec();
        *self.output_mode.lock().unwrap() = output_mode.to_string();
        *self.debug.lock().unwrap() = debug;

        // Get CPAL host and device
        let host = cpal::default_host();
        let device = host
            .default_input_device()
            .expect("No input device available");

        println!("Using audio device: {}", device.name().unwrap());

        let config = device
            .default_input_config()
            .expect("Failed to get default input stream config");

        println!("Default input stream config: {:?}", config);

        let total_channels = config.channels() as usize;
        let sample_rate = config.sample_rate().0;

        // Auto-adapt to available channels
        let mut actual_channels: Vec<usize> = Vec::new();

        // Validate and adapt channels
        for &channel in channels {
            if channel < total_channels {
                actual_channels.push(channel);
            } else {
                println!(
                    "Warning: Channel {} not available on device. Device only has {} channels.",
                    channel, total_channels
                );
            }
        }

        // If no requested channels are available, use all available channels
        if actual_channels.is_empty() {
            println!(
                "No requested channels available. Using all available channels (0 to {}).",
                total_channels - 1
            );
            actual_channels = (0..total_channels).collect();
        }

        println!("Using channels: {:?}", actual_channels);

        // Setup the appropriate output mode
        let valid_modes = ["split", "single"];
        if !valid_modes.contains(&output_mode) {
            panic!(
                "Invalid output mode: '{}'. Valid options are: {:?}",
                output_mode, valid_modes
            );
        }

        match output_mode {
            "split" => {
                if let Err(e) = self.setup_split_mode(&actual_channels, sample_rate) {
                    panic!("Failed to setup split mode: {}", e);
                }
            }
            "single" if actual_channels.len() <= 2 => {
                // For mono or stereo, use the standard WAV format
                if let Err(e) = self.setup_standard_mode(&actual_channels, sample_rate) {
                    panic!("Failed to setup standard mode: {}", e);
                }
            }
            "single" => {
                // For more than 2 channels, use multichannel mode
                if let Err(e) = self.setup_multichannel_mode(&actual_channels, sample_rate) {
                    panic!("Failed to setup multichannel mode: {}", e);
                }
            }
            _ => unreachable!(), // We already validated the output mode above
        }

        // Clone channels to own them in the closure
        let channels_owned: Vec<usize> = actual_channels.clone();
        let output_mode_owned = output_mode.to_string();

        let writer_clone = Arc::clone(&self.writer);
        let buffer_clone = Arc::clone(&self.intermediate_buffer);
        let multichannel_writers_clone = Arc::clone(&self.multichannel_writers);
        let multichannel_buffers_clone = Arc::clone(&self.multichannel_buffers);

        // For continuous mode, we need to check if we should rotate files
        let continuous_mode = self.continuous_mode;
        let last_rotation_time_clone = Arc::clone(&self.last_rotation_time);
        let recording_cadence = self.recording_cadence;

        // Instead of using a raw pointer, create a thread-safe mechanism
        // to finalize files when needed
        let output_dir = self.output_dir.clone();
        let current_spec = Arc::clone(&self.current_spec);
        let channels_clone = Arc::clone(&self.channels);
        let output_mode_clone = Arc::clone(&self.output_mode);
        let writer_for_rotation = Arc::clone(&self.writer);
        let multichannel_writers_for_rotation = Arc::clone(&self.multichannel_writers);
        let sample_rate = self.sample_rate;

        // Error callback
        let err_fn = move |err| {
            eprintln!("an error occurred on stream: {}", err);
        };

        // Create a stream based on the sample format
        let stream = match config.sample_format() {
            SampleFormat::F32 => {
                // Build a stream for f32 samples
                device.build_input_stream(
                    &config.into(),
                    move |data: &[f32], _: &_| {
                        if debug {
                            println!("Processing {} samples", data.len());
                        }

                        // Check if we need to rotate files in continuous mode
                        if continuous_mode {
                            let now = Instant::now();
                            let last_rotation = *last_rotation_time_clone.lock().unwrap();

                            if now.duration_since(last_rotation) >= Duration::from_secs(recording_cadence) {
                                // Perform file rotation using thread-safe mechanisms
                                println!("Rotating recording files...");

                                // Get current configuration for recreating files
                                let output_mode = output_mode_clone.lock().unwrap().clone();
                                let channels = channels_clone.lock().unwrap().clone();
                                let silence_threshold = env::var("SILENCE_THRESHOLD")
                                    .unwrap_or_else(|_| "0".to_string())
                                    .parse::<f32>()
                                    .unwrap_or(0.0);

                                // Store paths of files being finalized to check for silence later
                                let mut created_files = Vec::new();

                                // Finalize the main WAV file if it exists
                                if let Some(writer) = writer_for_rotation.lock().unwrap().take() {
                                    let file_path = if output_mode == "single" && channels.len() > 2 {
                                        let now: DateTime<Local> = Local::now();
                                        format!(
                                            "{}/{}-{:02}-{:02}-{:02}-{:02}-multichannel.wav",
                                            output_dir,
                                            now.year(),
                                            now.month(),
                                            now.day(),
                                            now.hour(),
                                            now.minute() - (now.minute() % 5) // Round to nearest 5 minutes for better organization
                                        )
                                    } else {
                                        let now: DateTime<Local> = Local::now();
                                        format!(
                                            "{}/{}-{:02}-{:02}-{:02}-{:02}.wav",
                                            output_dir,
                                            now.year(),
                                            now.month(),
                                            now.day(),
                                            now.hour(),
                                            now.minute() - (now.minute() % 5)
                                        )
                                    };

                                    created_files.push(file_path.clone());

                                    if let Err(e) = writer.finalize() {
                                        eprintln!("Error finalizing WAV file during rotation: {}", e);
                                    } else {
                                        println!("Finalized recording to {}", file_path);
                                    }
                                }

                                // Finalize any multichannel writers
                                let mut writers = multichannel_writers_for_rotation.lock().unwrap();
                                for (idx, writer_opt) in writers.iter_mut().enumerate() {
                                    if let Some(writer) = writer_opt.take() {
                                        let now: DateTime<Local> = Local::now();
                                        let file_path = format!(
                                            "{}/{}-{:02}-{:02}-{:02}-{:02}-ch{}.wav",
                                            output_dir,
                                            now.year(),
                                            now.month(),
                                            now.day(),
                                            now.hour(),
                                            now.minute() - (now.minute() % 5),
                                            channels.get(idx).unwrap_or(&idx)
                                        );

                                        created_files.push(file_path.clone());

                                        if let Err(e) = writer.finalize() {
                                            eprintln!("Error finalizing channel WAV file during rotation: {}", e);
                                        } else {
                                            println!("Finalized recording to {}", file_path);
                                        }
                                    }
                                }

                                // Check for silence and delete silent files if threshold is set
                                if silence_threshold > 0.0 {
                                    for file_path in created_files {
                                        match is_silent(&file_path, silence_threshold) {
                                            Ok(true) => {
                                                println!(
                                                    "Recording is silent (below threshold {}), deleting file",
                                                    silence_threshold
                                                );
                                                if let Err(e) = fs::remove_file(&file_path) {
                                                    eprintln!("Error deleting silent file: {}", e);
                                                }
                                            }
                                            Ok(false) => {
                                                println!("Recording is not silent (above threshold {}), keeping file", silence_threshold);
                                            }
                                            Err(e) => {
                                                eprintln!("Error checking for silence: {}", e);
                                            }
                                        }
                                    }
                                }

                                // Create new files for the next recording period
                                match output_mode.as_str() {
                                    "split" => {
                                        // Create a WAV writer for each channel
                                        for (idx, &channel) in channels.iter().enumerate() {
                                            let now: DateTime<Local> = Local::now();
                                            let channel_file_name = format!(
                                                "{}/{}-{:02}-{:02}-{:02}-{:02}-ch{}.wav",
                                                output_dir,
                                                now.year(),
                                                now.month(),
                                                now.day(),
                                                now.hour(),
                                                now.minute(),
                                                channel
                                            );

                                            let spec = hound::WavSpec {
                                                channels: 1, // Mono for each individual channel
                                                sample_rate,
                                                bits_per_sample: 16,
                                                sample_format: hound::SampleFormat::Int,
                                            };

                                            match hound::WavWriter::create(&channel_file_name, spec) {
                                                Ok(writer) => {
                                                    writers[idx] = Some(writer);
                                                    println!("Created channel WAV file: {}", channel_file_name);
                                                },
                                                Err(e) => {
                                                    eprintln!("Failed to create channel WAV file: {}", e);
                                                }
                                            }
                                        }
                                    }
                                    "single" if channels.len() > 2 => {
                                        let now: DateTime<Local> = Local::now();
                                        let multichannel_file_name = format!(
                                            "{}/{}-{:02}-{:02}-{:02}-{:02}-multichannel.wav",
                                            output_dir,
                                            now.year(),
                                            now.month(),
                                            now.day(),
                                            now.hour(),
                                            now.minute()
                                        );

                                        let spec = hound::WavSpec {
                                            channels: channels.len() as u16,
                                            sample_rate,
                                            bits_per_sample: 16,
                                            sample_format: hound::SampleFormat::Int,
                                        };

                                        match hound::WavWriter::create(&multichannel_file_name, spec) {
                                            Ok(writer) => {
                                                *writer_for_rotation.lock().unwrap() = Some(writer);
                                                println!("Created multichannel WAV file: {}", multichannel_file_name);
                                            },
                                            Err(e) => {
                                                eprintln!("Failed to create multichannel WAV file: {}", e);
                                            }
                                        }
                                    }
                                    _ => {
                                        // Standard stereo mode
                                        let now: DateTime<Local> = Local::now();
                                        let file_name = format!(
                                            "{}-{:02}-{:02}-{:02}-{:02}.wav",
                                            now.year(),
                                            now.month(),
                                            now.day(),
                                            now.hour(),
                                            now.minute()
                                        );

                                        let full_path = format!("{}/{}", output_dir, file_name);

                                        if let Some(spec) = &*current_spec.lock().unwrap() {
                                            match hound::WavWriter::create(&full_path, *spec) {
                                                Ok(writer) => {
                                                    *writer_for_rotation.lock().unwrap() = Some(writer);
                                                    // We can't update self.file_name directly in this context
                                                    println!("Created new recording file: {}", full_path);
                                                },
                                                Err(e) => {
                                                    eprintln!("Failed to create new WAV file: {}", e);
                                                }
                                            }
                                        }
                                    }
                                }

                                // Reset the rotation timer
                                *last_rotation_time_clone.lock().unwrap() = Instant::now();
                            }
                        }

                        // Process the audio data based on the output mode
                        match output_mode_owned.as_str() {
                            "split" => {
                                // Split mode: write each channel to its own file
                                let mut writers = multichannel_writers_clone.lock().unwrap();
                                let mut buffers = multichannel_buffers_clone.lock().unwrap();

                                // Process each frame (a frame contains one sample for each channel)
                                let frame_size = total_channels;
                                let frames = data.chunks(frame_size);

                                for frame in frames {
                                    // Extract and write the selected channels
                                    for (idx, &channel) in channels_owned.iter().enumerate() {
                                        if channel < frame.len() {
                                            if let Some(writer) = &mut writers[idx] {
                                                // Convert f32 to i16 range
                                                let sample = (frame[channel] * 32767.0) as i32;
                                                let _ = writer.write_sample(sample);

                                                // Also store in the buffer for later processing
                                                if idx < buffers.len() {
                                                    buffers[idx].push(sample);
                                                    if buffers[idx].len() >= INTERMEDIATE_BUFFER_SIZE {
                                                        buffers[idx].clear();
                                                    }
                                                }
                                            }
                                        }
                                    }
                                }
                            }
                            "single" if channels_owned.len() > 2 => {
                                // Multichannel mode: write all selected channels to one file
                                let mut writer_guard = writer_clone.lock().unwrap();
                                if let Some(writer) = &mut *writer_guard {
                                    // Process each frame
                                    let frame_size = total_channels;
                                    let frames = data.chunks(frame_size);

                                    for frame in frames {
                                        // Write only the selected channels
                                        for &channel in &channels_owned {
                                            if channel < frame.len() {
                                                // Convert f32 to i16 range
                                                let sample = (frame[channel] * 32767.0) as i32;
                                                let _ = writer.write_sample(sample);
                                            }
                                        }
                                    }
                                }
                            }
                            _ => {
                                // Default stereo mode
                                let mut writer_guard = writer_clone.lock().unwrap();
                                let mut buffer = buffer_clone.lock().unwrap();

                                if let Some(writer) = &mut *writer_guard {
                                    // Process each frame
                                    let frame_size = total_channels;
                                    let frames = data.chunks(frame_size);

                                    for frame in frames {
                                        // Handle both mono and stereo inputs appropriately
                                        if frame.len() >= 2 {
                                            // Stereo input - convert f32 to i16 range and write to the stereo file
                                            let left_i16 = (frame[0] * 32767.0) as i32;
                                            let right_i16 = (frame[1] * 32767.0) as i32;

                                            let _ = writer.write_sample(left_i16);
                                            let _ = writer.write_sample(right_i16);

                                            // Also store in the buffer for later processing
                                            buffer.push(left_i16);
                                            buffer.push(right_i16);
                                        } else if frame.len() == 1 {
                                            // Mono input - duplicate the single channel to create stereo
                                            let sample_i16 = (frame[0] * 32767.0) as i32;

                                            // Write the same sample to both left and right channels
                                            let _ = writer.write_sample(sample_i16);
                                            let _ = writer.write_sample(sample_i16);

                                            // Also store in the buffer for later processing
                                            buffer.push(sample_i16);
                                            buffer.push(sample_i16);
                                        } else {
                                            eprintln!("Empty frame encountered");
                                        }

                                        if buffer.len() >= INTERMEDIATE_BUFFER_SIZE {
                                            buffer.clear();
                                        }
                                    }
                                }
                            }
                        }
                    },
                    err_fn,
                    None,
                ).expect("Failed to build input stream")
            }
            // Similar implementations for I16 and U16 would follow the same pattern
            // ... other formats ...
            _ => panic!("Unsupported sample format"),
        };

        // Start recording
        stream.play().expect("Failed to play stream");

        // Store the stream to keep it alive during recording
        self.stream = Some(Box::new(stream));

        // In continuous mode, initialize the rotation timer
        if self.continuous_mode {
            *self.last_rotation_time.lock().unwrap() = Instant::now();
        }
    }

    fn finalize(&mut self) -> std::io::Result<()> {
        // Load configuration
        let config = AppConfig::load();
        let silence_threshold = config.get_silence_threshold();

        // Get the file path before finalizing
        let file_path = self.file_name.lock().unwrap().clone();
        let channels = self.channels.lock().unwrap().clone();

        // Store paths of files being finalized to check for silence later
        let mut created_files = Vec::new();

        // Finalize the WAV file first
        if let Some(writer) = self.writer.lock().unwrap().take() {
            // Finalize the writer
            if let Err(e) = writer.finalize() {
                eprintln!("Error finalizing WAV file: {}", e);
                return Err(std::io::Error::other(e.to_string()));
            }
            created_files.push(file_path.clone());
        }

        // Finalize any multichannel writers
        let mut writers = self.multichannel_writers.lock().unwrap();
        for (idx, writer_opt) in writers.iter_mut().enumerate() {
            if let Some(writer) = writer_opt.take() {
                let now: DateTime<Local> = Local::now();
                let file_path = format!(
                    "{}/{}-{:02}-{:02}-{:02}-{:02}-ch{}.wav",
                    self.output_dir,
                    now.year(),
                    now.month(),
                    now.day(),
                    now.hour(),
                    now.minute(),
                    channels.get(idx).unwrap_or(&idx)
                );

                created_files.push(file_path.clone());

                if let Err(e) = writer.finalize() {
                    eprintln!("Error finalizing channel WAV file: {}", e);
                    return Err(std::io::Error::other(e.to_string()));
                }
            }
        }

        // Then close the stream
        self.stream = None;

        println!("Finalized recording to {}", file_path);

        // Check if the files are in the output directory and move them if needed
        for file_path in &created_files {
            let path = Path::new(file_path);
            if let Some(file_name) = path.file_name() {
                if let Some(file_name_str) = file_name.to_str() {
                    if let Some(parent_dir) = path.parent() {
                        if let Some(parent_dir_str) = parent_dir.to_str() {
                            // If the file is not in the output directory, move it there
                            if parent_dir_str != self.output_dir {
                                let new_path = format!("{}/{}", self.output_dir, file_name_str);
                                println!("Moving file from {} to {}", file_path, new_path);
                                fs::rename(file_path, &new_path)?;
                            }
                        }
                    }
                }
            }
        }

        // Check if we should apply silence detection
        if silence_threshold > 0.0 {
            // Check if each file is silent
            for file_path in created_files {
                match is_silent(&file_path, silence_threshold) {
                    Ok(true) => {
                        println!(
                            "Recording is silent (below threshold {}), deleting file: {}",
                            silence_threshold, file_path
                        );
                        if let Err(e) = fs::remove_file(&file_path) {
                            eprintln!("Error deleting silent file: {}", e);
                            return Err(std::io::Error::other(e.to_string()));
                        }
                    }
                    Ok(false) => {
                        println!(
                            "Recording is not silent (above threshold {}), keeping file: {}",
                            silence_threshold, file_path
                        );
                    }
                    Err(e) => {
                        eprintln!("Error checking for silence: {}", e);
                        return Err(std::io::Error::other(e));
                    }
                }
            }
        }

        Ok(())
    }

    fn start_recording(&mut self) -> std::io::Result<()> {
        // Get configuration from the AudioRecorder
        let config = AppConfig::load();
        let channels_str = config.get_audio_channels();
        let channels = match parse_channel_string(&channels_str) {
            Ok(chs) => chs,
            Err(e) => return Err(std::io::Error::new(std::io::ErrorKind::InvalidInput, e)),
        };

        let output_mode = config.get_output_mode();
        let debug = config.get_debug();

        // Start the audio processing
        self.process_audio(&channels, &output_mode, debug);
        Ok(())
    }

    fn stop_recording(&mut self) -> std::io::Result<()> {
        // Just call finalize to stop the recording
        self.finalize()
    }

    fn is_recording(&self) -> bool {
        // If there's an active stream, we're recording
        self.stream.is_some()
    }
}

// Add Drop implementation to ensure cleanup
impl Drop for CpalAudioProcessor {
    fn drop(&mut self) {
        // Try to finalize if we're still recording
        if self.is_recording() {
            if let Err(e) = self.finalize() {
                eprintln!("Error during cleanup: {}", e);
            }
        }
    }
}
