import Foundation

/// The recording-mode settings the onboarding wizard reads and writes.
/// Extracted from `OnboardingView` so the first-run and "Run Setup Again"
/// behavior can be tested against a scratch defaults suite.
///
/// `nonisolated`: pure functions over the defaults store they are handed.
nonisolated enum OnboardingSettings {
    /// Rotation interval the wizard sets when it turns continuous recording
    /// on and none has been chosen yet: hourly files.
    static let recommendedCadenceSecs = 3600

    /// Rotation interval the engine gets when none is saved and continuous
    /// recording is off (matches the Output tab's default).
    static let fallbackCadenceSecs = 300

    /// The recording-mode step's starting state: the saved choices on a
    /// re-run, the recommended setup (continuous, auto-split on silence) on
    /// the first run. Starting from the recommendation on a re-run used to
    /// switch a user who had turned either off back on.
    static func initialRecordingMode(from defaults: UserDefaults) -> (continuous: Bool, silenceGate: Bool) {
        (
            continuous: defaults.object(forKey: SettingsKeys.continuousMode) as? Bool ?? true,
            silenceGate: defaults.object(forKey: SettingsKeys.silenceGateEnabled) as? Bool ?? true
        )
    }

    /// Save the wizard's recording mode and return the engine config for
    /// it. A saved rotation interval is kept: only when continuous recording
    /// is on and no interval was ever chosen is the recommended hourly one
    /// saved (the wizard used to force 3600 s on every run).
    static func applyRecordingMode(
        continuous: Bool,
        silenceGate: Bool,
        to defaults: UserDefaults
    ) -> [String: Any] {
        defaults.set(continuous, forKey: SettingsKeys.continuousMode)
        if continuous, defaults.object(forKey: SettingsKeys.recordingCadence) == nil {
            defaults.set(recommendedCadenceSecs, forKey: SettingsKeys.recordingCadence)
        }
        defaults.set(silenceGate, forKey: SettingsKeys.silenceGateEnabled)
        let cadence = defaults.integer(forKey: SettingsKeys.recordingCadence)
        return [
            "continuous_mode": continuous,
            "recording_cadence": cadence > 0 ? cadence : fallbackCadenceSecs,
            "silence_gate_enabled": silenceGate,
        ]
    }

    /// "Skip Setup": the recommended setup on the first run, and on a re-run
    /// whatever is already saved (Skip must not reset it).
    static func applySkip(to defaults: UserDefaults) -> [String: Any] {
        let mode = initialRecordingMode(from: defaults)
        return applyRecordingMode(continuous: mode.continuous, silenceGate: mode.silenceGate, to: defaults)
    }
}
