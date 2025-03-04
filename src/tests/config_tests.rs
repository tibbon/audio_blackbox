use std::env;
use std::fs;
use temp_env;
use tempfile::tempdir;

use crate::config::AppConfig;
use crate::constants::*;

#[test]
fn test_config_loading() {
    // Use temp_env to isolate the test environment
    temp_env::with_vars(
        [
            ("AUDIO_CHANNELS", None::<&str>),
            ("DEBUG", None::<&str>),
            ("RECORD_DURATION", None::<&str>),
            ("OUTPUT_MODE", None::<&str>),
            ("SILENCE_THRESHOLD", None::<&str>),
            ("CONTINUOUS_MODE", None::<&str>),
            ("RECORDING_CADENCE", None::<&str>),
            ("OUTPUT_DIR", None::<&str>),
            ("PERFORMANCE_LOGGING", None::<&str>),
            ("BLACKBOX_AUDIO_CHANNELS", None::<&str>),
            ("BLACKBOX_DEBUG", None::<&str>),
            ("BLACKBOX_DURATION", None::<&str>),
            ("BLACKBOX_OUTPUT_MODE", None::<&str>),
            ("BLACKBOX_SILENCE_THRESHOLD", None::<&str>),
            ("BLACKBOX_CONTINUOUS_MODE", None::<&str>),
            ("BLACKBOX_RECORDING_CADENCE", None::<&str>),
            ("BLACKBOX_OUTPUT_DIR", None::<&str>),
            ("BLACKBOX_PERFORMANCE_LOGGING", None::<&str>),
            ("BLACKBOX_CONFIG", None::<&str>),
        ],
        || {
            let temp_dir = tempdir().unwrap();
            let config_path = temp_dir.path().join("blackbox.toml");

            // Create a test config file
            let config_content = r#"
            # Test configuration file
            audio_channels = "1,2,3"
            debug = true
            duration = 60
            output_mode = "split"
            silence_threshold = 0.05
            continuous_mode = true
            recording_cadence = 1800
            output_dir = "/tmp"
            performance_logging = true
        "#;
            fs::write(&config_path, config_content).unwrap();

            // Point to our test config file
            env::set_var("BLACKBOX_CONFIG", config_path.to_str().unwrap());

            // Load the configuration
            let config = AppConfig::load();

            // Verify that the config was loaded from the file
            assert_eq!(config.get_audio_channels(), "1,2,3");
            assert_eq!(config.get_debug(), true);
            assert_eq!(config.get_duration(), 60);
            assert_eq!(config.get_output_mode(), "split");
            assert_eq!(config.get_silence_threshold(), 0.05);
            assert_eq!(config.get_continuous_mode(), true);
            assert_eq!(config.get_recording_cadence(), 1800);
            assert_eq!(config.get_output_dir(), "/tmp");
            assert_eq!(config.get_performance_logging(), true);
        },
    );
}

#[test]
fn test_config_merge() {
    temp_env::with_vars(
        [
            ("AUDIO_CHANNELS", None::<&str>),
            ("DEBUG", None::<&str>),
            ("BLACKBOX_AUDIO_CHANNELS", None::<&str>),
            ("BLACKBOX_DEBUG", None::<&str>),
        ],
        || {
            let mut config1 = AppConfig::default();
            let mut config2 = AppConfig::default();

            // Set some values in config1
            config1.audio_channels = Some("0".to_string());
            config1.debug = Some(true);

            // Set different values in config2
            config2.audio_channels = Some("1,2".to_string()); // Different from config1
            config2.debug = Some(false); // Different from config1
            config2.duration = Some(60);
            config2.output_mode = Some("split".to_string());

            // Merge config2 into config1
            config1.merge(config2);

            // Verify merged values - config2 values should overwrite config1
            assert_eq!(config1.audio_channels, Some("1,2".to_string()));
            assert_eq!(config1.debug, Some(false));
            assert_eq!(config1.duration, Some(60));
            assert_eq!(config1.output_mode, Some("split".to_string()));
        },
    );
}

#[test]
fn test_config_defaults() {
    temp_env::with_vars(
        [
            ("AUDIO_CHANNELS", None::<&str>),
            ("DEBUG", None::<&str>),
            ("RECORD_DURATION", None::<&str>),
            ("OUTPUT_MODE", None::<&str>),
            ("SILENCE_THRESHOLD", None::<&str>),
            ("CONTINUOUS_MODE", None::<&str>),
            ("RECORDING_CADENCE", None::<&str>),
            ("OUTPUT_DIR", None::<&str>),
            ("PERFORMANCE_LOGGING", None::<&str>),
            ("BLACKBOX_AUDIO_CHANNELS", None::<&str>),
            ("BLACKBOX_DEBUG", None::<&str>),
            ("BLACKBOX_DURATION", None::<&str>),
            ("BLACKBOX_OUTPUT_MODE", None::<&str>),
            ("BLACKBOX_SILENCE_THRESHOLD", None::<&str>),
            ("BLACKBOX_CONTINUOUS_MODE", None::<&str>),
            ("BLACKBOX_RECORDING_CADENCE", None::<&str>),
            ("BLACKBOX_OUTPUT_DIR", None::<&str>),
            ("BLACKBOX_PERFORMANCE_LOGGING", None::<&str>),
        ],
        || {
            let config = AppConfig::default();

            // Check the default initialization values
            assert_eq!(config.audio_channels, Some(DEFAULT_CHANNELS.to_string()));
            assert_eq!(config.debug, Some(DEFAULT_DEBUG));
            assert_eq!(config.duration, Some(DEFAULT_DURATION));
            assert_eq!(config.output_mode, Some(DEFAULT_OUTPUT_MODE.to_string()));
            assert_eq!(config.silence_threshold, Some(DEFAULT_SILENCE_THRESHOLD));
            assert_eq!(config.continuous_mode, Some(DEFAULT_CONTINUOUS_MODE));
            assert_eq!(config.recording_cadence, Some(DEFAULT_RECORDING_CADENCE));
            assert_eq!(config.output_dir, Some(DEFAULT_OUTPUT_DIR.to_string()));
            assert_eq!(
                config.performance_logging,
                Some(DEFAULT_PERFORMANCE_LOGGING)
            );

            // Verify getter methods return the same values
            assert_eq!(config.get_audio_channels(), DEFAULT_CHANNELS);
            assert_eq!(config.get_debug(), DEFAULT_DEBUG);
            assert_eq!(config.get_duration(), DEFAULT_DURATION);
            assert_eq!(config.get_output_mode(), DEFAULT_OUTPUT_MODE);
            assert_eq!(config.get_silence_threshold(), DEFAULT_SILENCE_THRESHOLD);
            assert_eq!(config.get_continuous_mode(), DEFAULT_CONTINUOUS_MODE);
            assert_eq!(config.get_recording_cadence(), DEFAULT_RECORDING_CADENCE);
            assert_eq!(config.get_output_dir(), DEFAULT_OUTPUT_DIR);
            assert_eq!(
                config.get_performance_logging(),
                DEFAULT_PERFORMANCE_LOGGING
            );
        },
    );
}

#[test]
fn test_config_file_creation() {
    let temp_dir = tempdir().unwrap();
    let config_path = temp_dir.path().join("test_config.toml");

    let config = AppConfig::default();
    config
        .create_config_file(config_path.to_str().unwrap())
        .unwrap();

    assert!(config_path.exists());

    let content = fs::read_to_string(&config_path).unwrap();
    assert!(content.contains("audio_channels"));
    assert!(content.contains("debug"));
    assert!(content.contains("duration"));
}

#[test]
fn test_config_env_vars() {
    // Use temp_env to isolate the test environment
    temp_env::with_vars(
        [
            ("AUDIO_CHANNELS", None::<&str>),
            ("DEBUG", None::<&str>),
            ("RECORD_DURATION", None::<&str>),
            ("OUTPUT_MODE", None::<&str>),
            ("SILENCE_THRESHOLD", None::<&str>),
            ("CONTINUOUS_MODE", None::<&str>),
            ("RECORDING_CADENCE", None::<&str>),
            ("OUTPUT_DIR", None::<&str>),
            ("PERFORMANCE_LOGGING", None::<&str>),
            ("BLACKBOX_AUDIO_CHANNELS", None::<&str>),
            ("BLACKBOX_DEBUG", None::<&str>),
            ("BLACKBOX_DURATION", None::<&str>),
            ("BLACKBOX_OUTPUT_MODE", None::<&str>),
            ("BLACKBOX_SILENCE_THRESHOLD", None::<&str>),
            ("BLACKBOX_CONTINUOUS_MODE", None::<&str>),
            ("BLACKBOX_RECORDING_CADENCE", None::<&str>),
            ("BLACKBOX_OUTPUT_DIR", None::<&str>),
            ("BLACKBOX_PERFORMANCE_LOGGING", None::<&str>),
            ("BLACKBOX_CONFIG", None::<&str>),
        ],
        || {
            let temp_dir = tempdir().unwrap();
            let config_path = temp_dir.path().join("blackbox.toml");

            // Create a minimal config file with different values
            let config_content = r#"
            # Config file values should be overridden by environment variables
            audio_channels = "0,1,2"
            debug = false
            duration = 30
            output_mode = "single"
            silence_threshold = 0.1
            continuous_mode = false
            recording_cadence = 300
            output_dir = "./recordings"
            performance_logging = false
        "#;
            fs::write(&config_path, config_content).unwrap();

            // Point to our test config file and ONLY this config file
            env::set_var("BLACKBOX_CONFIG", config_path.to_str().unwrap());

            // Load configuration and verify it uses values from the config file
            let config = AppConfig::load();

            // Print values for debugging
            println!("Config values:");
            println!("  audio_channels: {}", config.get_audio_channels());
            println!("  debug: {}", config.get_debug());

            assert_eq!(
                config.get_audio_channels(),
                "0,1,2",
                "Config should load audio_channels from file"
            );
            assert_eq!(
                config.get_debug(),
                false,
                "Config should load debug from file"
            );
        },
    );
}

#[test]
fn test_config_env_vars_precedence() {
    // Use temp_env to isolate the test environment
    temp_env::with_vars(
        [
            ("AUDIO_CHANNELS", None::<&str>),
            ("DEBUG", None::<&str>),
            ("RECORD_DURATION", None::<&str>),
            ("OUTPUT_MODE", None::<&str>),
            ("SILENCE_THRESHOLD", None::<&str>),
            ("CONTINUOUS_MODE", None::<&str>),
            ("RECORDING_CADENCE", None::<&str>),
            ("OUTPUT_DIR", None::<&str>),
            ("PERFORMANCE_LOGGING", None::<&str>),
            ("BLACKBOX_AUDIO_CHANNELS", Some("3,4,5")),
            ("BLACKBOX_DEBUG", Some("true")),
            ("BLACKBOX_DURATION", Some("120")),
            ("BLACKBOX_OUTPUT_MODE", Some("split")),
            ("BLACKBOX_SILENCE_THRESHOLD", Some("0.001")),
            ("BLACKBOX_CONTINUOUS_MODE", Some("true")),
            ("BLACKBOX_RECORDING_CADENCE", Some("600")),
            ("BLACKBOX_OUTPUT_DIR", Some("/tmp/test_output")),
            ("BLACKBOX_PERFORMANCE_LOGGING", Some("true")),
        ],
        || {
            let temp_dir = tempdir().unwrap();
            let config_path = temp_dir.path().join("blackbox.toml");

            // Create a minimal config file with different values
            let config_content = r#"
            # Config file values should be overridden by environment variables
            audio_channels = "0,1,2"
            debug = false
            duration = 30
            output_mode = "single"
            silence_threshold = 0.1
            continuous_mode = false
            recording_cadence = 300
            output_dir = "./recordings"
            performance_logging = false
        "#;
            fs::write(&config_path, config_content).unwrap();

            // Point to our test config file
            env::set_var("BLACKBOX_CONFIG", config_path.to_str().unwrap());

            // Verify the environment variables are set correctly
            println!("Environment variables:");
            println!(
                "  BLACKBOX_AUDIO_CHANNELS: {:?}",
                env::var("BLACKBOX_AUDIO_CHANNELS")
            );
            println!("  BLACKBOX_DEBUG: {:?}", env::var("BLACKBOX_DEBUG"));

            // Test that environment variables take precedence over config file values
            let config = AppConfig::load();
            println!("Loaded config values:");
            println!("  audio_channels: {}", config.get_audio_channels());
            println!("  debug: {}", config.get_debug());

            assert_eq!(
                config.get_audio_channels(),
                "3,4,5",
                "Environment variable should override config file"
            );
            assert_eq!(
                config.get_debug(),
                true,
                "Environment variable should override config file"
            );
            assert_eq!(
                config.get_duration(),
                120,
                "Environment variable should override config file"
            );
            assert_eq!(
                config.get_output_mode(),
                "split",
                "Environment variable should override config file"
            );
            assert_eq!(
                config.get_silence_threshold(),
                0.001,
                "Environment variable should override config file"
            );
            assert_eq!(
                config.get_continuous_mode(),
                true,
                "Environment variable should override config file"
            );
            assert_eq!(
                config.get_recording_cadence(),
                600,
                "Environment variable should override config file"
            );
            assert_eq!(
                config.get_output_dir(),
                "/tmp/test_output",
                "Environment variable should override config file"
            );
            assert_eq!(
                config.get_performance_logging(),
                true,
                "Environment variable should override config file"
            );
        },
    );
}

#[test]
fn test_config_invalid_env_vars() {
    // Use temp_env to isolate the test environment
    temp_env::with_vars(
        [
            ("AUDIO_CHANNELS", None::<&str>),
            ("DEBUG", None::<&str>),
            ("RECORD_DURATION", None::<&str>),
            ("OUTPUT_MODE", None::<&str>),
            ("SILENCE_THRESHOLD", None::<&str>),
            ("CONTINUOUS_MODE", None::<&str>),
            ("RECORDING_CADENCE", None::<&str>),
            ("OUTPUT_DIR", None::<&str>),
            ("PERFORMANCE_LOGGING", None::<&str>),
            // Valid channel spec, will be used
            ("BLACKBOX_AUDIO_CHANNELS", Some("3,4,5")),
            // Invalid values, will fall back to defaults
            ("BLACKBOX_DEBUG", Some("not_a_bool")),
            ("BLACKBOX_DURATION", Some("not_a_number")),
            ("BLACKBOX_SILENCE_THRESHOLD", Some("not_a_float")),
            ("BLACKBOX_CONTINUOUS_MODE", Some("invalid")),
            ("BLACKBOX_RECORDING_CADENCE", Some("invalid")),
            ("BLACKBOX_PERFORMANCE_LOGGING", Some("not_bool")),
        ],
        || {
            let temp_dir = tempdir().unwrap();
            let config_path = temp_dir.path().join("blackbox.toml");

            // Create an empty config file
            fs::write(&config_path, "").unwrap();

            // Point to our test config file
            env::set_var("BLACKBOX_CONFIG", config_path.to_str().unwrap());

            let config = AppConfig::load();

            // Print values for debugging
            println!("Config values:");
            println!("  audio_channels: {}", config.get_audio_channels());
            println!("  debug: {}", config.get_debug());

            // Verify that valid channel specification is used,
            // but invalid values fall back to defaults
            assert_eq!(
                config.get_audio_channels(),
                "3,4,5",
                "Valid channel spec should be used"
            );
            assert_eq!(
                config.get_debug(),
                DEFAULT_DEBUG,
                "Invalid debug should fall back to default"
            );
            assert_eq!(
                config.get_duration(),
                DEFAULT_DURATION,
                "Invalid duration should fall back to default"
            );
            assert_eq!(
                config.get_output_mode(),
                DEFAULT_OUTPUT_MODE,
                "Default output mode should be used"
            );
            assert_eq!(
                config.get_silence_threshold(),
                DEFAULT_SILENCE_THRESHOLD,
                "Invalid threshold should fall back to default"
            );
            assert_eq!(
                config.get_continuous_mode(),
                DEFAULT_CONTINUOUS_MODE,
                "Invalid continuous mode should fall back to default"
            );
            assert_eq!(
                config.get_recording_cadence(),
                DEFAULT_RECORDING_CADENCE,
                "Invalid cadence should fall back to default"
            );
            assert_eq!(
                config.get_output_dir(),
                DEFAULT_OUTPUT_DIR,
                "Default output dir should be used"
            );
            assert_eq!(
                config.get_performance_logging(),
                DEFAULT_PERFORMANCE_LOGGING,
                "Invalid performance logging should fall back to default"
            );
        },
    );
}
