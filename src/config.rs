use log::{error, info};
use serde::{Deserialize, Serialize};
use std::env;
use std::fs;
use std::path::{Path, PathBuf};

use crate::constants::{
    DEFAULT_CHANNELS, DEFAULT_CONTINUOUS_MODE, DEFAULT_DEBUG, DEFAULT_DURATION, DEFAULT_OUTPUT_DIR,
    DEFAULT_OUTPUT_MODE, DEFAULT_PERFORMANCE_LOGGING, DEFAULT_RECORDING_CADENCE,
    DEFAULT_SILENCE_THRESHOLD,
};
use crate::error::BlackboxError;

/// The main configuration struct that holds all settings for the audio recorder.
///
/// This structure can be initialized from environment variables, a TOML file,
/// or with default values. Values are resolved with environment variables having
/// the highest precedence, followed by the config file, and then defaults.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppConfig {
    /// Audio channels to record (comma-separated or range)
    pub audio_channels: Option<String>,
    /// Enable debug output
    pub debug: Option<bool>,
    /// Recording duration in seconds
    pub duration: Option<u64>,
    /// Output mode: "single" or "split"
    pub output_mode: Option<String>,
    /// Threshold for silence detection
    pub silence_threshold: Option<f32>,
    /// Enable continuous recording
    pub continuous_mode: Option<bool>,
    /// How often to rotate files in continuous mode (seconds)
    pub recording_cadence: Option<u64>,
    /// Directory for saving audio files
    pub output_dir: Option<String>,
    /// Enable performance metrics collection
    pub performance_logging: Option<bool>,
}

impl Default for AppConfig {
    fn default() -> Self {
        AppConfig {
            audio_channels: Some(DEFAULT_CHANNELS.to_string()),
            debug: Some(DEFAULT_DEBUG),
            duration: Some(DEFAULT_DURATION),
            output_mode: Some(DEFAULT_OUTPUT_MODE.to_string()),
            silence_threshold: Some(DEFAULT_SILENCE_THRESHOLD),
            continuous_mode: Some(DEFAULT_CONTINUOUS_MODE),
            recording_cadence: Some(DEFAULT_RECORDING_CADENCE),
            output_dir: Some(DEFAULT_OUTPUT_DIR.to_string()),
            performance_logging: Some(DEFAULT_PERFORMANCE_LOGGING),
        }
    }
}

impl AppConfig {
    /// Create a new configuration with default values
    pub fn new() -> Self {
        AppConfig::default()
    }

    /// Find the configuration file path
    fn find_config_file() -> Option<PathBuf> {
        // First check if a config file path is specified in the environment
        if let Ok(config_path) = env::var("BLACKBOX_CONFIG") {
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
    pub fn load() -> Self {
        let mut config = AppConfig::default();

        // Try to find and load the configuration file
        if let Some(config_path) = Self::find_config_file() {
            match fs::read_to_string(&config_path) {
                Ok(content) => match toml::from_str::<AppConfig>(&content) {
                    Ok(file_config) => {
                        info!("Loaded configuration from {}", config_path.display());
                        // Merge with defaults
                        config.merge(file_config);
                    }
                    Err(e) => {
                        error!("Error parsing config file: {}", e);
                    }
                },
                Err(e) => {
                    error!("Error reading config file: {}", e);
                }
            }
        }

        // Override with environment variables
        config.apply_env_vars();

        config
    }

    /// Merge another configuration into this one, only taking values that are Some
    pub fn merge(&mut self, other: AppConfig) {
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
    }

    /// Parse a boolean value from a string
    fn parse_bool(val: &str) -> Option<bool> {
        match val.to_lowercase().as_str() {
            "true" | "1" | "yes" | "on" => Some(true),
            "false" | "0" | "no" | "off" => Some(false),
            _ => None,
        }
    }

    /// Apply environment variables to override configuration
    fn apply_env_vars(&mut self) {
        // Try both prefixed and unprefixed environment variables
        let channels = std::env::var("BLACKBOX_AUDIO_CHANNELS")
            .ok()
            .or_else(|| std::env::var("AUDIO_CHANNELS").ok());
        if let Some(val) = channels {
            self.audio_channels = Some(val);
        }

        let debug = std::env::var("BLACKBOX_DEBUG")
            .ok()
            .and_then(|s| Self::parse_bool(&s))
            .or_else(|| {
                std::env::var("DEBUG")
                    .ok()
                    .and_then(|s| Self::parse_bool(&s))
            });
        if let Some(val) = debug {
            self.debug = Some(val);
        }

        let duration = std::env::var("BLACKBOX_DURATION")
            .ok()
            .and_then(|s| s.parse().ok())
            .or_else(|| {
                std::env::var("RECORD_DURATION")
                    .ok()
                    .and_then(|s| s.parse().ok())
            });
        if let Some(val) = duration {
            self.duration = Some(val);
        }

        let output_mode = std::env::var("BLACKBOX_OUTPUT_MODE")
            .ok()
            .or_else(|| std::env::var("OUTPUT_MODE").ok());
        if let Some(val) = output_mode {
            self.output_mode = Some(val);
        }

        let threshold = std::env::var("BLACKBOX_SILENCE_THRESHOLD")
            .ok()
            .and_then(|s| s.parse().ok())
            .or_else(|| {
                std::env::var("SILENCE_THRESHOLD")
                    .ok()
                    .and_then(|s| s.parse().ok())
            });
        if let Some(val) = threshold {
            self.silence_threshold = Some(val);
        }

        let continuous = std::env::var("BLACKBOX_CONTINUOUS_MODE")
            .ok()
            .and_then(|s| Self::parse_bool(&s))
            .or_else(|| {
                std::env::var("CONTINUOUS_MODE")
                    .ok()
                    .and_then(|s| Self::parse_bool(&s))
            });
        if let Some(val) = continuous {
            self.continuous_mode = Some(val);
        }

        let cadence = std::env::var("BLACKBOX_RECORDING_CADENCE")
            .ok()
            .and_then(|s| s.parse().ok())
            .or_else(|| {
                std::env::var("RECORDING_CADENCE")
                    .ok()
                    .and_then(|s| s.parse().ok())
            });
        if let Some(val) = cadence {
            self.recording_cadence = Some(val);
        }

        let output_dir = std::env::var("BLACKBOX_OUTPUT_DIR")
            .ok()
            .or_else(|| std::env::var("OUTPUT_DIR").ok());
        if let Some(val) = output_dir {
            self.output_dir = Some(val);
        }

        let perf_logging = std::env::var("BLACKBOX_PERFORMANCE_LOGGING")
            .ok()
            .and_then(|s| Self::parse_bool(&s))
            .or_else(|| {
                std::env::var("PERFORMANCE_LOGGING")
                    .ok()
                    .and_then(|s| Self::parse_bool(&s))
            });
        if let Some(val) = perf_logging {
            self.performance_logging = Some(val);
        }
    }

    /// Generate a sample configuration file with comments
    pub fn generate_sample_config() -> String {
        let default_config = AppConfig::default();

        // Create a string with comments and the default values
        let sample = format!(
            r#"# Blackbox Audio Recorder Configuration
# This file configures the behavior of the audio recorder.
# Values set here can be overridden by environment variables.

# Audio channels to record (comma-separated list or ranges like 0-2)
# Default: {}
audio_channels = "{}"

# Enable debug output (true/false)
# Default: {}
debug = {}

# Recording duration in seconds (0 for unlimited)
# Default: {}
duration = {}

# Output mode: "single" (one file), "split" (one file per channel)
# Default: {}
output_mode = "{}"

# Silence threshold (0-100, 0 disables silence detection)
# Default: {}
silence_threshold = {}

# Continuous recording mode (true/false)
# Default: {}
continuous_mode = {}

# Recording cadence in seconds (how often to rotate files in continuous mode)
# Default: {}
recording_cadence = {}

# Output directory for recordings
# Default: {}
output_dir = "{}"

# Enable performance logging (true/false)
# Default: {}
performance_logging = {}
"#,
            DEFAULT_CHANNELS,
            default_config.get_audio_channels(),
            DEFAULT_DEBUG,
            default_config.get_debug(),
            DEFAULT_DURATION,
            default_config.get_duration(),
            DEFAULT_OUTPUT_MODE,
            default_config.get_output_mode(),
            DEFAULT_SILENCE_THRESHOLD,
            default_config.get_silence_threshold(),
            DEFAULT_CONTINUOUS_MODE,
            default_config.get_continuous_mode(),
            DEFAULT_RECORDING_CADENCE,
            default_config.get_recording_cadence(),
            DEFAULT_OUTPUT_DIR,
            default_config.get_output_dir(),
            DEFAULT_PERFORMANCE_LOGGING,
            default_config.get_performance_logging()
        );

        // We don't need to convert to TOML since we're creating a template with comments
        sample
    }

    /// Create a configuration file in the specified location
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

    pub fn get_audio_channels(&self) -> String {
        self.audio_channels
            .clone()
            .unwrap_or_else(|| DEFAULT_CHANNELS.to_string())
    }

    pub fn get_debug(&self) -> bool {
        self.debug.unwrap_or(DEFAULT_DEBUG)
    }

    pub fn get_duration(&self) -> u64 {
        self.duration.unwrap_or(DEFAULT_DURATION)
    }

    pub fn get_output_mode(&self) -> String {
        self.output_mode
            .clone()
            .unwrap_or_else(|| DEFAULT_OUTPUT_MODE.to_string())
    }

    pub fn get_silence_threshold(&self) -> f32 {
        self.silence_threshold.unwrap_or(DEFAULT_SILENCE_THRESHOLD)
    }

    pub fn get_continuous_mode(&self) -> bool {
        self.continuous_mode.unwrap_or(DEFAULT_CONTINUOUS_MODE)
    }

    pub fn get_recording_cadence(&self) -> u64 {
        self.recording_cadence.unwrap_or(DEFAULT_RECORDING_CADENCE)
    }

    pub fn get_output_dir(&self) -> String {
        self.output_dir
            .clone()
            .unwrap_or_else(|| DEFAULT_OUTPUT_DIR.to_string())
    }

    pub fn get_performance_logging(&self) -> bool {
        self.performance_logging
            .unwrap_or(DEFAULT_PERFORMANCE_LOGGING)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn test_default_config() {
        let config = AppConfig::default();
        assert_eq!(config.audio_channels, Some(DEFAULT_CHANNELS.to_string()));
        assert_eq!(config.debug, Some(DEFAULT_DEBUG));
    }

    #[test]
    fn test_env_vars_override() {
        temp_env::with_vars(
            vec![("AUDIO_CHANNELS", Some("0,2,3")), ("DEBUG", Some("true"))],
            || {
                let mut config = AppConfig {
                    audio_channels: Some(DEFAULT_CHANNELS.to_string()),
                    debug: Some(false),
                    duration: None,
                    output_mode: None,
                    silence_threshold: None,
                    continuous_mode: None,
                    recording_cadence: None,
                    output_dir: None,
                    performance_logging: None,
                };

                // Apply environment variables directly
                config.apply_env_vars();

                // Verify environment variables were applied correctly
                assert_eq!(config.audio_channels, Some("0,2,3".to_string()));
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

                // SAFETY: test serialized by temp_env
                unsafe { std::env::set_var("BLACKBOX_CONFIG", config_path.to_str().unwrap()) };

                let config = AppConfig::load();
                // Should fall back to defaults, not crash
                assert_eq!(config.get_audio_channels(), DEFAULT_CHANNELS);
                assert_eq!(config.get_debug(), DEFAULT_DEBUG);
            },
        );
    }

    #[test]
    fn test_merge_configs() {
        let mut base_config = AppConfig {
            audio_channels: Some("0,1".to_string()),
            debug: Some(false),
            duration: Some(10),
            output_mode: Some("single".to_string()),
            silence_threshold: Some(0.0),
            continuous_mode: Some(false),
            recording_cadence: Some(300),
            output_dir: Some("./recordings".to_string()),
            performance_logging: Some(false),
        };

        let override_config = AppConfig {
            audio_channels: Some("2,3".to_string()),
            debug: Some(true),
            duration: None, // This shouldn't override
            output_mode: Some("split".to_string()),
            silence_threshold: None,   // This shouldn't override
            continuous_mode: None,     // This shouldn't override
            recording_cadence: None,   // This shouldn't override
            output_dir: None,          // This shouldn't override
            performance_logging: None, // This shouldn't override
        };

        base_config.merge(override_config);

        // Check that only the Some values were overridden
        assert_eq!(base_config.audio_channels, Some("2,3".to_string()));
        assert!(base_config.get_debug());
        assert_eq!(base_config.duration, Some(10)); // Unchanged
        assert_eq!(base_config.output_mode, Some("split".to_string()));
        assert_eq!(base_config.silence_threshold, Some(0.0)); // Unchanged
        assert_eq!(base_config.continuous_mode, Some(false)); // Unchanged
        assert_eq!(base_config.recording_cadence, Some(300)); // Unchanged
        assert_eq!(base_config.output_dir, Some("./recordings".to_string())); // Unchanged
        assert_eq!(base_config.performance_logging, Some(false)); // Unchanged
    }
}
