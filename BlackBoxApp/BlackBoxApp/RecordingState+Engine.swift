import Foundation

/// The engine calls that can block for seconds: start probes and opens the
/// device, stop finalizes the files and joins the writer. `RecordingState`
/// runs them off the main actor (DOLL-659) so the menu stays responsive.
/// A value type of closures so tests can put slow or failing fakes in place
/// of the real engine.
nonisolated struct EngineCalls: Sendable {
    var start: @Sendable () -> BlackBoxError
    var stop: @Sendable () -> BlackBoxError

    init(start: @escaping @Sendable () -> BlackBoxError, stop: @escaping @Sendable () -> BlackBoxError) {
        self.start = start
        self.stop = stop
    }

    init(bridge: RustBridge) {
        self.init(start: { bridge.startRecording() }, stop: { bridge.stopRecording() })
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
    func runEngine(_ call: @escaping @Sendable () -> BlackBoxError) async -> BlackBoxError {
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
    func startEngine(generation: Int) async -> BlackBoxError? {
        let result = await runEngine(engineCalls.start)
        guard generation == sessionGeneration else {
            if result.isSuccess {
                _ = await runEngine(engineCalls.stop)
            }
            return nil
        }
        return result
    }

    /// Mark everything in flight as belonging to a session that has ended.
    /// Every stop calls this before it reaches the engine; a start that was
    /// already on its way then sees on return that its session ended under
    /// it (see `startEngine`).
    func endSessionGeneration() -> Int {
        sessionGeneration &+= 1
        return sessionGeneration
    }

    /// Stop the session and wait, up to `timeout`, for the engine to finish
    /// writing its files. Used by quit, which replies to AppKit once this
    /// returns. A start still in flight is stopped as soon as it returns.
    func stopBeforeQuit(timeout: Duration) async {
        await withTaskGroup(of: Void.self) { group in
            group.addTask { await self.stopAndDrainEngine() }
            group.addTask { try? await Task.sleep(for: timeout) }
            await group.next()
            group.cancelAll()
        }
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
