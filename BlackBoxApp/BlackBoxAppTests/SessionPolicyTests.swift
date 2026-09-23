import XCTest

@testable import BlackBox_Audio_Recorder

nonisolated final class SessionPolicyTests: StandardDefaultsTestCase {
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

    // MARK: - restartProceeds

    /// A restart whose stop failed with the engine still recording must not
    /// swap folders or start again: that released the live folder's scope
    /// and left the UI idle over a running engine.
    func testRestartStopsWhenTheOldSessionIsStillRecording() {
        XCTAssertFalse(SessionPolicy.restartProceeds(after: .stillRecording))
    }

    /// Once the engine has stopped, even with a finalize error, the restart
    /// goes ahead.
    func testRestartProceedsOnceTheEngineStopped() {
        XCTAssertTrue(SessionPolicy.restartProceeds(after: .stopped))
        XCTAssertTrue(SessionPolicy.restartProceeds(after: .stoppedWithError))
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

    // MARK: - startAndWait and the launch restore

    /// A start right at launch waits for the bookmark-restore Task (folder
    /// scope and crash recovery) before it goes near the engine: starting
    /// first would record into the default folder, and recovery would then
    /// finalize the live session's .recording.wav.
    @MainActor
    func testStartWaitsForTheLaunchRestore() async {
        let recorder = RecordingState()
        let (restoreDone, finishRestore) = AsyncStream<Void>.makeStream()
        recorder.bookmarkRestoreTask = Task {
            for await _ in restoreDone {
                // Nothing is yielded; the loop ends when the test finishes the stream.
            }
        }

        let start = Task { await recorder.startAndWait() }
        while !recorder.isStartingRecording {
            await Task.yield()
        }
        for _ in 0..<50 {
            await Task.yield()
        }
        XCTAssertTrue(recorder.isStartingRecording, "the start must still be waiting on the restore")
        XCTAssertFalse(recorder.isRecording)

        // Cancel so the resumed start stops before the permission check and
        // the engine, which a unit test must not reach.
        start.cancel()
        finishRestore.finish()
        let started = await start.value

        XCTAssertFalse(started)
        XCTAssertFalse(recorder.isStartingRecording)
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

    /// The warning lands in the menu banner and a notification, and the
    /// call returns: it runs with the engine already recording, where a
    /// modal alert (the old path when the app was active) would block the
    /// main actor until the user clicked OK.
    @MainActor
    func testPreflightWarningSetsTheBannerWithoutBlocking() {
        let recorder = RecordingState()
        recorder.sampleRate = 48_000
        recorder.configSnapshot = RecordingState.RecordingConfigSnapshot(
            continuousMode: true,
            recordingCadence: 7200,
            channelCount: 8,
            bitDepth: 24,
            outputMode: "single",
            deviceName: nil
        )

        recorder.evaluatePreflightFileSizeWarning(isRestart: false)

        XCTAssertNotNil(recorder.preflightSizeWarning)
        XCTAssertEqual(recorder.lastPreflightEstimate, largeEstimate())
    }

    /// The order startRecordingInternal uses after a sample-rate-change
    /// restart: adopt the rate the new stream reports, then evaluate. The
    /// estimate must be the new rate's, so the changed projection is
    /// announced; with the stale 48 kHz it matched the old one and stayed
    /// silent.
    @MainActor
    func testRestartAtANewSampleRateEstimatesWithTheNewRate() {
        let recorder = RecordingState()
        recorder.sampleRate = 48_000
        recorder.configSnapshot = RecordingState.RecordingConfigSnapshot(
            continuousMode: true,
            recordingCadence: 7200,
            channelCount: 8,
            bitDepth: 24,
            outputMode: "single",
            deviceName: nil
        )
        recorder.evaluatePreflightFileSizeWarning(isRestart: false)

        recorder.adoptReportedSampleRate(96_000)
        recorder.evaluatePreflightFileSizeWarning(isRestart: true)

        XCTAssertEqual(recorder.sampleRate, 96_000)
        XCTAssertEqual(recorder.lastPreflightEstimate, largeEstimate(sampleRate: 96_000))
    }

    /// 0 means the engine has no stream yet; the last known rate stays.
    @MainActor
    func testAnUnreportedSampleRateKeepsTheLastOne() {
        let recorder = RecordingState()
        recorder.sampleRate = 44_100
        recorder.adoptReportedSampleRate(0)
        XCTAssertEqual(recorder.sampleRate, 44_100)
    }

    func testNoWarningNeverAnnounces() {
        XCTAssertFalse(SessionPolicy.shouldAnnouncePreflightWarning(current: nil, previous: nil, isRestart: false))
        XCTAssertFalse(
            SessionPolicy.shouldAnnouncePreflightWarning(current: nil, previous: largeEstimate(), isRestart: true)
        )
    }
}
