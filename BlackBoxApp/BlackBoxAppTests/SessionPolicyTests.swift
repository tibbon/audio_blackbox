import XCTest

@testable import BlackBox_Audio_Recorder

nonisolated final class SessionPolicyTests: XCTestCase {
    // MARK: - stopOutcome

    func testSuccessfulStopIsStopped() {
        XCTAssertEqual(SessionPolicy.stopOutcome(stopSucceeded: true, engineStillRecording: false), .stopped)
    }

    /// The FFI takes the recorder before finalizing, so a failed stop usually
    /// leaves the engine stopped: the UI must leave "Recording" and show the
    /// error, not stay stuck in a session nothing is polling.
    func testFailedStopWithEngineStoppedEndsTheSession() {
        XCTAssertEqual(
            SessionPolicy.stopOutcome(stopSucceeded: false, engineStillRecording: false),
            .stoppedWithError
        )
    }

    func testFailedStopWithEngineRunningKeepsTheSession() {
        XCTAssertEqual(
            SessionPolicy.stopOutcome(stopSucceeded: false, engineStillRecording: true),
            .stillRecording
        )
    }

    // MARK: - shouldStartMonitoring

    func testMonitoringStartsForAnOpenIdleMeter() {
        XCTAssertTrue(
            SessionPolicy.shouldStartMonitoring(
                meterWindowOpen: true,
                isRecording: false,
                isStartingRecording: false,
                isMonitoring: false
            )
        )
    }

    /// The meter window closed while the permission prompt was up: starting
    /// now would leave a stream running that nothing will stop.
    func testMonitoringDoesNotStartAfterTheMeterClosed() {
        XCTAssertFalse(
            SessionPolicy.shouldStartMonitoring(
                meterWindowOpen: false,
                isRecording: false,
                isStartingRecording: false,
                isMonitoring: false
            )
        )
    }

    // MARK: - applySessionSetting

    /// Idle: a session-start setting is applied at once, with no prompt.
    /// (While recording it asks to restart first, which needs a modal.)
    @MainActor
    func testSessionSettingAppliesImmediatelyWhenIdle() {
        let recorder = RecordingState()
        var applied = 0
        let result = applySessionSetting(recorder: recorder, reason: "test") { applied += 1 }
        XCTAssertTrue(result)
        XCTAssertEqual(applied, 1)
    }

    func testMonitoringNeverTakesTheStreamFromARecording() {
        for (recording, starting) in [(true, false), (false, true)] {
            XCTAssertFalse(
                SessionPolicy.shouldStartMonitoring(
                    meterWindowOpen: true,
                    isRecording: recording,
                    isStartingRecording: starting,
                    isMonitoring: false
                )
            )
        }
    }

    // MARK: - Pre-flight 4 GiB warning

    /// 8 channels, 24-bit, 48 kHz, one file, 2 h rotation: ~8.3 GB per file.
    private func largeEstimate(sampleRate: Int = 48_000) -> SessionPolicy.PreflightSizeEstimate? {
        SessionPolicy.preflightSizeEstimate(
            rotationSeconds: 7200,
            channelCount: 8,
            bitDepth: 24,
            splitOutput: false,
            sampleRate: sampleRate
        )
    }

    func testEstimateOverTheCapWarns() {
        let estimate = largeEstimate()
        XCTAssertEqual(estimate?.bytesPerFile, 8 * 3 * 48_000 * 7200)
        XCTAssertEqual(estimate?.sampleRateKnown, true)
    }

    func testEstimateUnderTheCapOrOutsideContinuousModeIsNil() {
        // Split mode: one channel per file, ~1 GB.
        XCTAssertNil(
            SessionPolicy.preflightSizeEstimate(
                rotationSeconds: 7200,
                channelCount: 8,
                bitDepth: 24,
                splitOutput: true,
                sampleRate: 48_000
            )
        )
        // Not continuous mode: no rotation to bound the file.
        XCTAssertNil(
            SessionPolicy.preflightSizeEstimate(
                rotationSeconds: nil,
                channelCount: 8,
                bitDepth: 24,
                splitOutput: false,
                sampleRate: 48_000
            )
        )
    }

    func testUnknownSampleRateAssumes48kHz() {
        let estimate = largeEstimate(sampleRate: 0)
        XCTAssertEqual(estimate?.bytesPerFile, largeEstimate()?.bytesPerFile)
        XCTAssertEqual(estimate?.sampleRateKnown, false)
    }

    func testFreshStartAlwaysAnnounces() {
        let estimate = largeEstimate()
        XCTAssertTrue(
            SessionPolicy.shouldAnnouncePreflightWarning(current: estimate, previous: estimate, isRestart: false)
        )
        XCTAssertTrue(SessionPolicy.shouldAnnouncePreflightWarning(current: estimate, previous: nil, isRestart: false))
    }

    /// A device change or config restart with the same projection must not
    /// repeat the notification.
    func testRestartWithUnchangedInputsDoesNotAnnounce() {
        let estimate = largeEstimate()
        XCTAssertFalse(
            SessionPolicy.shouldAnnouncePreflightWarning(current: estimate, previous: estimate, isRestart: true)
        )
    }

    func testRestartWithChangedInputsAnnounces() {
        XCTAssertTrue(
            SessionPolicy.shouldAnnouncePreflightWarning(
                current: largeEstimate(sampleRate: 96_000),
                previous: largeEstimate(),
                isRestart: true
            )
        )
        XCTAssertTrue(
            SessionPolicy.shouldAnnouncePreflightWarning(current: largeEstimate(), previous: nil, isRestart: true)
        )
    }

    func testNoWarningNeverAnnounces() {
        XCTAssertFalse(SessionPolicy.shouldAnnouncePreflightWarning(current: nil, previous: nil, isRestart: false))
        XCTAssertFalse(
            SessionPolicy.shouldAnnouncePreflightWarning(current: nil, previous: largeEstimate(), isRestart: true)
        )
    }
}
