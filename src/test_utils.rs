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
        let freq = (ch as f32).mul_add(110.0, 440.0);
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
