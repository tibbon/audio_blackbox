import BlackBoxFFI
import XCTest

@testable import BlackBox_Audio_Recorder

nonisolated final class EngineStatusPolicyTests: XCTestCase {
    /// A healthy, recording engine.
    private func recording() -> StatusFlags {
        var status = StatusFlags()
        status.is_recording = true
        status.sample_rate = 48_000
        return status
    }

    // MARK: - action: priority order

    func testHealthyEngineKeepsRecording() {
        XCTAssertEqual(EngineStatusPolicy.action(isRecording: true, status: recording()), .keepRecording)
    }

    /// An engine that stopped on its own wins over every flag it left set.
    func testUnexpectedStopWinsOverEveryFlag() {
        var status = recording()
        status.is_recording = false
        status.sample_rate_changed = true
        status.stream_error = true
        status.write_failed = true
        status.disk_space_low = true
        status.write_errors = 100_000
        XCTAssertEqual(EngineStatusPolicy.action(isRecording: true, status: status), .handleUnexpectedStop)
    }

    /// Idle UI and idle engine is not an "unexpected stop".
    func testIdleEngineWhileIdleIsNotAnUnexpectedStop() {
        XCTAssertEqual(EngineStatusPolicy.action(isRecording: false, status: StatusFlags()), .keepRecording)
    }

    func testSampleRateChangeWinsOverStreamError() {
        var status = recording()
        status.sample_rate_changed = true
        status.stream_error = true
        status.write_failed = true
        XCTAssertEqual(EngineStatusPolicy.action(isRecording: true, status: status), .restartForSampleRateChange)
    }

    func testStreamErrorWinsOverWriteFailure() {
        var status = recording()
        status.stream_error = true
        status.write_failed = true
        status.disk_space_low = true
        XCTAssertEqual(EngineStatusPolicy.action(isRecording: true, status: status), .recoverFromStreamError)
    }

    /// DOLL-437: a write failure is reported as such, not as low disk.
    func testWriteFailureWinsOverLowDiskSpace() {
        var status = recording()
        status.write_failed = true
        status.disk_space_low = true
        status.write_errors = 100_000
        XCTAssertEqual(EngineStatusPolicy.action(isRecording: true, status: status), .stopForWriteFailure)
    }

    func testLowDiskSpaceWinsOverExcessiveDrops() {
        var status = recording()
        status.disk_space_low = true
        status.write_errors = 100_000
        XCTAssertEqual(EngineStatusPolicy.action(isRecording: true, status: status), .stopForLowDiskSpace)
    }

    func testExcessiveDropsStopOnlyAboveTheLimit() {
        var status = recording()
        status.write_errors = UInt64(EngineStatusPolicy.excessiveWriteErrors)
        XCTAssertEqual(EngineStatusPolicy.action(isRecording: true, status: status), .keepRecording)
        status.write_errors += 1
        XCTAssertEqual(EngineStatusPolicy.action(isRecording: true, status: status), .stopForExcessiveWriteErrors)
    }

    // MARK: - streamRestart (DOLL-351 flapping cap)

    func testFirstRestartIsAllowed() {
        let restart = EngineStatusPolicy.streamRestart(previousCount: 0, lastRestart: nil, now: Date())
        XCTAssertEqual(restart, .init(count: 1, isAllowed: true))
    }

    /// A device that faults every second is restarted three times, then the
    /// fourth restart inside the window is refused.
    func testRapidRestartsAreCappedAtThree() {
        let start = Date(timeIntervalSinceReferenceDate: 0)
        var count = 0
        var last: Date?
        var allowed: [Bool] = []
        for second in 0..<4 {
            let now = start.addingTimeInterval(TimeInterval(second))
            let restart = EngineStatusPolicy.streamRestart(previousCount: count, lastRestart: last, now: now)
            count = restart.count
            last = now
            allowed.append(restart.isAllowed)
        }
        XCTAssertEqual(allowed, [true, true, true, false])
    }

    /// A restart after a stable run (longer than the window) starts a new burst.
    func testRestartAfterAStableRunResetsTheCount() {
        let last = Date(timeIntervalSinceReferenceDate: 0)
        let now = last.addingTimeInterval(EngineStatusPolicy.streamRestartWindow)
        let restart = EngineStatusPolicy.streamRestart(previousCount: 3, lastRestart: last, now: now)
        XCTAssertEqual(restart, .init(count: 1, isAllowed: true))
    }
}
