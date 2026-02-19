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
        let value = ((i as f32 / 10.0).sin() * 0.5 * i16::MAX as f32) as i16;
        data.push(value);
    }
    data
}

#[cfg(test)]
pub fn generate_test_audio_u16(channels: u16, samples_per_channel: usize) -> Vec<u16> {
    let mut data = Vec::with_capacity(channels as usize * samples_per_channel);
    for i in 0..(channels as usize * samples_per_channel) {
        // Generate a sine wave scaled and shifted to u16 range
        let value = ((i as f32 / 10.0).sin().mul_add(0.5, 0.5) * u16::MAX as f32) as u16;
        data.push(value);
    }
    data
}

/// Generate properly interleaved multi-channel f32 data.
///
/// Each channel in `channel_amplitudes` receives a sine wave at a distinct
/// frequency scaled by the given amplitude.  Channels not listed are silent.
/// Returns `total_channels * samples_per_channel` interleaved samples.
#[cfg(test)]
pub fn generate_interleaved_f32(
    total_channels: usize,
    samples_per_channel: usize,
    channel_amplitudes: &[(usize, f32)],
) -> Vec<f32> {
    let total = total_channels * samples_per_channel;
    let mut data = vec![0.0_f32; total];
    for &(ch, amp) in channel_amplitudes {
        // Give each channel a different frequency so they're distinguishable
        let freq = 440.0 + ch as f32 * 110.0;
        for frame in 0..samples_per_channel {
            let t = frame as f32 / 44100.0;
            data[frame * total_channels + ch] = (2.0 * std::f32::consts::PI * freq * t).sin() * amp;
        }
    }
    data
}

/// Generate interleaved f32 data that is all zeros (silence).
#[cfg(test)]
pub fn generate_silent_interleaved_f32(
    total_channels: usize,
    samples_per_channel: usize,
) -> Vec<f32> {
    vec![0.0_f32; total_channels * samples_per_channel]
}

/// Generate interleaved f32 data where `selected_channels` all get a sine
/// wave at the same `amplitude`.
#[cfg(test)]
pub fn generate_uniform_interleaved_f32(
    total_channels: usize,
    samples_per_channel: usize,
    selected_channels: &[usize],
    amplitude: f32,
) -> Vec<f32> {
    let pairs: Vec<(usize, f32)> = selected_channels
        .iter()
        .map(|&ch| (ch, amplitude))
        .collect();
    generate_interleaved_f32(total_channels, samples_per_channel, &pairs)
}
