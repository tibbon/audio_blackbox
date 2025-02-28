#[cfg(test)]
use cpal::SampleFormat;

// Simplified mock structures for testing
#[cfg(test)]
pub struct MockStream {
    pub playing: bool,
}

#[cfg(test)]
#[derive(Clone)]
pub struct MockDevice {
    pub name: String,
    pub sample_format: SampleFormat,
    pub channels: u16,
    pub sample_rate: u32,
}

#[cfg(test)]
impl MockDevice {
    pub fn new(name: &str, sample_format: SampleFormat, channels: u16, sample_rate: u32) -> Self {
        MockDevice {
            name: name.to_string(),
            sample_format,
            channels,
            sample_rate,
        }
    }
}

#[cfg(test)]
pub struct MockHost {
    pub device: MockDevice,
}

#[cfg(test)]
impl MockHost {
    pub fn new(device: MockDevice) -> Self {
        MockHost { device }
    }
}

// Helper functions to generate test audio data
#[cfg(test)]
pub fn generate_test_audio_f32(channels: u16, samples_per_channel: usize) -> Vec<f32> {
    let mut data = Vec::with_capacity(channels as usize * samples_per_channel);
    for i in 0..(channels as usize * samples_per_channel) {
        // Generate a sine wave
        let value = (i as f32 / 10.0).sin() * 0.5;
        data.push(value);
    }
    data
}

#[cfg(test)]
pub fn generate_test_audio_i16(channels: u16, samples_per_channel: usize) -> Vec<i16> {
    let mut data = Vec::with_capacity(channels as usize * samples_per_channel);
    for i in 0..(channels as usize * samples_per_channel) {
        // Generate a sine wave scaled to i16 range
        let value = ((i as f32 / 10.0).sin() * 0.5 * std::i16::MAX as f32) as i16;
        data.push(value);
    }
    data
}

#[cfg(test)]
pub fn generate_test_audio_u16(channels: u16, samples_per_channel: usize) -> Vec<u16> {
    let mut data = Vec::with_capacity(channels as usize * samples_per_channel);
    for i in 0..(channels as usize * samples_per_channel) {
        // Generate a sine wave scaled and shifted to u16 range
        let value = (((i as f32 / 10.0).sin() * 0.5 + 0.5) * std::u16::MAX as f32) as u16;
        data.push(value);
    }
    data
}
