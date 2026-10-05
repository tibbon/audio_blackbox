import Dispatch
import Synchronization
import XCTest

@testable import BlackBox_Audio_Recorder

/// An engine stand-in that records every call in order and can hold the
/// first start or the first stop until the test releases it; later calls
/// pass straight through, so a synchronous stop on the main thread never
/// waits on a gate. The calls run off the main actor, so a test that awaits
/// while one is held also shows the main actor stays free: if the call ran
/// on the main thread, the test could never release it. Unlike the real
/// engine it has no lock, so a call can run while a held one is still in it.
nonisolated final class FakeEngine: Sendable {
    private let log = Mutex<[String]>([])
    private let startGate: DispatchSemaphore?
    private let stopGate: DispatchSemaphore?
    private let gatesUsed = Mutex<Set<String>>([])

    init(holdStart: Bool = false, holdStop: Bool = false) {
        startGate = holdStart ? DispatchSemaphore(value: 0) : nil
        stopGate = holdStop ? DispatchSemaphore(value: 0) : nil
    }

    var calls: [String] { log.withLock { $0 } }

    var engineCalls: EngineCalls {
        EngineCalls(
            start: { [self] in call("start", gate: startGate) },
            stop: { [self] in call("stop", gate: stopGate) }
        )
    }

    private func call(_ name: String, gate: DispatchSemaphore?) -> EngineOutcome {
        log.withLock { $0.append(name) }
        if gatesUsed.withLock({ $0.insert(name).inserted }) {
            gate?.wait()
        }
        return .ok
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

    /// A synchronous stop (sleep, power off) while a restart's start is in
    /// the engine: the session ends at once, and the start returns into an
    /// ended session, so it backs out and stops the engine again. (The fake
    /// has no lock, so the stop runs while the start is held; the real
    /// engine would make the stop wait for the start. Either way the start
    /// returns after the session ended.)
    @MainActor
    func testSynchronousStopDuringAStartBacksTheStartOut() async throws {
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

    /// A start still queued (behind a running stop) when a synchronous stop
    /// bypasses the queue never reaches the engine: it would otherwise open
    /// the device as the Mac goes to sleep.
    @MainActor
    func testQueuedStartSkipsTheEngineAfterASynchronousStop() async throws {
        let recorder = RecordingState()
        let engine = FakeEngine(holdStop: true)
        recorder.engineCalls = engine.engineCalls
        recorder.isRecording = true

        recorder.stop()
        try await waitUntil("the stop reaches the engine") { engine.calls == ["stop"] }
        let generation = recorder.sessionGeneration
        let start = Task { await recorder.startEngine(generation: generation) }
        await Task.yield()

        recorder.stopSynchronously(reason: .sleepInterruption)
        engine.releaseStop()
        let outcome = await start.value

        XCTAssertNil(outcome, "the queued start belongs to an ended session")
        XCTAssertFalse(engine.calls.contains("start"), "the start must not reach the engine")
    }

    /// Sleep while a user stop is finalizing must not mark the session for
    /// resume: wake would otherwise bring back a recording the user stopped.
    @MainActor
    func testSleepDuringAStopDoesNotResumeOnWake() async throws {
        UserDefaults.standard.set("resume", forKey: SettingsKeys.sleepBehavior)
        let recorder = RecordingState()
        let engine = FakeEngine(holdStop: true)
        recorder.engineCalls = engine.engineCalls
        recorder.isRecording = true

        recorder.stop()
        try await waitUntil("the stop reaches the engine") { engine.calls == ["stop"] }
        recorder.handleWillSleep()

        XCTAssertFalse(recorder.wasSleepInterrupted, "a stopped session is not resumed")
        XCTAssertFalse(recorder.isRecording, "sleep still finalizes before it returns")
        engine.releaseStop()
        try await waitUntil("the stop finishes") { recorder.sessionPhase == .idle }
        recorder.handleDidWake()
        XCTAssertNil(recorder.pendingResumeTask)
    }

    /// A setting changed while a restart is starting is not dropped: the
    /// second restart waits for the first, then restarts again so the
    /// engine picks up the change.
    @MainActor
    func testRestartRequestedDuringARestartRunsAfterIt() async throws {
        let recorder = RecordingState()
        let engine = FakeEngine(holdStart: true)
        recorder.engineCalls = engine.engineCalls
        recorder.isRecording = true

        let first = Task { await recorder.restartIfRecording(reason: "first") }
        try await waitUntil("the first restart's start reaches the engine") { engine.calls == ["stop", "start"] }
        let second = Task { await recorder.restartIfRecording(reason: "second") }
        engine.releaseStart()

        let firstRestarted = await first.value
        let secondRestarted = await second.value
        XCTAssertTrue(firstRestarted)
        XCTAssertTrue(secondRestarted, "the second restart must run, not be dropped")
        XCTAssertEqual(engine.calls, ["stop", "start", "stop", "start"])
        recorder.stopSynchronously()
    }

    /// Quit waits for the stop only up to its timeout: a finalize that hangs
    /// must not keep the app from quitting.
    @MainActor
    func testQuitStopIsBoundedByItsTimeout() async throws {
        let recorder = RecordingState()
        let engine = FakeEngine(holdStop: true)
        recorder.engineCalls = engine.engineCalls
        recorder.isRecording = true

        let began = ContinuousClock.now
        await recorder.stopBeforeQuit(timeout: .milliseconds(100))

        XCTAssertLessThan(ContinuousClock.now - began, .seconds(3), "quit must not wait for the held stop")
        XCTAssertEqual(recorder.sessionPhase, .stopping)
        engine.releaseStop()
        try await waitUntil("the stop finishes") { recorder.sessionPhase == .idle }
    }
}
