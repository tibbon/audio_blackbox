import XCTest

@testable import BlackBox_Audio_Recorder

/// Onboarding's recording-mode handling, against a throwaway defaults suite.
nonisolated final class OnboardingSettingsTests: XCTestCase {
    private var suiteName = ""
    private var defaults = UserDefaults()

    override func setUp() {
        super.setUp()
        suiteName = "OnboardingSettingsTests.\(UUID().uuidString)"
        defaults = UserDefaults(suiteName: suiteName) ?? UserDefaults()
    }

    override func tearDown() {
        defaults.removePersistentDomain(forName: suiteName)
        super.tearDown()
    }

    func testFirstRunStartsFromTheRecommendation() {
        let mode = OnboardingSettings.initialRecordingMode(from: defaults)
        XCTAssertTrue(mode.continuous)
        XCTAssertTrue(mode.silenceGate)
    }

    /// "Run Setup Again" must start from what the user chose, not switch
    /// continuous recording and auto-split back on.
    func testRerunStartsFromTheSavedChoices() {
        defaults.set(false, forKey: SettingsKeys.continuousMode)
        defaults.set(false, forKey: SettingsKeys.silenceGateEnabled)
        let mode = OnboardingSettings.initialRecordingMode(from: defaults)
        XCTAssertFalse(mode.continuous)
        XCTAssertFalse(mode.silenceGate)
    }

    func testFirstRunContinuousSavesHourlyRotation() {
        let config = OnboardingSettings.applyRecordingMode(continuous: true, silenceGate: true, to: defaults)
        XCTAssertEqual(defaults.integer(forKey: SettingsKeys.recordingCadence), 3600)
        XCTAssertEqual(config["recording_cadence"] as? Int, 3600)
        XCTAssertEqual(config["continuous_mode"] as? Bool, true)
    }

    /// A re-run used to force the rotation interval back to one hour.
    func testRerunKeepsTheSavedRotationInterval() {
        defaults.set(900, forKey: SettingsKeys.recordingCadence)
        let config = OnboardingSettings.applyRecordingMode(continuous: true, silenceGate: false, to: defaults)
        XCTAssertEqual(defaults.integer(forKey: SettingsKeys.recordingCadence), 900)
        XCTAssertEqual(config["recording_cadence"] as? Int, 900)
        XCTAssertEqual(config["silence_gate_enabled"] as? Bool, false)
    }

    func testFirstRunSkipAppliesTheRecommendation() {
        let config = OnboardingSettings.applySkip(to: defaults)
        XCTAssertTrue(defaults.bool(forKey: SettingsKeys.continuousMode))
        XCTAssertTrue(defaults.bool(forKey: SettingsKeys.silenceGateEnabled))
        XCTAssertEqual(config["recording_cadence"] as? Int, 3600)
    }

    /// Skip on a re-run keeps the saved mode instead of resetting it.
    func testRerunSkipKeepsTheSavedMode() {
        defaults.set(false, forKey: SettingsKeys.continuousMode)
        defaults.set(false, forKey: SettingsKeys.silenceGateEnabled)
        defaults.set(1800, forKey: SettingsKeys.recordingCadence)
        let config = OnboardingSettings.applySkip(to: defaults)
        XCTAssertFalse(defaults.bool(forKey: SettingsKeys.continuousMode))
        XCTAssertFalse(defaults.bool(forKey: SettingsKeys.silenceGateEnabled))
        XCTAssertEqual(defaults.integer(forKey: SettingsKeys.recordingCadence), 1800)
        XCTAssertEqual(config["continuous_mode"] as? Bool, false)
    }
}
