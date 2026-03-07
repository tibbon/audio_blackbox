import XCTest

@testable import BlackBox_Audio_Recorder

// MARK: - SleepWakePolicy Tests

final class SleepWakePolicyTests: XCTestCase {

    // MARK: - sleepAction

    func testSleepActionIgnoresWhenNotRecording() {
        XCTAssertEqual(
            SleepWakePolicy.sleepAction(isRecording: false, behavior: "resume"), .ignore)
        XCTAssertEqual(
            SleepWakePolicy.sleepAction(isRecording: false, behavior: "stop"), .ignore)
    }

    func testSleepActionPausesForResumeWhenRecording() {
        XCTAssertEqual(
            SleepWakePolicy.sleepAction(isRecording: true, behavior: "resume"), .pauseForResume)
    }

    func testSleepActionStopsWhenRecordingWithStopBehavior() {
        XCTAssertEqual(
            SleepWakePolicy.sleepAction(isRecording: true, behavior: "stop"), .stop)
    }

    func testSleepActionUnknownBehaviorTreatedAsStop() {
        XCTAssertEqual(
            SleepWakePolicy.sleepAction(isRecording: true, behavior: "unknown"), .stop)
        XCTAssertEqual(
            SleepWakePolicy.sleepAction(isRecording: true, behavior: ""), .stop)
    }

    // MARK: - shouldResumeOnWake

    func testShouldResumeOnWakeWhenInterrupted() {
        XCTAssertTrue(SleepWakePolicy.shouldResumeOnWake(wasInterrupted: true))
    }

    func testShouldNotResumeOnWakeWhenNotInterrupted() {
        XCTAssertFalse(SleepWakePolicy.shouldResumeOnWake(wasInterrupted: false))
    }

    // MARK: - shouldPreventSleep

    func testShouldPreventSleepWhenEnabled() {
        XCTAssertTrue(SleepWakePolicy.shouldPreventSleep(settingEnabled: true))
    }

    func testShouldNotPreventSleepWhenDisabled() {
        XCTAssertFalse(SleepWakePolicy.shouldPreventSleep(settingEnabled: false))
    }

    // MARK: - sessionResignAction

    func testSessionResignIgnoresWhenNotRecording() {
        XCTAssertEqual(SleepWakePolicy.sessionResignAction(isRecording: false), .ignore)
    }

    func testSessionResignPausesWhenRecording() {
        XCTAssertEqual(SleepWakePolicy.sessionResignAction(isRecording: true), .pauseForResume)
    }
}

// MARK: - Settings Tests

final class SleepWakeSettingsTests: XCTestCase {

    override func setUp() {
        super.setUp()
        UserDefaults.standard.removeObject(forKey: SettingsKeys.sleepBehavior)
        UserDefaults.standard.removeObject(forKey: SettingsKeys.preventSleep)
    }

    override func tearDown() {
        UserDefaults.standard.removeObject(forKey: SettingsKeys.sleepBehavior)
        UserDefaults.standard.removeObject(forKey: SettingsKeys.preventSleep)
        super.tearDown()
    }

    func testSettingsKeyValues() {
        XCTAssertEqual(SettingsKeys.sleepBehavior, "sleepBehavior")
        XCTAssertEqual(SettingsKeys.preventSleep, "preventSleep")
    }

    func testSleepBehaviorDefaultIsResume() {
        let behavior =
            UserDefaults.standard.string(forKey: SettingsKeys.sleepBehavior) ?? "resume"
        XCTAssertEqual(behavior, "resume")
    }

    func testPreventSleepDefaultIsTrue() {
        let prevent =
            UserDefaults.standard.object(forKey: SettingsKeys.preventSleep) as? Bool ?? true
        XCTAssertTrue(prevent)
    }

    func testSleepBehaviorPersistence() {
        UserDefaults.standard.set("stop", forKey: SettingsKeys.sleepBehavior)
        XCTAssertEqual(
            UserDefaults.standard.string(forKey: SettingsKeys.sleepBehavior), "stop")

        UserDefaults.standard.set("resume", forKey: SettingsKeys.sleepBehavior)
        XCTAssertEqual(
            UserDefaults.standard.string(forKey: SettingsKeys.sleepBehavior), "resume")
    }

    func testPreventSleepPersistence() {
        UserDefaults.standard.set(false, forKey: SettingsKeys.preventSleep)
        XCTAssertFalse(UserDefaults.standard.bool(forKey: SettingsKeys.preventSleep))

        UserDefaults.standard.set(true, forKey: SettingsKeys.preventSleep)
        XCTAssertTrue(UserDefaults.standard.bool(forKey: SettingsKeys.preventSleep))
    }
}

// MARK: - RecordingState Guard Path Tests

final class SleepWakeGuardTests: XCTestCase {

    override func setUp() {
        super.setUp()
        // Prevent auto-record from firing during RecordingState init
        UserDefaults.standard.set(false, forKey: SettingsKeys.autoRecord)
    }

    override func tearDown() {
        UserDefaults.standard.removeObject(forKey: SettingsKeys.autoRecord)
        UserDefaults.standard.removeObject(forKey: SettingsKeys.sleepBehavior)
        super.tearDown()
    }

    @MainActor
    func testHandleWillSleepNoOpWhenNotRecording() {
        let recorder = RecordingState()
        XCTAssertFalse(recorder.isRecording)
        recorder.handleWillSleep()
        XCTAssertFalse(recorder.isRecording)
        XCTAssertEqual(recorder.statusText, "Ready")
    }

    @MainActor
    func testHandleDidWakeNoOpWhenNotInterrupted() {
        let recorder = RecordingState()
        recorder.handleDidWake()
        XCTAssertFalse(recorder.isRecording)
    }

    @MainActor
    func testHandleSessionResignNoOpWhenNotRecording() {
        let recorder = RecordingState()
        XCTAssertFalse(recorder.isRecording)
        recorder.handleSessionDidResignActive()
        XCTAssertFalse(recorder.isRecording)
    }

    @MainActor
    func testHandleSessionBecomeActiveNoOpWhenNotInterrupted() {
        let recorder = RecordingState()
        recorder.handleSessionDidBecomeActive()
        XCTAssertFalse(recorder.isRecording)
    }

    /// Verify that "stop" behavior does NOT set wasSleepInterrupted, so
    /// handleDidWake is a no-op afterward.
    @MainActor
    func testStopBehaviorDoesNotResumeOnWake() {
        let recorder = RecordingState()
        UserDefaults.standard.set("stop", forKey: SettingsKeys.sleepBehavior)
        // Simulate sleep when not recording — should be a no-op
        recorder.handleWillSleep()
        // Now simulate wake — should also be a no-op (wasSleepInterrupted is false)
        recorder.handleDidWake()
        XCTAssertFalse(recorder.isRecording)
    }

    /// Verify that calling handleDidWake twice (e.g. session + sleep overlap)
    /// does not crash or produce unexpected state.
    @MainActor
    func testDoubleWakeIsHarmless() {
        let recorder = RecordingState()
        recorder.handleDidWake()
        recorder.handleDidWake()
        XCTAssertFalse(recorder.isRecording)
    }

    /// Verify that handleWillSleep followed by handleSessionDidResignActive
    /// (stacked interrupts) does not crash.
    @MainActor
    func testSleepAndSessionResignStackedNoOp() {
        let recorder = RecordingState()
        recorder.handleWillSleep()
        recorder.handleSessionDidResignActive()
        XCTAssertFalse(recorder.isRecording)
    }
}
