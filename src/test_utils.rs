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

/// Block until `counter` reaches at least `target`, returning the final value.
///
/// Used by rotation tests to rendezvous on "writer has consumed N samples"
/// instead of `thread::sleep` for an arbitrary "long enough" duration
/// (DOLL-127).
///
/// Panics if the timeout elapses before the counter reaches target —
/// the caller's expectation is that the writer thread WILL drain the
/// pushed samples, so a timeout indicates a real test failure
/// (writer wedged, lost samples, etc.) rather than a flake.
#[cfg(test)]
pub fn wait_for_samples_consumed(
    counter: &std::sync::atomic::AtomicU64,
    target: u64,
    timeout: std::time::Duration,
) -> u64 {
    let start = std::time::Instant::now();
    loop {
        let n = counter.load(std::sync::atomic::Ordering::Relaxed);
        if n >= target {
            return n;
        }
        assert!(
            start.elapsed() < timeout,
            "writer thread did not consume {target} samples within {timeout:?} \
             (final count: {n})"
        );
        // 1ms poll keeps the test responsive without busy-waiting.
        std::thread::sleep(std::time::Duration::from_millis(1));
    }
}

/// Block until `flag` is cleared (becomes `false`).
///
/// Used by rotation tests to wait for the writer thread to acknowledge a
/// rotation request (the writer flips `rotation_needed` back to `false`
/// after rotating). Replaces a fixed `thread::sleep` rendezvous (DOLL-127).
#[cfg(test)]
pub fn wait_for_flag_cleared(flag: &std::sync::atomic::AtomicBool, timeout: std::time::Duration) {
    let start = std::time::Instant::now();
    while flag.load(std::sync::atomic::Ordering::Relaxed) {
        assert!(
            start.elapsed() < timeout,
            "flag did not clear within {timeout:?}"
        );
        std::thread::sleep(std::time::Duration::from_millis(1));
    }
}

/// Standard set of env-var overrides to isolate tests from the host
/// environment. Sets the `*` (unprefixed) keys to defaults and clears
/// every `BLACKBOX_*` override. Use with `temp_env::with_vars(...)`.
///
/// Consolidated here from previously duplicated copies in lib.rs +
/// recorder_tests.rs (DOLL-118).
#[cfg(test)]
pub fn default_test_env() -> Vec<(&'static str, Option<&'static str>)> {
    use crate::constants::{DEFAULT_CHANNELS, DEFAULT_OUTPUT_DIR, DEFAULT_OUTPUT_MODE};
    vec![
        ("AUDIO_CHANNELS", Some(DEFAULT_CHANNELS)),
        ("DEBUG", Some("false")),
        ("RECORD_DURATION", Some("30")),
        ("OUTPUT_MODE", Some(DEFAULT_OUTPUT_MODE)),
        ("SILENCE_THRESHOLD", Some("0.01")),
        ("CONTINUOUS_MODE", Some("false")),
        ("RECORDING_CADENCE", Some("300")),
        ("OUTPUT_DIR", Some(DEFAULT_OUTPUT_DIR)),
        ("PERFORMANCE_LOGGING", Some("false")),
        ("BLACKBOX_AUDIO_CHANNELS", None),
        ("BLACKBOX_DEBUG", None),
        ("BLACKBOX_DURATION", None),
        ("BLACKBOX_OUTPUT_MODE", None),
        ("BLACKBOX_SILENCE_THRESHOLD", None),
        ("BLACKBOX_CONTINUOUS_MODE", None),
        ("BLACKBOX_RECORDING_CADENCE", None),
        ("BLACKBOX_OUTPUT_DIR", None),
        ("BLACKBOX_PERFORMANCE_LOGGING", None),
        ("BLACKBOX_INPUT_DEVICE", None),
        ("INPUT_DEVICE", None),
        ("BLACKBOX_MIN_DISK_SPACE_MB", None),
        ("MIN_DISK_SPACE_MB", None),
        ("BLACKBOX_BITS_PER_SAMPLE", None),
        ("BITS_PER_SAMPLE", None),
        ("BLACKBOX_SILENCE_GATE_ENABLED", None),
        ("SILENCE_GATE_ENABLED", None),
        ("BLACKBOX_SILENCE_GATE_TIMEOUT_SECS", None),
        ("SILENCE_GATE_TIMEOUT_SECS", None),
        ("BLACKBOX_CONFIG", None),
    ]
}

/// `default_test_env()` with `SILENCE_THRESHOLD` set to `0` so the
/// silence-deletion code paths don't fire mid-test (DOLL-118).
#[cfg(test)]
pub fn test_env_no_silence() -> Vec<(&'static str, Option<&'static str>)> {
    let mut env = default_test_env();
    env.retain(|&(k, _)| k != "SILENCE_THRESHOLD");
    env.push(("SILENCE_THRESHOLD", Some("0")));
    env
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
        self.tick.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    }

    /// Return a `Send + Sync` closure suitable to pass to
    /// `WriterThreadState::set_timestamp_fn`.
    pub fn as_timestamp_fn(&self) -> std::sync::Arc<dyn Fn() -> String + Send + Sync> {
        let tick = std::sync::Arc::clone(&self.tick);
        std::sync::Arc::new(move || {
            let n = tick.load(std::sync::atomic::Ordering::Relaxed);
            format!("tick-{n:03}")
        })
    }
}
