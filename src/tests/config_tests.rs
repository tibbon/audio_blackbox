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
            config1.audio_channels = Some("0".to_owned());
            config1.debug = Some(true);

            // Set different values in config2
            config2.audio_channels = Some("1,2".to_owned()); // Different from config1
            config2.debug = Some(false); // Different from config1
            config2.duration = Some(60);
            config2.output_mode = Some("split".to_owned());

            // Merge config2 into config1
            config1.merge(config2);

            // Verify merged values - config2 values should overwrite config1
            assert_eq!(config1.audio_channels, Some("1,2".to_owned()));
            assert_eq!(config1.debug, Some(false));
            assert_eq!(config1.duration, Some(60));
            assert_eq!(config1.output_mode, Some("split".to_owned()));
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
            assert_eq!(config.audio_channels, Some(DEFAULT_CHANNELS.to_owned()));
            assert_eq!(config.debug, Some(DEFAULT_DEBUG));
            assert_eq!(config.duration, Some(DEFAULT_DURATION));
            assert_eq!(config.output_mode, Some(DEFAULT_OUTPUT_MODE.to_owned()));
            assert!(
                (config.get_silence_threshold() - DEFAULT_SILENCE_THRESHOLD).abs() < f32::EPSILON
            );
            assert_eq!(config.continuous_mode, Some(DEFAULT_CONTINUOUS_MODE));
            assert_eq!(config.recording_cadence, Some(DEFAULT_RECORDING_CADENCE));
            assert_eq!(config.output_dir, Some(DEFAULT_OUTPUT_DIR.to_owned()));
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

/// Legacy names cleared and every `BLACKBOX_*` override for the original nine
/// fields set to a value that differs from `PRECEDENCE_FILE`.
const PRECEDENCE_ENV: [(&str, Option<&str>); 18] = [
    ("AUDIO_CHANNELS", None),
    ("DEBUG", None),
    ("RECORD_DURATION", None),
    ("OUTPUT_MODE", None),
    ("SILENCE_THRESHOLD", None),
    ("CONTINUOUS_MODE", None),
    ("RECORDING_CADENCE", None),
    ("OUTPUT_DIR", None),
    ("PERFORMANCE_LOGGING", None),
    ("BLACKBOX_AUDIO_CHANNELS", Some("3,4,5")),
    ("BLACKBOX_DEBUG", Some("true")),
    ("BLACKBOX_DURATION", Some("120")),
    ("BLACKBOX_OUTPUT_MODE", Some("split")),
    ("BLACKBOX_SILENCE_THRESHOLD", Some("0.001")),
    ("BLACKBOX_CONTINUOUS_MODE", Some("true")),
    ("BLACKBOX_RECORDING_CADENCE", Some("600")),
    ("BLACKBOX_OUTPUT_DIR", Some("/tmp/test_output")),
    ("BLACKBOX_PERFORMANCE_LOGGING", Some("true")),
];

/// Config file values that `PRECEDENCE_ENV` should override.
const PRECEDENCE_FILE: &str = r#"
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

#[test]
fn test_config_env_vars_precedence() {
    // Use temp_env to isolate the test environment
    temp_env::with_vars(PRECEDENCE_ENV, || {
        let temp_dir = tempdir().unwrap();
        let config_path = temp_dir.path().join("blackbox.toml");
        fs::write(&config_path, PRECEDENCE_FILE).unwrap();

        temp_env::with_var(
            "BLACKBOX_CONFIG",
            Some(config_path.to_str().unwrap()),
            || assert_env_overrides_file(&AppConfig::load()),
        );
    });
}

/// Every field loaded under `PRECEDENCE_ENV` carries the environment value.
fn assert_env_overrides_file(config: &AppConfig) {
    const WHY: &str = "Environment variable should override config file";
    assert_eq!(config.get_audio_channels(), "3,4,5", "{WHY}");
    assert!(config.get_debug(), "{WHY}");
    assert_eq!(config.get_duration(), 120, "{WHY}");
    assert_eq!(config.get_output_mode(), "split", "{WHY}");
    assert!(
        (config.get_silence_threshold() - 0.001).abs() < f32::EPSILON,
        "{WHY}"
    );
    assert!(config.get_continuous_mode(), "{WHY}");
    assert_eq!(config.get_recording_cadence(), 600, "{WHY}");
    assert_eq!(config.get_output_dir(), "/tmp/test_output", "{WHY}");
    assert!(config.get_performance_logging(), "{WHY}");
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

/// DOLL-454: the newer fields (`input_device`, `min_disk_space_mb`,
/// `bits_per_sample`, `silence_gate_*`) load from TOML — `config_tests` previously
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
                assert_eq!(config.get_input_device(), Some("Test Mic".to_owned()));
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
/// inside `apply_env_vars` would previously ship green.
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
                assert_eq!(config.get_input_device(), Some("Env Mic".to_owned()));
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
        assert_eq!(config.get_input_device(), Some("Legacy Mic".to_owned()));
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
        assert_eq!(config.get_input_device(), Some("Prefixed Mic".to_owned()));
        assert_eq!(config.get_min_disk_space_mb(), 66);
    });
}

/// DOLL-454: unparseable env values for the newer fields must fall through to
/// the TOML tier (forgiving validation), and a parseable-but-invalid
/// `bits_per_sample` from env (e.g. 20) is rejected by the getter — falling
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

/// DOLL-454: `merge()` must carry each NEWER field across when Some — the
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
        input_device: Some("Overlay Mic".to_owned()),
        min_disk_space_mb: Some(999),
        bits_per_sample: Some(32),
        silence_gate_enabled: Some(false),
        silence_gate_timeout_secs: Some(7),
    };

    base.merge(overlay);

    assert_eq!(base.input_device, Some("Overlay Mic".to_owned()));
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

/// `blackbox.example.toml` is what users copy to configure the CLI, so it
/// must set every key (a misspelled key would be silently ignored, since
/// `AppConfig` does not deny unknown fields) and show the real defaults.
#[test]
fn example_config_sets_every_key_to_its_default() {
    let config: AppConfig = toml::from_str(include_str!("../../blackbox.example.toml"))
        .expect("blackbox.example.toml must parse");

    assert_eq!(config.audio_channels.as_deref(), Some(DEFAULT_CHANNELS));
    assert_eq!(config.debug, Some(DEFAULT_DEBUG));
    assert_eq!(config.duration, Some(DEFAULT_DURATION));
    assert_eq!(config.output_mode.as_deref(), Some(DEFAULT_OUTPUT_MODE));
    let threshold = config
        .silence_threshold
        .expect("silence_threshold must be set");
    assert!((threshold - DEFAULT_SILENCE_THRESHOLD).abs() < f32::EPSILON);
    assert_eq!(config.continuous_mode, Some(DEFAULT_CONTINUOUS_MODE));
    assert_eq!(config.recording_cadence, Some(DEFAULT_RECORDING_CADENCE));
    assert_eq!(config.output_dir.as_deref(), Some(DEFAULT_OUTPUT_DIR));
    assert_eq!(
        config.performance_logging,
        Some(DEFAULT_PERFORMANCE_LOGGING)
    );
    assert_eq!(config.min_disk_space_mb, Some(DEFAULT_MIN_DISK_SPACE_MB));
    assert_eq!(config.bits_per_sample, Some(DEFAULT_BITS_PER_SAMPLE));
    assert_eq!(
        config.silence_gate_enabled,
        Some(DEFAULT_SILENCE_GATE_ENABLED)
    );
    assert_eq!(
        config.silence_gate_timeout_secs,
        Some(DEFAULT_SILENCE_GATE_TIMEOUT_SECS)
    );
    // Left commented out on purpose: unset means the system default input.
    assert_eq!(config.input_device, None);
}

/// A cadence so large that `sample_rate * channels * cadence` overflows u64
/// is rejected like 0: the wrapped rotation threshold was arbitrary (often
/// tiny, a rotation storm). The largest accepted cadence cannot overflow
/// even at `u32::MAX` Hz with `MAX_CHANNELS` channels.
#[test]
fn recording_cadence_rejects_overflowing_values() {
    for bad in [MAX_RECORDING_CADENCE + 1, u64::MAX / 2, u64::MAX] {
        let config = AppConfig {
            recording_cadence: Some(bad),
            ..AppConfig::default()
        };
        assert_eq!(
            config.get_recording_cadence(),
            DEFAULT_RECORDING_CADENCE,
            "cadence {bad} must fall back to the default"
        );
    }
    let max = AppConfig {
        recording_cadence: Some(MAX_RECORDING_CADENCE),
        ..AppConfig::default()
    };
    assert_eq!(max.get_recording_cadence(), MAX_RECORDING_CADENCE);
    assert!(
        u64::from(u32::MAX)
            .checked_mul(MAX_CHANNELS as u64)
            .and_then(|v| v.checked_mul(MAX_RECORDING_CADENCE))
            .is_some(),
        "the largest accepted cadence must not overflow the threshold"
    );
}

/// A zero gate timeout would close the gate after the first silent batch and
/// split a take at every pause; it falls back to the default.
#[test]
fn silence_gate_timeout_rejects_zero() {
    let zero = AppConfig {
        silence_gate_timeout_secs: Some(0),
        ..AppConfig::default()
    };
    assert_eq!(
        zero.get_silence_gate_timeout_secs(),
        DEFAULT_SILENCE_GATE_TIMEOUT_SECS
    );
    for good in [1, 5, 300, 86_400] {
        let config = AppConfig {
            silence_gate_timeout_secs: Some(good),
            ..AppConfig::default()
        };
        assert_eq!(config.get_silence_gate_timeout_secs(), good);
    }
}

/// Misspelled keys in the TOML file are reported (the file still loads).
#[test]
fn unknown_toml_keys_are_detected() {
    let content = "recording_cadance = 60\nduration = 10\n[extra]\nx = 1\n";
    assert_eq!(
        crate::config::unknown_config_keys(content),
        vec!["extra".to_owned(), "recording_cadance".to_owned()]
    );
    let parsed: AppConfig = toml::from_str(content).expect("unknown keys stay non-fatal");
    assert_eq!(parsed.duration, Some(10));
    assert!(crate::config::unknown_config_keys("not [[[ toml").is_empty());
    assert!(
        crate::config::unknown_config_keys(include_str!("../../blackbox.example.toml")).is_empty(),
        "the example config must use only known keys"
    );
    assert!(crate::config::unknown_config_keys(&AppConfig::generate_sample_config()).is_empty());
}

/// `CONFIG_KEYS` lists exactly the fields `AppConfig` serializes, so a new
/// field can't be flagged as unknown (or a removed one kept as known).
#[test]
fn config_keys_match_the_struct() {
    let full = AppConfig {
        input_device: Some("mic".to_owned()),
        ..AppConfig::default()
    };
    let table: toml::Table = toml::from_str(&toml::to_string(&full).unwrap()).unwrap();
    let mut from_struct: Vec<&str> = table.keys().map(String::as_str).collect();
    let mut listed = crate::config::CONFIG_KEYS.to_vec();
    from_struct.sort_unstable();
    listed.sort_unstable();
    assert_eq!(from_struct, listed);
}

/// The unknown-key warning comes from `AppConfig::load` on a real file, and
/// the rest of the file still applies.
#[test]
fn load_keeps_known_keys_when_one_is_misspelled() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("typo.toml");
    fs::write(&path, "duraton = 5\nduration = 7\n").unwrap();
    temp_env::with_vars(
        vec![
            ("BLACKBOX_CONFIG", Some(path.to_str().unwrap())),
            ("BLACKBOX_DURATION", None),
            ("RECORD_DURATION", None),
        ],
        || {
            assert_eq!(AppConfig::load().get_duration(), 7);
        },
    );
}
