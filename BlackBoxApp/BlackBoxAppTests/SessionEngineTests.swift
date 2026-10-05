import Dispatch
import Synchronization
import XCTest

@testable import BlackBox_Audio_Recorder

/// An engine stand-in that records every call in order and can hold a start
/// or a stop until the test releases it. The calls run off the main actor,
/// so a test that awaits while one is held also shows the main actor stays
/// free: if the call ran on the main thread, the test could never release it.
nonisolated final class FakeEngine: Sendable {
    private let log = Mutex<[String]>([])
    private let startGate: DispatchSemaphore?
    private let stopGate: DispatchSemaphore?

    init(holdStart: Bool = false, holdStop: Bool = false) {
        startGate = holdStart ? DispatchSemaphore(value: 0) : nil
        stopGate = holdStop ? DispatchSemaphore(value: 0) : nil
    }

    var calls: [String] { log.withLock { $0 } }

    var engineCalls: EngineCalls {
        EngineCalls(
            start: { [self] in
                log.withLock { $0.append("start") }
                startGate?.wait()
                return .ok
            },
            stop: { [self] in
                log.withLock { $0.append("stop") }
                stopGate?.wait()
                return .ok
            }
        )
    }

    func releaseStart() { startGate?.signal() }
    func releaseStop() { stopGate?.signal() }
}

/// DOLL-659: start and stop run off the main actor, in the order they were
/// asked for, and a stop issued while a start is in flight ends the session.
nonisolated final class SessionEngineTests: StandardDefaultsTestCase {
    override func setUp() {
        super.setUp()
        UserDefaults.standard.set(false, forKey: SettingsKeys.autoRecord)
    }

    /// Poll until `condition` holds; the engine calls finish on other threads.
    @MainActor
    private func waitUntil(
        _ what: String,
        file: StaticString = #filePath,
        line: UInt = #line,
        _ condition: () -> Bool
    ) async throws {
        let deadline = ContinuousClock.now + .seconds(5)
        while !condition() {
            guard ContinuousClock.now < deadline else {
                XCTFail("timed out waiting until \(what)", file: file, line: line)
                return
            }
            try await Task.sleep(for: .milliseconds(5))
        }
    }

    @MainActor
    func testStopReturnsBeforeTheEngineHasFinalized() async throws {
        let recorder = RecordingState()
        let engine = FakeEngine(holdStop: true)
        recorder.engineCalls = engine.engineCalls
        recorder.isRecording = true

        recorder.stop()
        try await waitUntil("the stop reaches the engine") { engine.calls == ["stop"] }

        XCTAssertEqual(recorder.sessionPhase, .stopping)
        XCTAssertEqual(recorder.displayedStatusText, "Stopping\u{2026}")
        XCTAssertTrue(recorder.isRecording, "the session lasts until the engine has finalized")
        recorder.toggle()  // ignored while stopping

        engine.releaseStop()
        try await waitUntil("the stop finishes") { recorder.sessionPhase == .idle }
        XCTAssertFalse(recorder.isRecording)
        XCTAssertEqual(recorder.statusText, "Ready")
        XCTAssertEqual(engine.calls, ["stop"], "a toggle while stopping must not issue another call")
    }

    @MainActor
    func testRestartRunsStopThenStartAndReportsTheNewSession() async {
        let recorder = RecordingState()
        let engine = FakeEngine()
        recorder.engineCalls = engine.engineCalls
        recorder.isRecording = true

        let restarted = await recorder.restartIfRecording(reason: "test")

        XCTAssertTrue(restarted)
        XCTAssertTrue(recorder.isRecording)
        XCTAssertEqual(recorder.sessionPhase, .recording)
        XCTAssertEqual(engine.calls, ["stop", "start"])
        recorder.stopSynchronously()  // end the session's timer and sleep prevention
    }

    @MainActor
    func testRestartWithoutASessionDoesNothing() async {
        let recorder = RecordingState()
        let engine = FakeEngine()
        recorder.engineCalls = engine.engineCalls

        let restarted = await recorder.restartIfRecording(reason: "test")

        XCTAssertFalse(restarted)
        XCTAssertEqual(engine.calls, [])
    }

    /// Stop pressed while the restart's start is in the engine is queued
    /// behind it, and the session ends once the start returns. It used to
    /// reach toggle() as another start, which the in-flight check dropped.
    @MainActor
    func testStopDuringARestartEndsTheSession() async throws {
        let recorder = RecordingState()
        let engine = FakeEngine(holdStart: true)
        recorder.engineCalls = engine.engineCalls
        recorder.isRecording = true

        let restart = Task { await recorder.restartIfRecording(reason: "test") }
        try await waitUntil("the restart's start reaches the engine") { engine.calls == ["stop", "start"] }
        XCTAssertEqual(recorder.sessionPhase, .starting)
        XCTAssertEqual(recorder.displayedStatusText, "Starting\u{2026}")

        recorder.toggle()
        XCTAssertEqual(recorder.sessionPhase, .stopping)

        engine.releaseStart()
        let restarted = await restart.value
        try await waitUntil("the stop finishes") { recorder.sessionPhase == .idle }

        XCTAssertFalse(restarted, "a stopped restart must not report a new session")
        XCTAssertFalse(recorder.isRecording)
        XCTAssertEqual(Array(engine.calls.prefix(3)), ["stop", "start", "stop"], "the stop runs after the start")
        XCTAssertEqual(engine.calls.last, "stop")
    }

    /// A synchronous stop (sleep, power off) while a start is in flight: the
    /// session ends at once, and the start, reaching the engine after the
    /// stop, stops the engine again when it returns.
    @MainActor
    func testSynchronousStopDuringAStartStopsTheLateStart() async throws {
        let recorder = RecordingState()
        let engine = FakeEngine(holdStart: true)
        recorder.engineCalls = engine.engineCalls
        recorder.isRecording = true

        let restart = Task { await recorder.restartIfRecording(reason: "test") }
        try await waitUntil("the restart's start reaches the engine") { engine.calls == ["stop", "start"] }

        recorder.stopSynchronously(reason: .sleepInterruption)
        XCTAssertFalse(recorder.isRecording, "a synchronous stop ends the session before it returns")
        XCTAssertEqual(engine.calls, ["stop", "start", "stop"])

        engine.releaseStart()
        let restarted = await restart.value
        try await waitUntil("the late start is stopped") { engine.calls.count == 4 }

        XCTAssertFalse(restarted)
        XCTAssertFalse(recorder.isRecording)
        XCTAssertEqual(engine.calls.last, "stop")
    }
}
