use crate::constants::*;
use crate::AppConfig;
use std::env;
use std::fs;
use tempfile::tempdir;

#[test]
fn test_config_loading() {
    let temp_dir = tempdir().unwrap();
    let config_path = temp_dir.path().join("blackbox.toml");

    // Clean up any existing environment variables
    env::remove_var("BLACKBOX_CONFIG");
    env::remove_var("BLACKBOX_AUDIO_CHANNELS");
    env::remove_var("BLACKBOX_DEBUG");
    env::remove_var("BLACKBOX_DURATION");
    env::remove_var("BLACKBOX_OUTPUT_MODE");
    env::remove_var("BLACKBOX_SILENCE_THRESHOLD");
    env::remove_var("BLACKBOX_CONTINUOUS_MODE");
    env::remove_var("BLACKBOX_RECORDING_CADENCE");
    env::remove_var("BLACKBOX_OUTPUT_DIR");
    env::remove_var("BLACKBOX_PERFORMANCE_LOGGING");

    // Create a test config file
    let config_content = r#"
        audio_channels = "3,4,5"
        debug = true
        duration = 120
        output_mode = "split"
        silence_threshold = 0.001
        continuous_mode = true
        recording_cadence = 600
        output_dir = "/tmp/test_output"
        performance_logging = true
    "#;

    fs::write(&config_path, config_content).unwrap();

    // Set environment variables to override config file values
    env::set_var("BLACKBOX_CONFIG", config_path.to_str().unwrap());
    env::set_var("AUDIO_CHANNELS", "3,4,5");
    env::set_var("BLACKBOX_AUDIO_CHANNELS", "3,4,5");
    env::set_var("DEBUG", "true");
    env::set_var("BLACKBOX_DEBUG", "true");
    env::set_var("RECORD_DURATION", "120");
    env::set_var("BLACKBOX_DURATION", "120");
    env::set_var("OUTPUT_MODE", "split");
    env::set_var("BLACKBOX_OUTPUT_MODE", "split");
    env::set_var("SILENCE_THRESHOLD", "0.001");
    env::set_var("BLACKBOX_SILENCE_THRESHOLD", "0.001");
    env::set_var("CONTINUOUS_MODE", "true");
    env::set_var("BLACKBOX_CONTINUOUS_MODE", "true");
    env::set_var("RECORDING_CADENCE", "600");
    env::set_var("BLACKBOX_RECORDING_CADENCE", "600");
    env::set_var("OUTPUT_DIR", "/tmp/test_output");
    env::set_var("BLACKBOX_OUTPUT_DIR", "/tmp/test_output");
    env::set_var("PERFORMANCE_LOGGING", "true");
    env::set_var("BLACKBOX_PERFORMANCE_LOGGING", "true");

    let config = AppConfig::load();

    // Test environment variables take precedence
    assert_eq!(config.get_audio_channels(), "3,4,5");
    assert_eq!(config.get_debug(), true);
    assert_eq!(config.get_duration(), 120);
    assert_eq!(config.get_output_mode(), "split");
    assert_eq!(config.get_silence_threshold(), 0.001);
    assert_eq!(config.get_continuous_mode(), true);
    assert_eq!(config.get_output_dir(), "/tmp/test_output");
    assert_eq!(config.get_performance_logging(), true);
    assert_eq!(config.get_recording_cadence(), 600);

    // Clean up environment variables
    env::remove_var("BLACKBOX_CONFIG");
    env::remove_var("AUDIO_CHANNELS");
    env::remove_var("BLACKBOX_AUDIO_CHANNELS");
    env::remove_var("DEBUG");
    env::remove_var("BLACKBOX_DEBUG");
    env::remove_var("RECORD_DURATION");
    env::remove_var("BLACKBOX_DURATION");
    env::remove_var("OUTPUT_MODE");
    env::remove_var("BLACKBOX_OUTPUT_MODE");
    env::remove_var("SILENCE_THRESHOLD");
    env::remove_var("BLACKBOX_SILENCE_THRESHOLD");
    env::remove_var("CONTINUOUS_MODE");
    env::remove_var("BLACKBOX_CONTINUOUS_MODE");
    env::remove_var("RECORDING_CADENCE");
    env::remove_var("BLACKBOX_RECORDING_CADENCE");
    env::remove_var("OUTPUT_DIR");
    env::remove_var("BLACKBOX_OUTPUT_DIR");
    env::remove_var("PERFORMANCE_LOGGING");
    env::remove_var("BLACKBOX_PERFORMANCE_LOGGING");
}

#[test]
fn test_config_merge() {
    let mut config1 = AppConfig::default();
    config1.audio_channels = Some("0".to_string());
    config1.debug = Some(true);

    let mut config2 = AppConfig::default();
    config2.debug = Some(false);
    config2.duration = Some(60);

    config1.merge(config2);

    assert_eq!(config1.get_audio_channels(), "0"); // Should keep original value
    assert_eq!(config1.get_debug(), false); // Should be overridden
    assert_eq!(config1.get_duration(), 60); // Should be set from config2
}

#[test]
fn test_config_defaults() {
    let config = AppConfig::default();

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
}

#[test]
fn test_config_file_creation() {
    let temp_dir = tempdir().unwrap();
    let config_path = temp_dir.path().join("new_config.toml");

    let config = AppConfig::default();
    assert!(config
        .create_config_file(config_path.to_str().unwrap())
        .is_ok());

    // Verify file was created and contains expected content
    let content = fs::read_to_string(&config_path).unwrap();
    assert!(content.contains(&format!("audio_channels = \"{}\"", DEFAULT_CHANNELS)));
    assert!(content.contains(&format!("debug = {}", DEFAULT_DEBUG)));
    assert!(content.contains(&format!("duration = {}", DEFAULT_DURATION)));
}

#[test]
fn test_config_env_vars() {
    // Clean up any existing environment variables
    env::remove_var("AUDIO_CHANNELS");
    env::remove_var("DEBUG");
    env::remove_var("RECORD_DURATION");
    env::remove_var("OUTPUT_MODE");
    env::remove_var("SILENCE_THRESHOLD");
    env::remove_var("CONTINUOUS_MODE");
    env::remove_var("RECORDING_CADENCE");
    env::remove_var("OUTPUT_DIR");
    env::remove_var("PERFORMANCE_LOGGING");
    env::remove_var("BLACKBOX_AUDIO_CHANNELS");
    env::remove_var("BLACKBOX_DEBUG");
    env::remove_var("BLACKBOX_DURATION");
    env::remove_var("BLACKBOX_OUTPUT_MODE");
    env::remove_var("BLACKBOX_SILENCE_THRESHOLD");
    env::remove_var("BLACKBOX_CONTINUOUS_MODE");
    env::remove_var("BLACKBOX_RECORDING_CADENCE");
    env::remove_var("BLACKBOX_OUTPUT_DIR");
    env::remove_var("BLACKBOX_PERFORMANCE_LOGGING");

    // Set test environment variables with explicit values
    env::set_var("AUDIO_CHANNELS", "3,4,5");
    env::set_var("DEBUG", "true");
    env::set_var("RECORD_DURATION", "120");
    env::set_var("OUTPUT_MODE", "split");
    env::set_var("SILENCE_THRESHOLD", "0.001");
    env::set_var("CONTINUOUS_MODE", "true");
    env::set_var("RECORDING_CADENCE", "600");
    env::set_var("OUTPUT_DIR", "/tmp/test_output");
    env::set_var("PERFORMANCE_LOGGING", "true");

    let config = AppConfig::load();

    assert_eq!(config.get_audio_channels(), "3,4,5");
    assert_eq!(config.get_debug(), true);
    assert_eq!(config.get_duration(), 120);
    assert_eq!(config.get_output_mode(), "split");
    assert_eq!(config.get_silence_threshold(), 0.001);
    assert_eq!(config.get_continuous_mode(), true);
    assert_eq!(config.get_recording_cadence(), 600);
    assert_eq!(config.get_output_dir(), "/tmp/test_output");
    assert_eq!(config.get_performance_logging(), true);

    // Clean up environment
    env::remove_var("AUDIO_CHANNELS");
    env::remove_var("DEBUG");
    env::remove_var("RECORD_DURATION");
    env::remove_var("OUTPUT_MODE");
    env::remove_var("SILENCE_THRESHOLD");
    env::remove_var("CONTINUOUS_MODE");
    env::remove_var("RECORDING_CADENCE");
    env::remove_var("OUTPUT_DIR");
    env::remove_var("PERFORMANCE_LOGGING");
    env::remove_var("BLACKBOX_AUDIO_CHANNELS");
    env::remove_var("BLACKBOX_DEBUG");
    env::remove_var("BLACKBOX_DURATION");
    env::remove_var("BLACKBOX_OUTPUT_MODE");
    env::remove_var("BLACKBOX_SILENCE_THRESHOLD");
    env::remove_var("BLACKBOX_CONTINUOUS_MODE");
    env::remove_var("BLACKBOX_RECORDING_CADENCE");
    env::remove_var("BLACKBOX_OUTPUT_DIR");
    env::remove_var("BLACKBOX_PERFORMANCE_LOGGING");
}
