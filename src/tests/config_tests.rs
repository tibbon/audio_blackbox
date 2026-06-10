use std::fs;
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

            // Point to our test config file via temp_env so it's restored
            // even if assertions panic.
            temp_env::with_var(
                "BLACKBOX_CONFIG",
                Some(config_path.to_str().unwrap()),
                || {
                    let config = AppConfig::load();

                    assert_eq!(config.get_audio_channels(), "1,2,3");
                    assert!(config.get_debug());
                    assert_eq!(config.get_duration(), 60);
                    assert_eq!(config.get_output_mode(), "split");
                    assert!((config.get_silence_threshold() - 0.05).abs() < f32::EPSILON);
                    assert!(config.get_continuous_mode());
                    assert_eq!(config.get_recording_cadence(), 1800);
                    assert_eq!(config.get_output_dir(), "/tmp");
                    assert!(config.get_performance_logging());
                },
            );
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
            assert!(
                (config.get_silence_threshold() - DEFAULT_SILENCE_THRESHOLD).abs() < f32::EPSILON
            );
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
            assert!(
                (config.get_silence_threshold() - DEFAULT_SILENCE_THRESHOLD).abs() < f32::EPSILON
            );
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
            ("BLACKBOX_DEBUG", Some("true")),
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

            temp_env::with_var(
                "BLACKBOX_CONFIG",
                Some(config_path.to_str().unwrap()),
                || {
                    let config = AppConfig::load();

                    println!("Config values:");
                    println!("  audio_channels: {}", config.get_audio_channels());
                    println!("  debug: {}", config.get_debug());

                    assert_eq!(
                        config.get_audio_channels(),
                        "0,1,2",
                        "Config should load audio_channels from file"
                    );
                    assert!(config.get_debug(), "Config should load debug from file");
                },
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

            temp_env::with_var(
                "BLACKBOX_CONFIG",
                Some(config_path.to_str().unwrap()),
                || {
                    println!("Environment variables:");
                    println!(
                        "  BLACKBOX_AUDIO_CHANNELS: {:?}",
                        std::env::var("BLACKBOX_AUDIO_CHANNELS")
                    );
                    println!("  BLACKBOX_DEBUG: {:?}", std::env::var("BLACKBOX_DEBUG"));

                    let config = AppConfig::load();
                    println!("Loaded config values:");
                    println!("  audio_channels: {}", config.get_audio_channels());
                    println!("  debug: {}", config.get_debug());

                    assert_eq!(
                        config.get_audio_channels(),
                        "3,4,5",
                        "Environment variable should override config file"
                    );
                    assert!(
                        config.get_debug(),
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
                    assert!(
                        (config.get_silence_threshold() - 0.001).abs() < f32::EPSILON,
                        "Environment variable should override config file"
                    );
                    assert!(
                        config.get_continuous_mode(),
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
                    assert!(
                        config.get_performance_logging(),
                        "Environment variable should override config file"
                    );
                },
            );
        },
    );
}

/// All env vars (prefixed + legacy) that touch the newer config fields,
/// cleared so tests see only what they set (DOLL-454).
fn newer_field_env_cleared() -> Vec<(&'static str, Option<&'static str>)> {
    vec![
        ("BLACKBOX_CONFIG", None),
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
    ]
}

/// DOLL-454: the newer fields (input_device, min_disk_space_mb,
/// bits_per_sample, silence_gate_*) load from TOML — config_tests previously
/// covered only the original 9 fields, so a broken serde rename or a typo'd
/// field name would ship green.
#[test]
fn test_newer_fields_from_toml() {
    temp_env::with_vars(newer_field_env_cleared(), || {
        let temp_dir = tempdir().unwrap();
        let config_path = temp_dir.path().join("blackbox.toml");
        fs::write(
            &config_path,
            r#"
            input_device = "Test Mic"
            min_disk_space_mb = 123
            bits_per_sample = 16
            silence_gate_enabled = false
            silence_gate_timeout_secs = 42
        "#,
        )
        .unwrap();

        temp_env::with_var(
            "BLACKBOX_CONFIG",
            Some(config_path.to_str().unwrap()),
            || {
                let config = AppConfig::load();
                assert_eq!(config.get_input_device(), Some("Test Mic".to_string()));
                assert_eq!(config.get_min_disk_space_mb(), 123);
                assert_eq!(config.get_bits_per_sample(), 16);
                assert!(!config.get_silence_gate_enabled());
                assert_eq!(config.get_silence_gate_timeout_secs(), 42);
            },
        );
    });
}

/// DOLL-454: each newer field's BLACKBOX_-prefixed env var must parse AND
/// take precedence over a conflicting TOML value. A typo in one env-var name
/// inside apply_env_vars would previously ship green.
#[test]
fn test_newer_fields_env_override_toml() {
    let mut vars = newer_field_env_cleared();
    for (name, val) in [
        ("BLACKBOX_INPUT_DEVICE", "Env Mic"),
        ("BLACKBOX_MIN_DISK_SPACE_MB", "777"),
        ("BLACKBOX_BITS_PER_SAMPLE", "32"),
        ("BLACKBOX_SILENCE_GATE_ENABLED", "true"),
        ("BLACKBOX_SILENCE_GATE_TIMEOUT_SECS", "99"),
    ] {
        if let Some(v) = vars.iter_mut().find(|(n, _)| *n == name) {
            v.1 = Some(val);
        }
    }

    temp_env::with_vars(vars, || {
        let temp_dir = tempdir().unwrap();
        let config_path = temp_dir.path().join("blackbox.toml");
        fs::write(
            &config_path,
            r#"
            input_device = "Toml Mic"
            min_disk_space_mb = 123
            bits_per_sample = 16
            silence_gate_enabled = false
            silence_gate_timeout_secs = 42
        "#,
        )
        .unwrap();

        temp_env::with_var(
            "BLACKBOX_CONFIG",
            Some(config_path.to_str().unwrap()),
            || {
                let config = AppConfig::load();
                assert_eq!(config.get_input_device(), Some("Env Mic".to_string()));
                assert_eq!(config.get_min_disk_space_mb(), 777);
                assert_eq!(config.get_bits_per_sample(), 32);
                assert!(config.get_silence_gate_enabled());
                assert_eq!(config.get_silence_gate_timeout_secs(), 99);
            },
        );
    });
}

/// DOLL-454: the unprefixed legacy names work when the prefixed ones are
/// absent, and the prefixed names win when both are set.
#[test]
fn test_newer_fields_legacy_env_names_and_prefix_precedence() {
    // Legacy-only: unprefixed names apply.
    let mut legacy = newer_field_env_cleared();
    for (name, val) in [
        ("INPUT_DEVICE", "Legacy Mic"),
        ("MIN_DISK_SPACE_MB", "55"),
        ("BITS_PER_SAMPLE", "16"),
        ("SILENCE_GATE_ENABLED", "false"),
        ("SILENCE_GATE_TIMEOUT_SECS", "11"),
    ] {
        if let Some(v) = legacy.iter_mut().find(|(n, _)| *n == name) {
            v.1 = Some(val);
        }
    }
    temp_env::with_vars(legacy, || {
        let config = AppConfig::load();
        assert_eq!(config.get_input_device(), Some("Legacy Mic".to_string()));
        assert_eq!(config.get_min_disk_space_mb(), 55);
        assert_eq!(config.get_bits_per_sample(), 16);
        assert!(!config.get_silence_gate_enabled());
        assert_eq!(config.get_silence_gate_timeout_secs(), 11);
    });

    // Both set: BLACKBOX_-prefixed wins.
    let mut both = newer_field_env_cleared();
    for (name, val) in [
        ("INPUT_DEVICE", "Legacy Mic"),
        ("BLACKBOX_INPUT_DEVICE", "Prefixed Mic"),
        ("MIN_DISK_SPACE_MB", "55"),
        ("BLACKBOX_MIN_DISK_SPACE_MB", "66"),
    ] {
        if let Some(v) = both.iter_mut().find(|(n, _)| *n == name) {
            v.1 = Some(val);
        }
    }
    temp_env::with_vars(both, || {
        let config = AppConfig::load();
        assert_eq!(config.get_input_device(), Some("Prefixed Mic".to_string()));
        assert_eq!(config.get_min_disk_space_mb(), 66);
    });
}

/// DOLL-454: unparseable env values for the newer fields must fall through to
/// the TOML tier (forgiving validation), and a parseable-but-invalid
/// bits_per_sample from env (e.g. 20) is rejected by the getter — falling
/// back to the DEFAULT (24), not to the TOML value it overrode.
#[test]
fn test_newer_fields_invalid_env_values_fall_back() {
    let mut vars = newer_field_env_cleared();
    for (name, val) in [
        ("BLACKBOX_MIN_DISK_SPACE_MB", "lots"),
        ("BLACKBOX_BITS_PER_SAMPLE", "high"),
        ("BLACKBOX_SILENCE_GATE_ENABLED", "maybe"),
        ("BLACKBOX_SILENCE_GATE_TIMEOUT_SECS", "-5"),
    ] {
        if let Some(v) = vars.iter_mut().find(|(n, _)| *n == name) {
            v.1 = Some(val);
        }
    }

    temp_env::with_vars(vars, || {
        let temp_dir = tempdir().unwrap();
        let config_path = temp_dir.path().join("blackbox.toml");
        fs::write(
            &config_path,
            "
            min_disk_space_mb = 123
            bits_per_sample = 16
            silence_gate_enabled = false
            silence_gate_timeout_secs = 42
        ",
        )
        .unwrap();

        temp_env::with_var(
            "BLACKBOX_CONFIG",
            Some(config_path.to_str().unwrap()),
            || {
                let config = AppConfig::load();
                assert_eq!(
                    config.get_min_disk_space_mb(),
                    123,
                    "unparseable env must not clobber the TOML value"
                );
                assert_eq!(config.get_bits_per_sample(), 16);
                assert!(!config.get_silence_gate_enabled());
                assert_eq!(config.get_silence_gate_timeout_secs(), 42);
            },
        );
    });

    // Parseable-but-invalid bits_per_sample (20) DOES override the TOML tier,
    // then the getter's 16/24/32 validation kicks in → default 24, not 16.
    let mut bits20 = newer_field_env_cleared();
    if let Some(v) = bits20
        .iter_mut()
        .find(|(n, _)| *n == "BLACKBOX_BITS_PER_SAMPLE")
    {
        v.1 = Some("20");
    }
    temp_env::with_vars(bits20, || {
        let temp_dir = tempdir().unwrap();
        let config_path = temp_dir.path().join("blackbox.toml");
        fs::write(&config_path, "bits_per_sample = 16\n").unwrap();

        temp_env::with_var(
            "BLACKBOX_CONFIG",
            Some(config_path.to_str().unwrap()),
            || {
                let config = AppConfig::load();
                assert_eq!(config.bits_per_sample, Some(20), "env value lands raw");
                assert_eq!(
                    config.get_bits_per_sample(),
                    DEFAULT_BITS_PER_SAMPLE,
                    "getter must reject 20 and fall back to the default"
                );
            },
        );
    });
}

/// DOLL-454: merge() must carry each NEWER field across when Some — the
/// existing merge tests only assert that None doesn't override.
#[test]
fn test_merge_carries_newer_fields() {
    let mut base = AppConfig::default();
    let overlay = AppConfig {
        audio_channels: None,
        debug: None,
        duration: None,
        output_mode: None,
        silence_threshold: None,
        continuous_mode: None,
        recording_cadence: None,
        output_dir: None,
        performance_logging: None,
        input_device: Some("Overlay Mic".to_string()),
        min_disk_space_mb: Some(999),
        bits_per_sample: Some(32),
        silence_gate_enabled: Some(false),
        silence_gate_timeout_secs: Some(7),
    };

    base.merge(overlay);

    assert_eq!(base.input_device, Some("Overlay Mic".to_string()));
    assert_eq!(base.min_disk_space_mb, Some(999));
    assert_eq!(base.bits_per_sample, Some(32));
    assert_eq!(base.silence_gate_enabled, Some(false));
    assert_eq!(base.silence_gate_timeout_secs, Some(7));
}

#[test]
fn test_config_invalid_env_vars() {
    // Use temp_env to isolate the test environment
    temp_env::with_vars(
        [
            ("AUDIO_CHANNELS", None::<&str>),
            ("DEBUG", Some("invalid")),
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
            audio_channels = "3,4,5"
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

            temp_env::with_var(
                "BLACKBOX_CONFIG",
                Some(config_path.to_str().unwrap()),
                || {
                    let config = AppConfig::load();

                    println!("Config values:");
                    println!("  audio_channels: {}", config.get_audio_channels());
                    println!("  debug: {}", config.get_debug());

                    assert_eq!(
                        config.get_audio_channels(),
                        "3,4,5",
                        "Config should load audio_channels from file"
                    );
                    assert_eq!(
                        config.get_debug(),
                        DEFAULT_DEBUG,
                        "Invalid debug should fall back to default"
                    );
                },
            );
        },
    );
}
