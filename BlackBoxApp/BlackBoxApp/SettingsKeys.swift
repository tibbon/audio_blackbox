import Foundation

/// Centralized `UserDefaults` keys for all persisted settings.
///
/// Referenced from `RecordingState`, `BlackBoxApp`, `OnboardingView`,
/// `SettingsView`, and tests. Extracted to its own file (DOLL-203)
/// because the cross-file usage made finding-by-filename impossible
/// when it lived inline at the bottom of `SettingsView.swift`.
///
/// **Adding a key**: declare it here, then bind via `@AppStorage(SettingsKeys.foo)`
/// at the call site. Don't introduce string literals elsewhere —
/// `CoreTests.testAllKeyValues` is the regression guard against
/// accidental renames that would orphan stored UserDefaults values.
enum SettingsKeys {
    static let inputDevice = "inputDevice"
    static let audioChannels = "audioChannels"
    static let outputMode = "outputMode"
    static let silenceEnabled = "silenceEnabled"
    static let silenceThreshold = "silenceThreshold"
    static let continuousMode = "continuousMode"
    static let recordingCadence = "recordingCadence"
    static let launchAtLogin = "launchAtLogin"
    static let autoRecord = "autoRecord"
    static let minDiskSpaceMB = "minDiskSpaceMB"
    static let hasCompletedOnboarding = "hasCompletedOnboarding"
    static let bitDepth = "bitDepth"
    static let lastOutputDirPath = "lastOutputDirPath"
    static let silenceGateEnabled = "silenceGateEnabled"
    static let silenceGateTimeout = "silenceGateTimeout"
    static let sleepBehavior = "sleepBehavior"
    static let preventSleep = "preventSleep"
}
