use serde::{Deserialize, Serialize};
use std::env;
use std::fs;
use std::path::{Path, PathBuf};

use crate::constants::{
    DEFAULT_CHANNELS, DEFAULT_CONTINUOUS_MODE, DEFAULT_DEBUG, DEFAULT_DURATION, DEFAULT_OUTPUT_DIR,
    DEFAULT_OUTPUT_MODE, DEFAULT_PERFORMANCE_LOGGING, DEFAULT_RECORDING_CADENCE,
    DEFAULT_SILENCE_THRESHOLD, ENV_CONTINUOUS_MODE, ENV_DEBUG, ENV_DURATION,
    ENV_PERFORMANCE_LOGGING, ENV_RECORDING_CADENCE, ENV_SILENCE_THRESHOLD,
};

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
                        println!("Loaded configuration from {}", config_path.display());
                        // Merge with defaults
                        config.merge(file_config);
                    }
                    Err(e) => {
                        eprintln!("Error parsing config file: {}", e);
                    }
                },
                Err(e) => {
                    eprintln!("Error reading config file: {}", e);
                }
            }
        }

        // Override with environment variables
        config.apply_env_vars();

        config
    }

    /// Merge another configuration into this one, only taking values that are Some
    fn merge(&mut self, other: AppConfig) {
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

    /// Apply environment variables to override configuration
    fn apply_env_vars(&mut self) {
        if let Ok(val) = env::var("AUDIO_CHANNELS") {
            self.audio_channels = Some(val);
        }
        if let Ok(val) = env::var("DEBUG") {
            // Parse boolean values correctly
            match val.to_lowercase().as_str() {
                "true" | "1" | "yes" | "on" => self.debug = Some(true),
                "false" | "0" | "no" | "off" => self.debug = Some(false),
                _ => {
                    // Try standard parsing as fallback
                    if let Ok(debug) = val.parse() {
                        self.debug = Some(debug);
                    }
                }
            }
        }
        if let Ok(val) = env::var("RECORD_DURATION") {
            if let Ok(duration) = val.parse() {
                self.duration = Some(duration);
            }
        }
        if let Ok(val) = env::var("OUTPUT_MODE") {
            self.output_mode = Some(val);
        }
        if let Ok(val) = env::var("SILENCE_THRESHOLD") {
            if let Ok(threshold) = val.parse() {
                self.silence_threshold = Some(threshold);
            }
        }
        if let Ok(val) = env::var("CONTINUOUS_MODE") {
            if let Ok(continuous) = val.parse() {
                self.continuous_mode = Some(continuous);
            }
        }
        if let Ok(val) = env::var("RECORDING_CADENCE") {
            if let Ok(cadence) = val.parse() {
                self.recording_cadence = Some(cadence);
            }
        }
        if let Ok(val) = env::var("OUTPUT_DIR") {
            self.output_dir = Some(val);
        }
        if let Ok(val) = env::var("PERFORMANCE_LOGGING") {
            if let Ok(perf_logging) = val.parse() {
                self.performance_logging = Some(perf_logging);
            }
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
    pub fn create_config_file(&self, path: &str) -> Result<(), String> {
        // Generate sample config content
        let config_content = Self::generate_sample_config();

        // Ensure parent directories exist
        if let Some(parent) = Path::new(path).parent() {
            if !parent.exists() {
                fs::create_dir_all(parent)
                    .map_err(|e| format!("Failed to create directory: {}", e))?;
            }
        }

        // Write the file
        fs::write(path, config_content)
            .map_err(|e| format!("Failed to write config file: {}", e))?;

        Ok(())
    }

    // Accessor methods with proper unwrapping

    pub fn get_audio_channels(&self) -> String {
        self.audio_channels
            .clone()
            .unwrap_or_else(|| DEFAULT_CHANNELS.to_string())
    }

    pub fn get_debug(&self) -> bool {
        self.debug
            .or_else(|| {
                std::env::var(ENV_DEBUG)
                    .ok()
                    .map(|s| s.parse().unwrap_or(false))
            })
            .unwrap_or(DEFAULT_DEBUG)
    }

    pub fn get_duration(&self) -> u64 {
        self.duration
            .or_else(|| {
                std::env::var(ENV_DURATION)
                    .ok()
                    .map(|s| s.parse().unwrap_or(10))
            })
            .unwrap_or(DEFAULT_DURATION)
    }

    pub fn get_output_mode(&self) -> String {
        self.output_mode
            .clone()
            .unwrap_or_else(|| DEFAULT_OUTPUT_MODE.to_string())
    }

    pub fn get_silence_threshold(&self) -> f32 {
        self.silence_threshold
            .or_else(|| {
                std::env::var(ENV_SILENCE_THRESHOLD)
                    .ok()
                    .map(|s| s.parse().unwrap_or(0.0))
            })
            .unwrap_or(DEFAULT_SILENCE_THRESHOLD)
    }

    pub fn get_continuous_mode(&self) -> bool {
        self.continuous_mode
            .or_else(|| {
                std::env::var(ENV_CONTINUOUS_MODE)
                    .ok()
                    .map(|s| s.parse().unwrap_or(false))
            })
            .unwrap_or(DEFAULT_CONTINUOUS_MODE)
    }

    pub fn get_recording_cadence(&self) -> u64 {
        self.recording_cadence
            .or_else(|| {
                std::env::var(ENV_RECORDING_CADENCE)
                    .ok()
                    .map(|s| s.parse().unwrap_or(300))
            })
            .unwrap_or(DEFAULT_RECORDING_CADENCE)
    }

    pub fn get_output_dir(&self) -> String {
        self.output_dir
            .clone()
            .unwrap_or_else(|| DEFAULT_OUTPUT_DIR.to_string())
    }

    pub fn get_performance_logging(&self) -> bool {
        self.performance_logging
            .or_else(|| {
                std::env::var(ENV_PERFORMANCE_LOGGING)
                    .ok()
                    .map(|s| s.parse().unwrap_or(false))
            })
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
        // Set environment variables
        env::set_var("AUDIO_CHANNELS", "0,2,3");
        env::set_var("DEBUG", "true");

        // Create a new config with explicit values
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
        assert_eq!(config.debug, Some(true));

        // Test the getter methods
        assert_eq!(config.get_audio_channels(), "0,2,3");
        assert_eq!(config.get_debug(), true);

        // Clean up
        env::remove_var("AUDIO_CHANNELS");
        env::remove_var("DEBUG");
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
        assert_eq!(base_config.debug, Some(true));
        assert_eq!(base_config.duration, Some(10)); // Unchanged
        assert_eq!(base_config.output_mode, Some("split".to_string()));
        assert_eq!(base_config.silence_threshold, Some(0.0)); // Unchanged
        assert_eq!(base_config.continuous_mode, Some(false)); // Unchanged
        assert_eq!(base_config.recording_cadence, Some(300)); // Unchanged
        assert_eq!(base_config.output_dir, Some("./recordings".to_string())); // Unchanged
        assert_eq!(base_config.performance_logging, Some(false)); // Unchanged
    }
}
