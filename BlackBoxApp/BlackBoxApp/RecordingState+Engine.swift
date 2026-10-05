import Foundation
import Synchronization

/// What an engine call returned. `detail` is the engine's last error,
/// read on the same thread right after a failed call: read later on the
/// main actor, a config change in between (`blackbox_set_config_json`
/// clears it) could replace it with nothing.
nonisolated struct EngineOutcome: Sendable, Equatable {
    let code: BlackBoxError
    var detail: String?

    var isSuccess: Bool { code.isSuccess }

    static let ok = Self(code: .ok)
}

/// The engine calls that can block for seconds: start probes and opens the
/// device, stop finalizes the files and joins the writer. `RecordingState`
/// runs them off the main actor (DOLL-659) so the menu stays responsive.
/// A value type of closures so tests can put slow or failing fakes in place
/// of the real engine.
nonisolated struct EngineCalls: Sendable {
    var start: @Sendable () -> EngineOutcome
    var stop: @Sendable () -> EngineOutcome

    init(start: @escaping @Sendable () -> EngineOutcome, stop: @escaping @Sendable () -> EngineOutcome) {
        self.start = start
        self.stop = stop
    }

    init(bridge: RustBridge) {
        self.init(
            start: { Self.outcome(bridge.startRecording(), bridge) },
            stop: { Self.outcome(bridge.stopRecording(), bridge) }
        )
    }

    private static func outcome(_ code: BlackBoxError, _ bridge: RustBridge) -> EngineOutcome {
        EngineOutcome(code: code, detail: code.isSuccess ? nil : bridge.lastError)
    }
}

/// The session generation (see `RecordingState.endSessionGeneration`),
/// readable off the main actor so a queued start can check it right
/// before it reaches the engine.
nonisolated final class SessionEpoch: Sendable {
    private let value = Mutex(0)

    var current: Int { value.withLock { $0 } }

    func advance() -> Int {
        value.withLock { value in
            value &+= 1
            return value
        }
    }
}

/// Where the recording session is, for the menu (DOLL-659). Starting and
/// stopping each cover the engine call running off the main actor.
enum SessionPhase: Equatable {
    case idle
    case starting
    case recording
    case stopping
}

extension RecordingState {
    /// The phase the menu shows. A restart of a live session counts as
    /// starting from its first step, so Stop pressed during it ends the
    /// session instead of being ignored.
    var sessionPhase: SessionPhase {
        if isStoppingRecording { return .stopping }
        if isStartingRecording { return .starting }
        return isRecording ? .recording : .idle
    }

    /// `statusText`, or what is in flight while the engine starts or stops.
    var displayedStatusText: String {
        switch sessionPhase {
        case .starting: String(localized: "Starting\u{2026}")
        case .stopping: String(localized: "Stopping\u{2026}")
        case .idle, .recording: statusText
        }
    }

    /// The menu's Start/Stop button. Stop is offered while a start is still
    /// in flight; it ends the session as soon as the engine has started.
    var primaryActionTitle: String {
        switch sessionPhase {
        case .idle: String(localized: "Start Recording")
        case .starting, .recording: String(localized: "Stop Recording")
        case .stopping: String(localized: "Stopping\u{2026}")
        }
    }

    /// Run `call` off the main actor, after every engine call issued before
    /// it has returned, so starts and stops reach the engine in the order
    /// they were asked for. A chain on the main actor rather than an actor:
    /// calls from separate Tasks are not guaranteed to reach an actor in
    /// the order they were made.
    func runEngine(_ call: @escaping @Sendable () -> EngineOutcome) async -> EngineOutcome {
        let previous = engineTail
        let task = Task {
            _ = await previous?.value
            return await Task { @concurrent in call() }.value
        }
        engineTail = task
        return await task.value
    }

    /// Start the engine for the session `generation` belongs to. `nil` when
    /// a stop ended that session while the engine was starting: the stop
    /// did the teardown, and if it reached the engine before this start
    /// did, the engine is now recording for nobody, so it is stopped again.
    ///
    /// The generation is checked once more right before the engine call:
    /// a synchronous stop (sleep) bypasses the queue, and a start queued
    /// behind it would otherwise open the device as the Mac goes to sleep,
    /// to be stopped only after wake.
    func startEngine(generation: Int) async -> EngineOutcome? {
        let start = engineCalls.start
        let epoch = sessionEpoch
        let result = await runEngine {
            epoch.current == generation ? start() : .ok
        }
        guard generation == sessionGeneration else {
            if result.isSuccess {
                _ = await runEngine(engineCalls.stop)
            }
            return nil
        }
        return result
    }

    /// Bumped by every stop (`endSessionGeneration()`). A start compares it
    /// before and after its engine call to tell whether the session it was
    /// starting was stopped meanwhile.
    var sessionGeneration: Int { sessionEpoch.current }

    /// Mark everything in flight as belonging to a session that has ended.
    /// Every stop calls this before it reaches the engine; a start that was
    /// already on its way then sees on return that its session ended under
    /// it (see `startEngine`).
    func endSessionGeneration() -> Int {
        sessionEpoch.advance()
    }

    /// Wait until no start, stop or restart is in flight. Polled, because a
    /// start can sit in the microphone permission dialog with no engine
    /// call to wait on.
    func waitForSessionToSettle() async {
        while sessionPhase == .starting || sessionPhase == .stopping {
            try? await Task.sleep(for: .milliseconds(100))
        }
    }

    /// Stop the session and wait, up to `timeout`, for the engine to finish
    /// writing its files. Used by quit, which replies to AppKit once this
    /// returns. A start still in flight is stopped as soon as it returns.
    /// Not a task group: a group waits for all its children, and the stop
    /// cannot be cancelled, so a hung finalize would hang quit with it.
    func stopBeforeQuit(timeout: Duration) async {
        let finished = OneShot()
        let stop = Task {
            await stopAndDrainEngine()
            finished.fire()
        }
        let timer = Task {
            try? await Task.sleep(for: timeout)
            finished.fire()
        }
        await finished.wait()
        timer.cancel()
        _ = stop  // keeps running after a timeout; quit no longer waits for it
    }

    private func stopAndDrainEngine() async {
        if !isStoppingRecording {
            await stopAndWait()
        }
        // A stop that was already running, or a late start's own stop, is
        // queued behind the call above.
        _ = await engineTail?.value
    }
}

/// Resumes one waiter the first time `fire()` is called; later calls do
/// nothing.
private final class OneShot {
    private var fired = false
    private var waiter: CheckedContinuation<Void, Never>?

    func fire() {
        guard !fired else { return }
        fired = true
        waiter?.resume()
        waiter = nil
    }

    func wait() async {
        guard !fired else { return }
        await withCheckedContinuation { waiter = $0 }
    }
}
