/// Pure decision logic for sleep/wake handling, extracted from
/// `RecordingState` for testability (the live `@MainActor` methods
/// are awkward to unit-test). All entry points are pure functions over
/// the relevant inputs; no I/O, no side effects.
///
/// Callers (`handleWillSleep` / `handleSessionDidResignActive`) must
/// check the returned action before mutating state. The mapping is:
/// - `.ignore` → no recording running; do nothing.
/// - `.pauseForResume` → stop the current recording AND mark
///   `wasSleepInterrupted = true` so the next wake / session-active
///   restarts it.
/// - `.stop` → stop the current recording without marking for resume.
///
/// `nonisolated`: pure functions with no shared state, so the unit tests can
/// call them without hopping to the main actor.
nonisolated enum SleepWakePolicy {
    /// The action a sleep / session-resign event should trigger.
    enum SleepAction: Equatable {
        /// Stop now and mark the session as interrupted so it can
        /// resume on wake / session-active.
        case pauseForResume
        /// Stop now; do not auto-resume on wake.
        case stop
        /// Do nothing — there's no active recording to interrupt.
        case ignore
    }

    /// Decision for `NSWorkspace.willSleepNotification`.
    /// - `behavior` is the user's "When Mac sleeps" preference
    ///   (`"resume"` or `"stop"`; anything else is treated as `"stop"`).
    static func sleepAction(isRecording: Bool, behavior: String) -> SleepAction {
        guard isRecording else { return .ignore }
        return behavior == "resume" ? .pauseForResume : .stop
    }

    /// Decision for `NSWorkspace.didWakeNotification`. Resume only if
    /// the prior `willSleep` set `wasSleepInterrupted = true`.
    static func shouldResumeOnWake(wasInterrupted: Bool) -> Bool {
        wasInterrupted
    }

    /// Whether to add `.idleSystemSleepDisabled` to the
    /// `ProcessInfo.beginActivity` options. App Nap is always
    /// prevented while recording; idle-sleep prevention is opt-in.
    static func shouldPreventSleep(settingEnabled: Bool) -> Bool {
        settingEnabled
    }

    /// Decision for `NSWorkspace.sessionDidResignActiveNotification`
    /// (fast user switch / screen-saver activate). Always
    /// `.pauseForResume` when recording — session-resign is
    /// recoverable; session-become-active triggers a restart.
    static func sessionResignAction(isRecording: Bool) -> SleepAction {
        guard isRecording else { return .ignore }
        return .pauseForResume
    }

    /// Why `RecordingState.stop(reason:)` was called. Determines whether
    /// a pending resume-on-wake survives the stop.
    enum StopReason {
        /// The user (menu, hotkey, Settings, quit) or the engine (disk
        /// full, persistent write failures) ended the recording for good.
        case user
        /// A sleep / session-resign handler is pausing the recording
        /// with the intent to resume on wake / session-active.
        case sleepInterruption
    }

    /// Whether a stop with the given reason cancels a pending
    /// resume-on-wake (clears `wasSleepInterrupted`).
    ///
    /// A user stop must cancel: without that, a manual stop inside the
    /// 1.5 s deferred-resume window would let the deferred `start()`
    /// resurrect a recording the user explicitly ended (DOLL-182). The
    /// sleep-interruption stop must NOT cancel — it is the very stop
    /// that just set the flag, and clearing it here made resume-on-wake
    /// dead code (DOLL-442).
    static func stopCancelsPendingResume(_ reason: StopReason) -> Bool {
        reason == .user
    }
}
