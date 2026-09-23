//! Configuration loading and validation.
//!
//! **Precedence**: env vars override TOML, TOML overrides defaults.
//! Inside the env-var tier, `BLACKBOX_*`-prefixed names take precedence
//! over the unprefixed legacy names (e.g. `BLACKBOX_DURATION` wins over
//! `RECORD_DURATION`). See `apply_env_vars` for the full map.
//!
//! **Forgiving validation**: bad TOML, unparseable env vars, and
//! out-of-range numerics log a warning and fall back to defaults rather
//! than surfacing an error. This is deliberate — the App Store-shipped
//! product runs with whatever it can parse, never aborts on bad config.
//! If you need strict validation in the future, reintroduce
//! `BlackboxError::Config(_)` (deleted in DOLL-189) and produce it from
//! `AppConfig::load` / `apply_env_vars`.

use log::{error, info, warn};
use serde::{Deserialize, Serialize};
use std::env;
use std::fs;
use std::path::{Component, Path, PathBuf};

use crate::constants::{
    DEFAULT_BITS_PER_SAMPLE, DEFAULT_CHANNELS, DEFAULT_CONTINUOUS_MODE, DEFAULT_DEBUG,
    DEFAULT_DURATION, DEFAULT_MIN_DISK_SPACE_MB, DEFAULT_OUTPUT_DIR, DEFAULT_OUTPUT_MODE,
    DEFAULT_PERFORMANCE_LOGGING, DEFAULT_RECORDING_CADENCE, DEFAULT_SILENCE_GATE_ENABLED,
    DEFAULT_SILENCE_GATE_TIMEOUT_SECS, DEFAULT_SILENCE_THRESHOLD, MAX_RECORDING_CADENCE,
    OutputMode,
};
use crate::error::BlackboxError;

/// The main configuration struct that holds all settings for the audio recorder.
///
/// This structure can be initialized from environment variables, a TOML file,
/// or with default values. Values are resolved with environment variables having
/// the highest precedence, followed by the config file, and then defaults.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[non_exhaustive]
pub struct AppConfig {
    /// Audio channels to record. String form (`"0"`, `"0,2-4,7"`)
    /// parsed by `parse_channel_string`. Channel indices are 0-based at
    /// the engine; the Swift UI displays 1-based and converts at the
    /// boundary. `None` falls back to `DEFAULT_CHANNELS` ("0").
    pub audio_channels: Option<String>,
    /// Enable debug output (extra log lines from the writer thread).
    /// `None` falls back to `DEFAULT_DEBUG` (false).
    pub debug: Option<bool>,
    /// Recording duration in seconds. `0` means unlimited (run until
    /// Ctrl-C / stop). `None` falls back to `DEFAULT_DURATION` (30).
    pub duration: Option<u64>,
    /// Output mode: `"single"` (one multichannel file) or `"split"`
    /// (one file per channel). Parsed via `OutputMode::parse` at the
    /// config boundary; downstream code uses the enum. `None` falls
    /// back to `DEFAULT_OUTPUT_MODE` ("single").
    pub output_mode: Option<String>,
    /// Silence-detection threshold as a **normalized amplitude fraction**
    /// in [0.0, 1.0] (e.g. `0.01` = 1% of full scale). `0.0` disables
    /// detection. Negative, NaN, ±Inf, and > 1.0 values are rejected by
    /// `get_silence_threshold`, which falls back to
    /// `DEFAULT_SILENCE_THRESHOLD` (they do **not** disable detection;
    /// a super-unity threshold would classify everything as silent —
    /// DOLL-445). `None` also falls back to `DEFAULT_SILENCE_THRESHOLD`.
    pub silence_threshold: Option<f32>,
    /// Enable continuous recording — file rotates at
    /// `recording_cadence` intervals. `None` falls back to
    /// `DEFAULT_CONTINUOUS_MODE` (false).
    pub continuous_mode: Option<bool>,
    /// File-rotation cadence in seconds (continuous mode only). Must be
    /// at least 1: `0` is rejected by `get_recording_cadence` and falls
    /// back to `DEFAULT_RECORDING_CADENCE` (300, i.e. 5 min) — a zero
    /// cadence would rotate on every audio callback (DOLL-458). So is
    /// anything above `MAX_RECORDING_CADENCE` (about 195 days), which would
    /// overflow the rotation threshold. `None` falls back the same way.
    pub recording_cadence: Option<u64>,
    /// Output directory for WAV files. Relative paths are resolved
    /// against the working directory. `None` falls back to
    /// `DEFAULT_OUTPUT_DIR` ("recordings").
    pub output_dir: Option<String>,
    /// Enable performance-metric collection (writes
    /// `performance.log` next to recordings). Requires the
    /// `benchmarking` feature. `None` falls back to
    /// `DEFAULT_PERFORMANCE_LOGGING` (false).
    pub performance_logging: Option<bool>,
    /// Input device name (cpal-reported). `None` = system default.
    pub input_device: Option<String>,
    /// Minimum free disk space in MB before stopping recording.
    /// `0` disables the check. `None` falls back to
    /// `DEFAULT_MIN_DISK_SPACE_MB`.
    pub min_disk_space_mb: Option<u64>,
    /// Bits per sample for WAV output. Valid values: 16, 24, 32.
    /// Anything else is rejected by `get_bits_per_sample` and falls
    /// back to `DEFAULT_BITS_PER_SAMPLE` (24). `None` falls back the
    /// same way.
    pub bits_per_sample: Option<u16>,
    /// Enable the silence gate: writer thread closes files during
    /// extended silence, reopens on signal. `None` falls back to
    /// `DEFAULT_SILENCE_GATE_ENABLED` (true).
    pub silence_gate_enabled: Option<bool>,
    /// Seconds of silence before the gate closes and finalizes the
    /// current open file. Must be > 0: `0` is rejected by
    /// `get_silence_gate_timeout_secs` and falls back to the default, like
    /// `None` does: `DEFAULT_SILENCE_GATE_TIMEOUT_SECS` (300).
    pub silence_gate_timeout_secs: Option<u64>,
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            audio_channels: Some(DEFAULT_CHANNELS.to_owned()),
            debug: Some(DEFAULT_DEBUG),
            duration: Some(DEFAULT_DURATION),
            output_mode: Some(DEFAULT_OUTPUT_MODE.to_owned()),
            silence_threshold: Some(DEFAULT_SILENCE_THRESHOLD),
            continuous_mode: Some(DEFAULT_CONTINUOUS_MODE),
            recording_cadence: Some(DEFAULT_RECORDING_CADENCE),
            output_dir: Some(DEFAULT_OUTPUT_DIR.to_owned()),
            performance_logging: Some(DEFAULT_PERFORMANCE_LOGGING),
            input_device: None,
            min_disk_space_mb: Some(DEFAULT_MIN_DISK_SPACE_MB),
            bits_per_sample: Some(DEFAULT_BITS_PER_SAMPLE),
            silence_gate_enabled: Some(DEFAULT_SILENCE_GATE_ENABLED),
            silence_gate_timeout_secs: Some(DEFAULT_SILENCE_GATE_TIMEOUT_SECS),
        }
    }
}

impl AppConfig {
    /// Create a new configuration with default values
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Find the configuration file [`load`](Self::load) reads: the
    /// `BLACKBOX_CONFIG` path if that file exists, else the first existing
    /// file in the search order below. `None` when there is none. An empty
    /// `BLACKBOX_CONFIG` counts as unset.
    #[must_use]
    pub fn find_config_file() -> Option<PathBuf> {
        // First check if a config file path is specified in the environment
        if let Ok(config_path) = env::var("BLACKBOX_CONFIG")
            && !config_path.is_empty()
        {
            let path = Path::new(&config_path);
            if path.exists() {
                return Some(path.to_path_buf());
            }
        }

        // Search order:
        // 1. Current directory: "./blackbox.toml"
        // 2. User's home directory: "~/.config/blackbox/config.toml"
        // 3. System config: "/etc/blackbox/config.toml"

        let current_dir = Path::new("blackbox.toml");
        if current_dir.exists() {
            return Some(current_dir.to_path_buf());
        }

        if let Ok(home) = env::var("HOME") {
            let home_config = Path::new(&home).join(".config/blackbox/config.toml");
            if home_config.exists() {
                return Some(home_config);
            }
        }

        // XDG Base Directory specification
        if let Ok(xdg_config) = env::var("XDG_CONFIG_HOME") {
            let xdg_config_path = Path::new(&xdg_config).join("blackbox/config.toml");
            if xdg_config_path.exists() {
                return Some(xdg_config_path);
            }
        }

        // System-wide configuration
        let system_config = Path::new("/etc/blackbox/config.toml");
        if system_config.exists() {
            return Some(system_config.to_path_buf());
        }

        None
    }

    /// Load configuration from file, if available
    #[must_use]
    pub fn load() -> Self {
        let mut config = Self::default();

        // Try to find and load the configuration file
        if let Some(config_path) = Self::find_config_file() {
            match fs::read_to_string(&config_path) {
                Ok(content) => match Self::from_toml_forgiving(&content) {
                    Ok((file_config, problems)) => {
                        info!("Loaded configuration from {}", config_path.display());
                        for problem in problems {
                            warn!("{}: {problem}", config_path.display());
                        }
                        config.merge(file_config);
                    }
                    Err(e) => {
                        error!("Error parsing config file: {e}");
                    }
                },
                Err(e) => {
                    error!("Error reading config file: {e}");
                }
            }
        }

        // Override with environment variables
        config.apply_env_vars();

        config
    }

    /// Parse a TOML configuration key by key, keeping every key that parses.
    ///
    /// Forgiving like the rest of loading: a key whose value has the wrong
    /// type or range for its field (`recording_cadence = -1`,
    /// `bits_per_sample = 70000`) is skipped, and so is an unknown key
    /// (usually a typo that leaves the intended setting at its default).
    /// Each skipped key comes back as a message to warn with. Parsing the
    /// whole file as one struct used to drop every setting in it over one
    /// bad value.
    ///
    /// Fails only when `content` isn't a TOML document at all.
    pub(crate) fn from_toml_forgiving(
        content: &str,
    ) -> Result<(Self, Vec<String>), toml::de::Error> {
        let table: toml::Table = content.parse()?;
        let mut config = Self {
            audio_channels: None,
            debug: None,
            duration: None,
            output_mode: None,
            silence_threshold: None,
            continuous_mode: None,
            recording_cadence: None,
            output_dir: None,
            performance_logging: None,
            input_device: None,
            min_disk_space_mb: None,
            bits_per_sample: None,
            silence_gate_enabled: None,
            silence_gate_timeout_secs: None,
        };
        let mut problems = Vec::new();
        for (key, value) in table {
            if !CONFIG_KEYS.contains(&key.as_str()) {
                problems.push(format!(
                    "unknown key `{key}` ignored (misspelled?); known keys: {}",
                    CONFIG_KEYS.join(", ")
                ));
                continue;
            }
            let mut single = toml::Table::new();
            single.insert(key.clone(), value);
            match toml::Value::Table(single).try_into::<Self>() {
                Ok(one) => config.merge(one),
                Err(e) => problems.push(format!(
                    "`{key}` ignored, its value is invalid ({}); using the default",
                    e.message()
                )),
            }
        }
        Ok((config, problems))
    }

    /// Merge another configuration into this one, only taking values that are Some
    pub fn merge(&mut self, other: Self) {
        if other.audio_channels.is_some() {
            self.audio_channels = other.audio_channels;
        }
        if other.debug.is_some() {
            self.debug = other.debug;
        }
        if other.duration.is_some() {
            self.duration = other.duration;
        }
        if other.output_mode.is_some() {
            self.output_mode = other.output_mode;
        }
        if other.silence_threshold.is_some() {
            self.silence_threshold = other.silence_threshold;
        }
        if other.continuous_mode.is_some() {
            self.continuous_mode = other.continuous_mode;
        }
        if other.recording_cadence.is_some() {
            self.recording_cadence = other.recording_cadence;
        }
        if other.output_dir.is_some() {
            self.output_dir = other.output_dir;
        }
        if other.performance_logging.is_some() {
            self.performance_logging = other.performance_logging;
        }
        if other.input_device.is_some() {
            self.input_device = other.input_device;
        }
        if other.min_disk_space_mb.is_some() {
            self.min_disk_space_mb = other.min_disk_space_mb;
        }
        if other.bits_per_sample.is_some() {
            self.bits_per_sample = other.bits_per_sample;
        }
        if other.silence_gate_enabled.is_some() {
            self.silence_gate_enabled = other.silence_gate_enabled;
        }
        if other.silence_gate_timeout_secs.is_some() {
            self.silence_gate_timeout_secs = other.silence_gate_timeout_secs;
        }
    }

    /// Parse a boolean value from a string
    fn parse_bool(val: &str) -> Option<bool> {
        match val.to_lowercase().as_str() {
            "true" | "1" | "yes" | "on" => Some(true),
            "false" | "0" | "no" | "off" => Some(false),
            _ => None,
        }
    }

    /// Apply environment variables to override configuration.
    ///
    /// Each setting reads its `BLACKBOX_` variable first and falls back to the
    /// legacy unprefixed name. A variable that is unset or doesn't parse
    /// leaves the value from the file.
    fn apply_env_vars(&mut self) {
        fn number<T: std::str::FromStr>(s: &str) -> Option<T> {
            s.parse().ok()
        }
        let text = |s: &str| Some(s.to_owned());
        let flag = Self::parse_bool;

        set_if_some(
            &mut self.audio_channels,
            env_override("BLACKBOX_AUDIO_CHANNELS", "AUDIO_CHANNELS", text),
        );
        set_if_some(
            &mut self.debug,
            env_override("BLACKBOX_DEBUG", "DEBUG", flag),
        );
        set_if_some(
            &mut self.duration,
            env_override("BLACKBOX_DURATION", "RECORD_DURATION", number),
        );
        set_if_some(
            &mut self.output_mode,
            env_override("BLACKBOX_OUTPUT_MODE", "OUTPUT_MODE", text),
        );
        set_if_some(
            &mut self.silence_threshold,
            env_override("BLACKBOX_SILENCE_THRESHOLD", "SILENCE_THRESHOLD", number),
        );
        set_if_some(
            &mut self.continuous_mode,
            env_override("BLACKBOX_CONTINUOUS_MODE", "CONTINUOUS_MODE", flag),
        );
        set_if_some(
            &mut self.recording_cadence,
            env_override("BLACKBOX_RECORDING_CADENCE", "RECORDING_CADENCE", number),
        );
        set_if_some(
            &mut self.output_dir,
            env_override("BLACKBOX_OUTPUT_DIR", "OUTPUT_DIR", text),
        );
        set_if_some(
            &mut self.performance_logging,
            env_override("BLACKBOX_PERFORMANCE_LOGGING", "PERFORMANCE_LOGGING", flag),
        );
        set_if_some(
            &mut self.input_device,
            env_override("BLACKBOX_INPUT_DEVICE", "INPUT_DEVICE", text),
        );
        set_if_some(
            &mut self.min_disk_space_mb,
            env_override("BLACKBOX_MIN_DISK_SPACE_MB", "MIN_DISK_SPACE_MB", number),
        );
        set_if_some(
            &mut self.bits_per_sample,
            env_override("BLACKBOX_BITS_PER_SAMPLE", "BITS_PER_SAMPLE", number),
        );
        set_if_some(
            &mut self.silence_gate_enabled,
            env_override(
                "BLACKBOX_SILENCE_GATE_ENABLED",
                "SILENCE_GATE_ENABLED",
                flag,
            ),
        );
        set_if_some(
            &mut self.silence_gate_timeout_secs,
            env_override(
                "BLACKBOX_SILENCE_GATE_TIMEOUT_SECS",
                "SILENCE_GATE_TIMEOUT_SECS",
                number,
            ),
        );
    }

    /// Generate a sample configuration file with comments
    #[must_use]
    pub fn generate_sample_config() -> String {
        let default_config = Self::default();

        // Create a string with comments and the default values
        let sample = format!(
            r#"# Blackbox Audio Recorder Configuration
# This file configures the behavior of the audio recorder.
# Values set here can be overridden by environment variables.

# Audio channels to record (comma-separated list or ranges like 0-2)
# Default: {DEFAULT_CHANNELS}
audio_channels = "{}"

# Enable debug output (true/false)
# Default: {DEFAULT_DEBUG}
debug = {}

# Recording duration in seconds (0 for unlimited)
# Default: {DEFAULT_DURATION}
duration = {}

# Output mode: "single" (one file), "split" (one file per channel)
# Default: {DEFAULT_OUTPUT_MODE}
output_mode = "{}"

# Silence threshold: normalized amplitude 0.0-1.0 (0.01 = 1% of full scale).
# 0 disables silence detection. Out-of-range values fall back to the default.
# Default: {DEFAULT_SILENCE_THRESHOLD}
silence_threshold = {}

# Continuous recording mode (true/false)
# Default: {DEFAULT_CONTINUOUS_MODE}
continuous_mode = {}

# Recording cadence in seconds (how often to rotate files in continuous mode).
# Must be 1 to {MAX_RECORDING_CADENCE}; other values fall back to the default.
# Default: {DEFAULT_RECORDING_CADENCE}
recording_cadence = {}

# Output directory for recordings
# Default: {DEFAULT_OUTPUT_DIR}
output_dir = "{}"

# Enable performance logging (true/false)
# Default: {DEFAULT_PERFORMANCE_LOGGING}
performance_logging = {}

# Bits per sample for WAV output (16, 24, or 32)
# Default: {DEFAULT_BITS_PER_SAMPLE}
bits_per_sample = {}

# Silence gate: auto-split on silence (finalize the current file and open a
# new one on next signal) (true/false)
# Default: {DEFAULT_SILENCE_GATE_ENABLED}
silence_gate_enabled = {}

# Seconds of silence before the gate closes and finalizes files.
# Must be >= 1; 0 falls back to the default.
# Default: {DEFAULT_SILENCE_GATE_TIMEOUT_SECS}
silence_gate_timeout_secs = {}

# Input device name (leave commented out for system default). The CLI exits
# with an error if the named device isn't present.
# input_device = "MacBook Pro Microphone"
"#,
            default_config.get_audio_channels(),
            default_config.get_debug(),
            default_config.get_duration(),
            default_config.get_output_mode(),
            default_config.get_silence_threshold(),
            default_config.get_continuous_mode(),
            default_config.get_recording_cadence(),
            default_config.get_output_dir(),
            default_config.get_performance_logging(),
            default_config.get_bits_per_sample(),
            default_config.get_silence_gate_enabled(),
            default_config.get_silence_gate_timeout_secs()
        );

        // We don't need to convert to TOML since we're creating a template with comments
        sample
    }

    /// Create a configuration file in the specified location
    ///
    /// # Errors
    ///
    /// Returns [`BlackboxError::Io`] if the parent directory can't be created or
    /// the file can't be written.
    pub fn create_config_file(&self, path: &str) -> Result<(), BlackboxError> {
        // Generate sample config content
        let config_content = Self::generate_sample_config();

        // Ensure parent directories exist
        if let Some(parent) = Path::new(path).parent()
            && !parent.exists()
        {
            fs::create_dir_all(parent)?;
        }

        // Write the file
        fs::write(path, config_content)?;

        Ok(())
    }

    // Accessor methods — env vars are already resolved by apply_env_vars() during load().
    // These just unwrap the Option with a default fallback.

    /// Channel spec in comma + range form (`"0,2-4,7"`, 0-based); `BLACKBOX_AUDIO_CHANNELS`.
    #[must_use]
    pub fn get_audio_channels(&self) -> String {
        self.audio_channels
            .clone()
            .unwrap_or_else(|| DEFAULT_CHANNELS.to_owned())
    }

    /// Whether verbose debug logging is on; `BLACKBOX_DEBUG`.
    #[must_use]
    pub fn get_debug(&self) -> bool {
        self.debug.unwrap_or(DEFAULT_DEBUG)
    }

    /// Recording length in seconds, `0` = unlimited; `BLACKBOX_DURATION`.
    #[must_use]
    pub fn get_duration(&self) -> u64 {
        self.duration.unwrap_or(DEFAULT_DURATION)
    }

    /// Raw output mode string, `"single"` or `"split"`; `BLACKBOX_OUTPUT_MODE`.
    /// See [`output_mode_parsed`](Self::output_mode_parsed) for the typed form.
    #[must_use]
    pub fn get_output_mode(&self) -> String {
        self.output_mode
            .clone()
            .unwrap_or_else(|| DEFAULT_OUTPUT_MODE.to_owned())
    }

    /// Parsed output mode. Falls back to the default if the configured value
    /// is missing or unparseable, matching the historical lenient behaviour.
    pub fn output_mode_parsed(&self) -> OutputMode {
        self.output_mode
            .as_deref()
            .and_then(OutputMode::parse)
            .unwrap_or_default()
    }

    /// Normalized amplitude (0.0–1.0) below which a recording counts as silent,
    /// `0` disables the check; `BLACKBOX_SILENCE_THRESHOLD`. Out-of-range values
    /// fall back to the default.
    #[must_use]
    pub fn get_silence_threshold(&self) -> f32 {
        // Reject NaN, ±Inf, and negatives — any of those break the gate's
        // `max_peak > threshold` comparison or produce nonsensical
        // thresholds. Also reject > 1.0 (DOLL-445): the threshold is a
        // normalized amplitude fraction and peaks from the capture path
        // top out at full scale, so a super-unity threshold classifies
        // EVERY recording as silent — the gate never opens, and in
        // continuous mode the silence checker deletes every rotated file.
        let raw = self.silence_threshold.unwrap_or(DEFAULT_SILENCE_THRESHOLD);
        if raw.is_finite() && (0.0..=1.0).contains(&raw) {
            raw
        } else {
            DEFAULT_SILENCE_THRESHOLD
        }
    }

    /// Whether to keep rotating files indefinitely; `BLACKBOX_CONTINUOUS_MODE`.
    #[must_use]
    pub fn get_continuous_mode(&self) -> bool {
        self.continuous_mode.unwrap_or(DEFAULT_CONTINUOUS_MODE)
    }

    /// Seconds between file rotations in continuous mode;
    /// `BLACKBOX_RECORDING_CADENCE`. `0` falls back to the default.
    #[must_use]
    pub fn get_recording_cadence(&self) -> u64 {
        // Reject 0 — the rotation threshold is `sample_rate * channels *
        // cadence`, so a zero cadence makes the RT callback's counter cross
        // the threshold on EVERY callback: a rotation storm creating
        // hundreds of near-empty WAVs per second (DOLL-458). Same
        // getter-side validation style as get_bits_per_sample.
        //
        // Also reject a cadence so large that `sample_rate * channels *
        // cadence` overflows u64: the wrapped threshold was arbitrary, often
        // tiny, which is the same storm.
        let cadence = self.recording_cadence.unwrap_or(DEFAULT_RECORDING_CADENCE);
        if (1..=MAX_RECORDING_CADENCE).contains(&cadence) {
            cadence
        } else {
            warn!(
                "recording_cadence = {cadence} is outside 1..={MAX_RECORDING_CADENCE} seconds; \
                 using the default ({DEFAULT_RECORDING_CADENCE})"
            );
            DEFAULT_RECORDING_CADENCE
        }
    }

    /// Directory recordings are written to; `BLACKBOX_OUTPUT_DIR`. Paths with
    /// `..` components fall back to the default.
    #[must_use]
    pub fn get_output_dir(&self) -> String {
        let dir = self
            .output_dir
            .clone()
            .unwrap_or_else(|| DEFAULT_OUTPUT_DIR.to_owned());

        // Reject paths containing ".." components to prevent path traversal
        let has_parent_traversal = Path::new(&dir)
            .components()
            .any(|c| matches!(c, Component::ParentDir));

        if has_parent_traversal {
            warn!("Output directory '{dir}' contains path traversal components, using default");
            return DEFAULT_OUTPUT_DIR.to_owned();
        }

        dir
    }

    /// Whether to emit per-operation timing logs (needs the `benchmarking`
    /// feature); `BLACKBOX_PERFORMANCE_LOGGING`.
    #[must_use]
    pub fn get_performance_logging(&self) -> bool {
        self.performance_logging
            .unwrap_or(DEFAULT_PERFORMANCE_LOGGING)
    }

    /// cpal input device name, `None` = system default; `BLACKBOX_INPUT_DEVICE`.
    #[must_use]
    pub fn get_input_device(&self) -> Option<String> {
        self.input_device.clone()
    }

    /// Free-space floor in MB below which recording refuses to start, `0`
    /// disables the check; `BLACKBOX_MIN_DISK_SPACE_MB`.
    #[must_use]
    pub fn get_min_disk_space_mb(&self) -> u64 {
        self.min_disk_space_mb.unwrap_or(DEFAULT_MIN_DISK_SPACE_MB)
    }

    /// WAV bit depth, one of 16 / 24 / 32; `BLACKBOX_BITS_PER_SAMPLE`. Other
    /// values fall back to the default.
    #[must_use]
    pub fn get_bits_per_sample(&self) -> u16 {
        match self.bits_per_sample.unwrap_or(DEFAULT_BITS_PER_SAMPLE) {
            16 | 24 | 32 => self.bits_per_sample.unwrap_or(DEFAULT_BITS_PER_SAMPLE),
            _ => DEFAULT_BITS_PER_SAMPLE,
        }
    }

    /// Whether the silence gate closes WAV files while input is silent;
    /// `BLACKBOX_SILENCE_GATE_ENABLED`.
    #[must_use]
    pub fn get_silence_gate_enabled(&self) -> bool {
        self.silence_gate_enabled
            .unwrap_or(DEFAULT_SILENCE_GATE_ENABLED)
    }

    /// Seconds of silence before the gate closes;
    /// `BLACKBOX_SILENCE_GATE_TIMEOUT_SECS`. `0` falls back to the default.
    #[must_use]
    pub fn get_silence_gate_timeout_secs(&self) -> u64 {
        // Reject 0: the gate would close after the first silent batch and
        // reopen on the next sound, splitting a take at every pause into a
        // stream of tiny files. Same fallback as a zero recording_cadence.
        match self
            .silence_gate_timeout_secs
            .unwrap_or(DEFAULT_SILENCE_GATE_TIMEOUT_SECS)
        {
            0 => {
                warn!(
                    "silence_gate_timeout_secs = 0 is invalid; \
                     using the default ({DEFAULT_SILENCE_GATE_TIMEOUT_SECS})"
                );
                DEFAULT_SILENCE_GATE_TIMEOUT_SECS
            }
            v => v,
        }
    }
}

/// Every key a configuration file can set: the `AppConfig` field names.
/// `config_keys_match_the_struct` keeps this in step with the struct.
pub(crate) const CONFIG_KEYS: [&str; 14] = [
    "audio_channels",
    "debug",
    "duration",
    "output_mode",
    "silence_threshold",
    "continuous_mode",
    "recording_cadence",
    "output_dir",
    "performance_logging",
    "input_device",
    "min_disk_space_mb",
    "bits_per_sample",
    "silence_gate_enabled",
    "silence_gate_timeout_secs",
];

/// Top-level keys in the TOML document `content` that `AppConfig` doesn't
/// know, sorted by name. Empty if `content` isn't a TOML table. Tests use it
/// to keep the example configs free of unknown keys; loading reports them
/// through `AppConfig::from_toml_forgiving`.
#[cfg(test)]
pub(crate) fn unknown_config_keys(content: &str) -> Vec<String> {
    content.parse::<toml::Table>().map_or_else(
        |_| Vec::new(),
        |table| {
            table
                .keys()
                .filter(|k| !CONFIG_KEYS.contains(&k.as_str()))
                .cloned()
                .collect()
        },
    )
}

/// The `name` variable parsed with `parse`, else the `legacy` variable parsed
/// the same way. A `name` value that fails to parse falls through to `legacy`.
fn env_override<T>(name: &str, legacy: &str, parse: impl Fn(&str) -> Option<T>) -> Option<T> {
    env::var(name)
        .ok()
        .and_then(|s| parse(&s))
        .or_else(|| env::var(legacy).ok().and_then(|s| parse(&s)))
}

/// Overwrite `slot` only when `value` is `Some`.
fn set_if_some<T>(slot: &mut Option<T>, value: Option<T>) {
    if value.is_some() {
        *slot = value;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn test_default_config() {
        let config = AppConfig::default();
        assert_eq!(config.audio_channels, Some(DEFAULT_CHANNELS.to_owned()));
        assert_eq!(config.debug, Some(DEFAULT_DEBUG));
    }

    #[test]
    fn test_env_vars_override() {
        temp_env::with_vars(
            vec![("AUDIO_CHANNELS", Some("0,2,3")), ("DEBUG", Some("true"))],
            || {
                let mut config = AppConfig {
                    audio_channels: Some(DEFAULT_CHANNELS.to_owned()),
                    debug: Some(false),
                    duration: None,
                    output_mode: None,
                    silence_threshold: None,
                    continuous_mode: None,
                    recording_cadence: None,
                    output_dir: None,
                    performance_logging: None,
                    input_device: None,
                    min_disk_space_mb: None,
                    bits_per_sample: None,
                    silence_gate_enabled: None,
                    silence_gate_timeout_secs: None,
                };

                // Apply environment variables directly
                config.apply_env_vars();

                // Verify environment variables were applied correctly
                assert_eq!(config.audio_channels, Some("0,2,3".to_owned()));
                assert!(config.get_debug());

                // Test the getter methods
                assert_eq!(config.get_audio_channels(), "0,2,3");
                assert!(config.get_debug());
            },
        );
    }

    #[test]
    fn test_create_and_load_config() {
        let temp_dir = tempdir().unwrap();
        let config_path = temp_dir.path().join("test_config.toml");
        let config_path_str = config_path.to_str().unwrap();

        // Create a default config
        let default_config = AppConfig::default();
        assert!(default_config.create_config_file(config_path_str).is_ok());

        // Make sure the file exists
        assert!(config_path.exists());

        // Read the file content to verify
        let content = fs::read_to_string(&config_path).unwrap();
        assert!(content.contains("audio_channels"));
        assert!(content.contains("debug"));
    }

    #[test]
    fn test_parse_bool_variants() {
        // True variants
        assert_eq!(AppConfig::parse_bool("true"), Some(true));
        assert_eq!(AppConfig::parse_bool("TRUE"), Some(true));
        assert_eq!(AppConfig::parse_bool("True"), Some(true));
        assert_eq!(AppConfig::parse_bool("1"), Some(true));
        assert_eq!(AppConfig::parse_bool("yes"), Some(true));
        assert_eq!(AppConfig::parse_bool("YES"), Some(true));
        assert_eq!(AppConfig::parse_bool("on"), Some(true));
        assert_eq!(AppConfig::parse_bool("ON"), Some(true));

        // False variants
        assert_eq!(AppConfig::parse_bool("false"), Some(false));
        assert_eq!(AppConfig::parse_bool("FALSE"), Some(false));
        assert_eq!(AppConfig::parse_bool("0"), Some(false));
        assert_eq!(AppConfig::parse_bool("no"), Some(false));
        assert_eq!(AppConfig::parse_bool("off"), Some(false));

        // Invalid values
        assert_eq!(AppConfig::parse_bool("invalid"), None);
        assert_eq!(AppConfig::parse_bool(""), None);
        assert_eq!(AppConfig::parse_bool("maybe"), None);
        assert_eq!(AppConfig::parse_bool("2"), None);
    }

    #[test]
    fn test_generate_sample_config_is_valid_toml() {
        let sample = AppConfig::generate_sample_config();

        // Should contain all expected keys
        assert!(sample.contains("audio_channels"));
        assert!(sample.contains("debug"));
        assert!(sample.contains("duration"));
        assert!(sample.contains("output_mode"));
        assert!(sample.contains("silence_threshold"));
        assert!(sample.contains("continuous_mode"));
        assert!(sample.contains("recording_cadence"));
        assert!(sample.contains("output_dir"));
        assert!(sample.contains("performance_logging"));

        // Should be parseable as TOML (ignoring comment lines)
        let parsed: Result<AppConfig, _> = toml::from_str(&sample);
        assert!(
            parsed.is_ok(),
            "Generated sample config should be valid TOML: {:?}",
            parsed.err()
        );
    }

    #[test]
    fn test_config_new_equals_default() {
        let new_config = AppConfig::new();
        let default_config = AppConfig::default();

        assert_eq!(new_config.audio_channels, default_config.audio_channels);
        assert_eq!(new_config.debug, default_config.debug);
        assert_eq!(new_config.duration, default_config.duration);
        assert_eq!(new_config.output_mode, default_config.output_mode);
    }

    #[test]
    fn test_config_create_file_nested_dirs() {
        let temp_dir = tempdir().unwrap();
        let nested_path = temp_dir.path().join("a/b/c/config.toml");

        let config = AppConfig::default();
        let result = config.create_config_file(nested_path.to_str().unwrap());
        assert!(result.is_ok());
        assert!(nested_path.exists());
    }

    #[test]
    fn test_config_malformed_toml_falls_back_to_defaults() {
        temp_env::with_vars(
            vec![
                ("BLACKBOX_CONFIG", None::<&str>),
                ("AUDIO_CHANNELS", None::<&str>),
                ("DEBUG", None::<&str>),
                ("BLACKBOX_AUDIO_CHANNELS", None::<&str>),
                ("BLACKBOX_DEBUG", None::<&str>),
            ],
            || {
                let temp_dir = tempdir().unwrap();
                let config_path = temp_dir.path().join("bad.toml");
                fs::write(&config_path, "this is not valid toml [[[").unwrap();

                temp_env::with_var(
                    "BLACKBOX_CONFIG",
                    Some(config_path.to_str().unwrap()),
                    || {
                        let config = AppConfig::load();
                        // Should fall back to defaults, not crash
                        assert_eq!(config.get_audio_channels(), DEFAULT_CHANNELS);
                        assert_eq!(config.get_debug(), DEFAULT_DEBUG);
                    },
                );
            },
        );
    }

    #[test]
    fn test_output_dir_rejects_path_traversal() {
        let leading_traversal = AppConfig {
            output_dir: Some("../../../etc/passwd".to_owned()),
            ..AppConfig::default()
        };
        assert_eq!(leading_traversal.get_output_dir(), DEFAULT_OUTPUT_DIR);

        let embedded_traversal = AppConfig {
            output_dir: Some("recordings/../../../tmp".to_owned()),
            ..AppConfig::default()
        };
        assert_eq!(embedded_traversal.get_output_dir(), DEFAULT_OUTPUT_DIR);

        // Normal paths should be fine
        let relative_dir = AppConfig {
            output_dir: Some("my/recordings".to_owned()),
            ..AppConfig::default()
        };
        assert_eq!(relative_dir.get_output_dir(), "my/recordings");

        let absolute_dir = AppConfig {
            output_dir: Some("/absolute/path/recordings".to_owned()),
            ..AppConfig::default()
        };
        assert_eq!(absolute_dir.get_output_dir(), "/absolute/path/recordings");
    }

    #[test]
    fn test_merge_configs() {
        let mut base_config = AppConfig {
            audio_channels: Some("0,1".to_owned()),
            debug: Some(false),
            duration: Some(10),
            output_mode: Some("single".to_owned()),
            silence_threshold: Some(0.0),
            continuous_mode: Some(false),
            recording_cadence: Some(300),
            output_dir: Some("./recordings".to_owned()),
            performance_logging: Some(false),
            input_device: None,
            min_disk_space_mb: Some(500),
            bits_per_sample: Some(24),
            silence_gate_enabled: Some(false),
            silence_gate_timeout_secs: Some(300),
        };

        let override_config = AppConfig {
            audio_channels: Some("2,3".to_owned()),
            debug: Some(true),
            duration: None, // This shouldn't override
            output_mode: Some("split".to_owned()),
            silence_threshold: None,   // This shouldn't override
            continuous_mode: None,     // This shouldn't override
            recording_cadence: None,   // This shouldn't override
            output_dir: None,          // This shouldn't override
            performance_logging: None, // This shouldn't override
            input_device: None,
            min_disk_space_mb: None,         // This shouldn't override
            bits_per_sample: None,           // This shouldn't override
            silence_gate_enabled: None,      // This shouldn't override
            silence_gate_timeout_secs: None, // This shouldn't override
        };

        base_config.merge(override_config);

        // Check that only the Some values were overridden
        assert_eq!(base_config.audio_channels, Some("2,3".to_owned()));
        assert!(base_config.get_debug());
        assert_eq!(base_config.duration, Some(10)); // Unchanged
        assert_eq!(base_config.output_mode, Some("split".to_owned()));
        assert_eq!(base_config.silence_threshold, Some(0.0)); // Unchanged
        assert_eq!(base_config.continuous_mode, Some(false)); // Unchanged
        assert_eq!(base_config.recording_cadence, Some(300)); // Unchanged
        assert_eq!(base_config.output_dir, Some("./recordings".to_owned())); // Unchanged
        assert_eq!(base_config.performance_logging, Some(false)); // Unchanged
    }

    #[test]
    #[expect(
        clippy::float_cmp,
        reason = "exact-value test: same literal in and out"
    )]
    fn test_silence_threshold_rejects_out_of_range() {
        // NaN, ±Inf, and negative values fall back to the default rather
        // than poison the gate's max_peak > threshold comparison. Values
        // above 1.0 fall back too (DOLL-445): peaks are normalized to
        // full scale, so a super-unity threshold marks every recording
        // silent and the silence checker deletes them all.
        for bad in [
            f32::NAN,
            f32::INFINITY,
            f32::NEG_INFINITY,
            -0.1,
            -1.0,
            1.000_1,
            2.0,
            100.0,
        ] {
            let config = AppConfig {
                silence_threshold: Some(bad),
                ..AppConfig::default()
            };
            assert_eq!(
                config.get_silence_threshold(),
                DEFAULT_SILENCE_THRESHOLD,
                "silence_threshold = {bad} should fall back to default"
            );
        }

        // Zero and in-range values pass through (1.0 is the inclusive
        // ceiling — degenerate but explicit).
        for good in [0.0_f32, 0.001, 0.5, 1.0] {
            let config = AppConfig {
                silence_threshold: Some(good),
                ..AppConfig::default()
            };
            assert_eq!(config.get_silence_threshold(), good);
        }
    }

    /// DOLL-458: cadence 0 would make the rotation threshold 0 — the RT
    /// callback's counter crosses it on every callback, producing hundreds
    /// of near-empty WAVs per second. The getter must fall back to the
    /// default instead.
    #[test]
    fn test_recording_cadence_rejects_zero() {
        let zero_cadence = AppConfig {
            recording_cadence: Some(0),
            ..AppConfig::default()
        };
        assert_eq!(
            zero_cadence.get_recording_cadence(),
            DEFAULT_RECORDING_CADENCE
        );

        // Positive values pass through.
        for good in [1, 60, 300, 86_400] {
            let good_cadence = AppConfig {
                recording_cadence: Some(good),
                ..AppConfig::default()
            };
            assert_eq!(good_cadence.get_recording_cadence(), good);
        }

        // None falls back to default.
        let unset_cadence = AppConfig {
            recording_cadence: None,
            ..AppConfig::default()
        };
        assert_eq!(
            unset_cadence.get_recording_cadence(),
            DEFAULT_RECORDING_CADENCE
        );
    }

    /// DOLL-458: the env-var path also lands a raw 0 in the field; the
    /// getter is the single validation point, so it must reject it there too.
    #[test]
    fn test_recording_cadence_zero_from_env_rejected() {
        temp_env::with_vars(
            vec![
                ("BLACKBOX_RECORDING_CADENCE", Some("0")),
                ("RECORDING_CADENCE", None::<&str>),
                ("BLACKBOX_CONFIG", None::<&str>),
            ],
            || {
                let mut config = AppConfig::default();
                config.apply_env_vars();
                assert_eq!(config.recording_cadence, Some(0), "env value lands raw");
                assert_eq!(
                    config.get_recording_cadence(),
                    DEFAULT_RECORDING_CADENCE,
                    "getter must reject the zero cadence"
                );
            },
        );
    }

    #[test]
    fn test_bits_per_sample_validation() {
        // Valid values pass through
        for valid in [16, 24, 32] {
            let config = AppConfig {
                bits_per_sample: Some(valid),
                ..AppConfig::default()
            };
            assert_eq!(config.get_bits_per_sample(), valid);
        }

        // Invalid values fall back to default (24)
        for invalid in [0, 8, 12, 48, 64, 128] {
            let config = AppConfig {
                bits_per_sample: Some(invalid),
                ..AppConfig::default()
            };
            assert_eq!(config.get_bits_per_sample(), DEFAULT_BITS_PER_SAMPLE);
        }

        // None falls back to default
        let config = AppConfig {
            bits_per_sample: None,
            ..AppConfig::default()
        };
        assert_eq!(config.get_bits_per_sample(), DEFAULT_BITS_PER_SAMPLE);
    }
}
