import Foundation

/// Builds the engine configuration from the settings saved in UserDefaults.
/// `RecordingState` pushes the result to Rust once at launch, before
/// auto-record can fire.
///
/// `nonisolated` and parameterized on the defaults store so tests can run it
/// against a scratch suite instead of the app's real preferences.
nonisolated enum SavedEngineConfig {
    /// The engine config the saved settings describe. Keys with no saved
    /// value are left out so the engine keeps its own default. Migrates a
    /// legacy 0-based channel spec in `defaults` to the 1-based form.
    static func restore(from defaults: UserDefaults) -> [String: Any] {
        var config: [String: Any] = [:]

        if let device = defaults.string(forKey: SettingsKeys.inputDevice), !device.isEmpty {
            config["input_device"] = device
        }
        if let channels = defaults.string(forKey: SettingsKeys.audioChannels) {
            if isLegacyZeroBasedSpec(channels) {
                // Migrate old 0-based spec to 1-based for UserDefaults
                let migrated = channelSpecToOneBased(channels)
                defaults.set(migrated, forKey: SettingsKeys.audioChannels)
                config["audio_channels"] = channels  // Already 0-based, pass directly
            } else {
                config["audio_channels"] = channelSpecToZeroBased(channels)
            }
        }
        config["output_mode"] = defaults.string(forKey: SettingsKeys.outputMode) ?? "split"

        // Silence threshold: reconstruct from enabled flag + threshold value
        let silenceEnabled = defaults.object(forKey: SettingsKeys.silenceEnabled) as? Bool ?? true
        let silenceThreshold = defaults.object(forKey: SettingsKeys.silenceThreshold) as? Double ?? 0.01
        config["silence_threshold"] = silenceEnabled ? silenceThreshold : 0.0

        // Output settings
        let continuousMode = defaults.object(forKey: SettingsKeys.continuousMode) as? Bool ?? false
        config["continuous_mode"] = continuousMode
        let cadence = defaults.integer(forKey: SettingsKeys.recordingCadence)
        if cadence > 0 {
            config["recording_cadence"] = cadence
        }

        // Disk space threshold. 0 is a real choice ("Disabled" in Settings),
        // so test for presence, not for a positive value: a `> 0` check
        // dropped it and the engine fell back to its 500 MB default on every
        // launch.
        if defaults.object(forKey: SettingsKeys.minDiskSpaceMB) != nil {
            config["min_disk_space_mb"] = defaults.integer(forKey: SettingsKeys.minDiskSpaceMB)
        }

        // Bit depth (0 means not yet set — use Rust default)
        let bitDepth = defaults.integer(forKey: SettingsKeys.bitDepth)
        if bitDepth > 0 {
            config["bits_per_sample"] = bitDepth
        }

        // Silence gate
        if defaults.object(forKey: SettingsKeys.silenceGateEnabled) != nil {
            config["silence_gate_enabled"] = defaults.bool(forKey: SettingsKeys.silenceGateEnabled)
        }
        let gateTimeout = defaults.integer(forKey: SettingsKeys.silenceGateTimeout)
        if gateTimeout > 0 {
            config["silence_gate_timeout_secs"] = gateTimeout
        }

        return config
    }
}
