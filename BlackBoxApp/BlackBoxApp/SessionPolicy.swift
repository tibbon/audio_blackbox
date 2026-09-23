/// Pure decisions for the recording-session lifecycle, extracted from
/// `RecordingState` so they can be unit-tested without a live engine (the
/// same approach as `SleepWakePolicy`).
///
/// `nonisolated`: pure functions with no shared state, so the unit tests can
/// call them without hopping to the main actor.
nonisolated enum SessionPolicy {
    /// What `RecordingState.stop()` found once `blackbox_stop_recording`
    /// returned.
    enum StopOutcome: Equatable {
        /// The engine stopped and finalized cleanly.
        case stopped
        /// The engine stopped, but the call reported an error (typically the
        /// final flush or header rewrite failed). The session is over; the
        /// error still needs showing.
        case stoppedWithError
        /// The call failed and the engine still reports a live recording.
        case stillRecording
    }

    /// Classify a stop attempt. `engineStillRecording` is the engine's own
    /// answer after the call; only a failed call that leaves it `true` means
    /// the session is still live.
    static func stopOutcome(stopSucceeded: Bool, engineStillRecording: Bool) -> StopOutcome {
        if stopSucceeded { return .stopped }
        return engineStillRecording ? .stillRecording : .stoppedWithError
    }

    /// Whether the level meter's monitoring stream should start, checked
    /// after the microphone-permission await, which can suspend for as long
    /// as the permission dialog is up. Monitoring exists only for an open
    /// meter window (it is tied to window-open, not occlusion, DOLL-348), and
    /// never takes the stream from a live or starting recording (DOLL-459).
    static func shouldStartMonitoring(
        meterWindowOpen: Bool,
        isRecording: Bool,
        isStartingRecording: Bool,
        isMonitoring: Bool
    ) -> Bool {
        meterWindowOpen && !isRecording && !isStartingRecording && !isMonitoring
    }
}
