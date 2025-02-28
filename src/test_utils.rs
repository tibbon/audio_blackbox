use cpal::{
    traits::{DeviceTrait, HostTrait, StreamTrait},
    BuildStreamError, InputCallbackInfo, SampleFormat, StreamConfig, SupportedStreamConfig,
    SampleRate
};
use std::sync::{Arc, Mutex};

// Mock structures to simulate CPAL functionality
pub struct MockStream {
    pub playing: Arc<Mutex<bool>>,
}

impl StreamTrait for MockStream {
    fn play(&self) -> Result<(), cpal::PlayStreamError> {
        let mut playing = self.playing.lock().unwrap();
        *playing = true;
        Ok(())
    }

    fn pause(&self) -> Result<(), cpal::PauseStreamError> {
        let mut playing = self.playing.lock().unwrap();
        *playing = false;
        Ok(())
    }
}

pub struct MockDevice {
    pub name: String,
    pub config: SupportedStreamConfig,
    pub sample_format: SampleFormat,
    pub channels: u16,
    pub sample_data: Vec<f32>, // For F32 format
    pub sample_data_i16: Vec<i16>, // For I16 format
    pub sample_data_u16: Vec<u16>, // For U16 format
}

impl MockDevice {
    pub fn new(
        name: &str, 
        sample_format: SampleFormat, 
        channels: u16, 
        sample_rate: u32
    ) -> Self {
        let config = SupportedStreamConfig::new(
            channels,
            SampleRate(sample_rate),
            cpal::SupportedBufferSize::Range { min: 15, max: 4096 },
            sample_format,
        );

        // Generate sample audio data based on format
        let sample_data = generate_test_audio_f32(channels, 1024);
        let sample_data_i16 = generate_test_audio_i16(channels, 1024);
        let sample_data_u16 = generate_test_audio_u16(channels, 1024);

        MockDevice {
            name: name.to_string(),
            config,
            sample_format,
            channels,
            sample_data,
            sample_data_i16,
            sample_data_u16,
        }
    }
}

impl DeviceTrait for MockDevice {
    fn name(&self) -> Result<String, cpal::DeviceNameError> {
        Ok(self.name.clone())
    }

    fn supported_input_configs(
        &self,
    ) -> Result<std::vec::Vec<cpal::SupportedStreamConfig>, cpal::SupportedStreamConfigsError> {
        Ok(vec![self.config.clone()])
    }

    fn supported_output_configs(
        &self,
    ) -> Result<std::vec::Vec<cpal::SupportedStreamConfig>, cpal::SupportedStreamConfigsError> {
        Ok(vec![self.config.clone()])
    }

    fn default_input_config(&self) -> Result<cpal::SupportedStreamConfig, cpal::DefaultStreamConfigError> {
        Ok(self.config.clone())
    }

    fn default_output_config(&self) -> Result<cpal::SupportedStreamConfig, cpal::DefaultStreamConfigError> {
        Ok(self.config.clone())
    }

    fn build_input_stream<D, E>(
        &self,
        config: &StreamConfig,
        data_callback: D,
        error_callback: E,
        _: Option<std::time::Duration>,
    ) -> Result<Box<dyn StreamTrait>, BuildStreamError>
    where
        D: FnMut(&[f32], &InputCallbackInfo) + Send + 'static,
        E: FnMut(cpal::StreamError) + Send + 'static,
    {
        // Create a stream that's "playing"
        let playing = Arc::new(Mutex::new(false));
        let stream = MockStream { playing: playing.clone() };

        // Execute the callback with our test data based on the format
        if self.sample_format == SampleFormat::F32 {
            let mut callback = data_callback;
            let info = InputCallbackInfo {};
            callback(&self.sample_data, &info);
        }

        Ok(Box::new(stream) as Box<dyn StreamTrait>)
    }

    fn build_input_stream_raw<D, E>(
        &self,
        _: &cpal::StreamConfig,
        _: cpal::SampleFormat,
        _: D,
        _: E,
        _: Option<std::time::Duration>,
    ) -> Result<Box<dyn StreamTrait>, BuildStreamError>
    where
        D: FnMut(&[u8], &InputCallbackInfo) + Send + 'static,
        E: FnMut(cpal::StreamError) + Send + 'static,
    {
        unimplemented!("Raw stream not implemented for tests")
    }

    fn build_output_stream<D, E>(
        &self,
        _: &cpal::StreamConfig,
        _: D,
        _: E,
        _: Option<std::time::Duration>,
    ) -> Result<Box<dyn StreamTrait>, BuildStreamError>
    where
        D: FnMut(&mut [f32], &cpal::OutputCallbackInfo) + Send + 'static,
        E: FnMut(cpal::StreamError) + Send + 'static,
    {
        unimplemented!("Output stream not implemented for tests")
    }

    fn build_output_stream_raw<D, E>(
        &self,
        _: &cpal::StreamConfig,
        _: cpal::SampleFormat,
        _: D,
        _: E,
        _: Option<std::time::Duration>,
    ) -> Result<Box<dyn StreamTrait>, BuildStreamError>
    where
        D: FnMut(&mut [u8], &cpal::OutputCallbackInfo) + Send + 'static,
        E: FnMut(cpal::StreamError) + Send + 'static,
    {
        unimplemented!("Raw output stream not implemented for tests")
    }
}

pub struct MockHost {
    pub device: Arc<MockDevice>,
}

impl MockHost {
    pub fn new(device: MockDevice) -> Self {
        MockHost { 
            device: Arc::new(device) 
        }
    }
}

impl HostTrait for MockHost {
    type Device = MockDevice;

    fn is_available() -> bool {
        true
    }

    fn devices(&self) -> Result<Vec<Self::Device>, cpal::DevicesError> {
        Ok(vec![(*self.device).clone()])
    }

    fn default_input_device(&self) -> Option<Self::Device> {
        Some((*self.device).clone())
    }

    fn default_output_device(&self) -> Option<Self::Device> {
        Some((*self.device).clone())
    }
}

// Helper functions to generate test audio data
fn generate_test_audio_f32(channels: u16, samples_per_channel: usize) -> Vec<f32> {
    let mut data = Vec::with_capacity(channels as usize * samples_per_channel);
    for i in 0..(channels as usize * samples_per_channel) {
        // Generate a sine wave
        let value = (i as f32 / 10.0).sin() * 0.5;
        data.push(value);
    }
    data
}

fn generate_test_audio_i16(channels: u16, samples_per_channel: usize) -> Vec<i16> {
    let mut data = Vec::with_capacity(channels as usize * samples_per_channel);
    for i in 0..(channels as usize * samples_per_channel) {
        // Generate a sine wave scaled to i16 range
        let value = ((i as f32 / 10.0).sin() * 0.5 * std::i16::MAX as f32) as i16;
        data.push(value);
    }
    data
}

fn generate_test_audio_u16(channels: u16, samples_per_channel: usize) -> Vec<u16> {
    let mut data = Vec::with_capacity(channels as usize * samples_per_channel);
    for i in 0..(channels as usize * samples_per_channel) {
        // Generate a sine wave scaled and shifted to u16 range
        let value = (((i as f32 / 10.0).sin() * 0.5 + 0.5) * std::u16::MAX as f32) as u16;
        data.push(value);
    }
    data
}

// Custom build_input_stream implementations for I16 and U16 data
impl MockDevice {
    pub fn build_input_stream_i16<D, E>(
        &self,
        config: &StreamConfig,
        mut data_callback: D,
        error_callback: E,
        _: Option<std::time::Duration>,
    ) -> Result<Box<dyn StreamTrait>, BuildStreamError>
    where
        D: FnMut(&[i16], &InputCallbackInfo) + Send + 'static,
        E: FnMut(cpal::StreamError) + Send + 'static,
    {
        // Create a stream that's "playing"
        let playing = Arc::new(Mutex::new(false));
        let stream = MockStream { playing };

        // Execute the callback with our i16 test data
        let info = InputCallbackInfo {};
        data_callback(&self.sample_data_i16, &info);

        Ok(Box::new(stream))
    }

    pub fn build_input_stream_u16<D, E>(
        &self,
        config: &StreamConfig,
        mut data_callback: D,
        error_callback: E,
        _: Option<std::time::Duration>,
    ) -> Result<Box<dyn StreamTrait>, BuildStreamError>
    where
        D: FnMut(&[u16], &InputCallbackInfo) + Send + 'static,
        E: FnMut(cpal::StreamError) + Send + 'static,
    {
        // Create a stream that's "playing"
        let playing = Arc::new(Mutex::new(false));
        let stream = MockStream { playing };

        // Execute the callback with our u16 test data
        let info = InputCallbackInfo {};
        data_callback(&self.sample_data_u16, &info);

        Ok(Box::new(stream))
    }
} 