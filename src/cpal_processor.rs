use chrono::prelude::*;
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::SampleFormat;
use std::env;
use std::fs;
use std::sync::{Arc, Mutex};

use crate::audio_processor::AudioProcessor;
use crate::constants::{
    MultiChannelWriters, WavWriterType, INTERMEDIATE_BUFFER_SIZE, MAX_CHANNELS,
};
use crate::utils::{check_alsa_availability, is_silent};

/// CpalAudioProcessor handles recording from audio devices using the CPAL library,
/// and saving the audio data to WAV files.
pub struct CpalAudioProcessor {
    file_name: String,
    writer: Arc<Mutex<Option<WavWriterType>>>,
    multichannel_writers: MultiChannelWriters,
    intermediate_buffer: Arc<Mutex<Vec<i32>>>,
    multichannel_buffers: Arc<Mutex<Vec<Vec<i32>>>>,
    #[allow(dead_code)]
    sample_rate: u32, // Kept for future features that might use it
    // Add a field to keep the stream alive
    #[allow(dead_code)]
    stream: Option<Box<dyn StreamTrait>>,
}

impl CpalAudioProcessor {
    /// Create a new CpalAudioProcessor instance.
    ///
    /// This sets up the recording environment, including WAV file writers
    /// and audio stream configuration.
    pub fn new() -> Result<Self, String> {
        // Check if ALSA is available on Linux
        check_alsa_availability()?;

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

        let config = device
            .default_input_config()
            .map_err(|e| format!("Failed to get default input stream config: {}", e))?;

        println!("Default input stream config: {:?}", config);

        let sample_rate = config.sample_rate().0;

        let spec = hound::WavSpec {
            channels: 2, // Default is stereo WAV for backward compatibility
            sample_rate,
            bits_per_sample: 16,
            sample_format: hound::SampleFormat::Int,
        };

        let writer = Arc::new(Mutex::new(Some(
            hound::WavWriter::create(&file_name, spec)
                .map_err(|e| format!("Failed to create WAV file: {}", e))?,
        )));

        Ok(CpalAudioProcessor {
            file_name,
            writer,
            multichannel_writers: Arc::new(Mutex::new(Vec::new())),
            intermediate_buffer: Arc::new(Mutex::new(Vec::with_capacity(INTERMEDIATE_BUFFER_SIZE))),
            multichannel_buffers: Arc::new(Mutex::new(Vec::new())),
            sample_rate,
            stream: None,
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
            let channel_file_name = format!("{}-ch{}.wav", date_str, channel);

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

        let multichannel_file_name = format!("{}-multichannel.wav", date_str);

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

        println!("Created multichannel WAV file: {}", multichannel_file_name);
        Ok(())
    }
}

impl AudioProcessor for CpalAudioProcessor {
    fn process_audio(&mut self, channels: &[usize], output_mode: &str, debug: bool) {
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

        // Validate channels
        for &channel in channels {
            if channel >= total_channels {
                panic!("The audio device does not have channel {}", channel);
            }
        }

        // Setup the appropriate output mode
        match output_mode {
            "split" => {
                if let Err(e) = self.setup_split_mode(channels, sample_rate) {
                    panic!("Failed to setup split mode: {}", e);
                }
            }
            "single" if channels.len() > 2 => {
                if let Err(e) = self.setup_multichannel_mode(channels, sample_rate) {
                    panic!("Failed to setup multichannel mode: {}", e);
                }
            }
            _ => {
                // Use the default stereo WAV for backward compatibility
                println!("Using standard stereo WAV output format");
            }
        }

        // Clone channels to own them in the closure
        let channels_owned: Vec<usize> = channels.to_vec();
        let output_mode_owned = output_mode.to_string();

        let writer_clone = Arc::clone(&self.writer);
        let buffer_clone = Arc::clone(&self.intermediate_buffer);
        let multichannel_writers_clone = Arc::clone(&self.multichannel_writers);
        let multichannel_buffers_clone = Arc::clone(&self.multichannel_buffers);

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
                                        // For stereo output, use the first two channels or duplicate mono

                                        if frame.len() >= 2 {
                                            // Convert f32 to i16 range and write to the stereo file
                                            let left_i16 = (frame[0] * 32767.0) as i32;
                                            let right_i16 = (frame[1] * 32767.0) as i32;

                                            let _ = writer.write_sample(left_i16);
                                            let _ = writer.write_sample(right_i16);

                                            // Also store in the buffer for later processing
                                            buffer.push(left_i16);
                                            buffer.push(right_i16);

                                            if buffer.len() >= INTERMEDIATE_BUFFER_SIZE {
                                                buffer.clear();
                                            }
                                        } else {
                                            eprintln!("Buffer too small: expected at least {} channels, found {}", channels_owned.len(), frame.len());
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
    }

    fn finalize(&mut self) {
        // Close the stream to stop recording
        self.stream = None;

        // Finalize the WAV file
        if let Some(writer) = self.writer.lock().unwrap().take() {
            // Get the file path before finalizing
            let file_path = self.file_name.clone();

            // Finalize the writer
            if let Err(e) = writer.finalize() {
                eprintln!("Error finalizing WAV file: {}", e);
            } else {
                println!("Finalized recording to {}", file_path);

                // Check if we should apply silence detection
                if let Ok(threshold) = env::var("SILENCE_THRESHOLD") {
                    if let Ok(threshold) = threshold.parse::<i32>() {
                        if threshold > 0 {
                            // Check if the file is silent
                            match is_silent(&file_path, threshold) {
                                Ok(true) => {
                                    println!(
                                        "Recording is silent (below threshold {}), deleting file",
                                        threshold
                                    );
                                    if let Err(e) = fs::remove_file(&file_path) {
                                        eprintln!("Error deleting silent file: {}", e);
                                    }
                                }
                                Ok(false) => {
                                    println!("Recording is not silent (above threshold {}), keeping file", threshold);
                                }
                                Err(e) => {
                                    eprintln!("Error checking for silence: {}", e);
                                }
                            }
                        }
                    }
                }
            }
        }

        // Finalize any multichannel writers
        let mut writers = self.multichannel_writers.lock().unwrap();
        for (idx, writer_opt) in writers.iter_mut().enumerate() {
            if let Some(writer) = writer_opt.take() {
                // Get the file path - we'll use the file_name pattern since we can't directly access the path
                let now: DateTime<Local> = Local::now();
                let file_path = format!(
                    "{}-{:02}-{:02}-{:02}-{:02}-ch{}.wav",
                    now.year(),
                    now.month(),
                    now.day(),
                    now.hour(),
                    now.minute(),
                    idx
                );

                // Finalize the writer
                if let Err(e) = writer.finalize() {
                    eprintln!("Error finalizing channel WAV file: {}", e);
                } else {
                    println!("Finalized recording to {}", file_path);

                    // Check if we should apply silence detection
                    if let Ok(threshold) = env::var("SILENCE_THRESHOLD") {
                        if let Ok(threshold) = threshold.parse::<i32>() {
                            if threshold > 0 {
                                // Check if the file is silent
                                match is_silent(&file_path, threshold) {
                                    Ok(true) => {
                                        println!("Channel recording is silent (below threshold {}), deleting file", threshold);
                                        if let Err(e) = fs::remove_file(&file_path) {
                                            eprintln!("Error deleting silent file: {}", e);
                                        }
                                    }
                                    Ok(false) => {
                                        println!("Channel recording is not silent (above threshold {}), keeping file", threshold);
                                    }
                                    Err(e) => {
                                        eprintln!("Error checking for silence: {}", e);
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}
