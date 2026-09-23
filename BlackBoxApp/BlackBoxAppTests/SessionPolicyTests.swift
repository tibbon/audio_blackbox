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
}
