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

/// Deterministic, advance-on-demand timestamp source for rotation tests.
///
/// Each call returns `tick-{N:03}` where `N` is the current value of an
/// internal atomic counter; tests bump the counter via `advance()` to make a
/// rotation produce a distinct filename without sleeping past a wall-clock
/// second.
///
/// The counter is shared between the test thread and the writer thread via
/// `Arc<AtomicU64>`, so the test can advance the clock at the moment it
/// signals a rotation.
#[cfg(test)]
#[derive(Clone, Default)]
pub struct MockClock {
    tick: std::sync::Arc<std::sync::atomic::AtomicU64>,
}

#[cfg(test)]
impl MockClock {
    pub fn new() -> Self {
        Self::default()
    }

    /// Advance the clock by one tick. Subsequent calls to the timestamp
    /// closure will produce a new string.
    pub fn advance(&self) {
        self.tick
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    }

    /// Return a `Send + Sync` closure suitable to pass to
    /// `WriterThreadState::set_timestamp_fn`.
    pub fn as_timestamp_fn(
        &self,
    ) -> std::sync::Arc<dyn Fn() -> String + Send + Sync> {
        let tick = std::sync::Arc::clone(&self.tick);
        std::sync::Arc::new(move || {
            let n = tick.load(std::sync::atomic::Ordering::Relaxed);
            format!("tick-{n:03}")
        })
    }
}
