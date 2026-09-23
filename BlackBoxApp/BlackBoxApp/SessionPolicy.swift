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

    /// Whether `restartIfRecording` may go on after stopping the old
    /// session: only once the engine has stopped. A session that is still
    /// recording keeps its folder and its UI state, and is not restarted.
    static func restartProceeds(after outcome: StopOutcome) -> Bool {
        outcome != .stillRecording
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

    // MARK: - Pre-flight 4 GiB warning (DOLL-220)

    /// A WAV file's `data` chunk size is a `u32`, so one file tops out at
    /// 4 GiB - 1.
    static let wavMaxFileBytes = Int64(UInt32.max)

    /// The projection a pre-flight 4 GiB warning is built from. Two equal
    /// estimates produce the same warning text.
    struct PreflightSizeEstimate: Equatable {
        /// Projected bytes in the largest file one rotation writes.
        var bytesPerFile: Int64
        /// False when the device's rate is not known yet and 48 kHz was assumed.
        var sampleRateKnown: Bool
    }

    /// The projection for a session, or nil when no warning applies: no
    /// rotation (`rotationSeconds` is nil outside continuous mode, which has
    /// no interval to bound a file with), a degenerate config, or a file
    /// that stays under the cap.
    ///
    /// `sampleRate` is 0 until a device has been seen; 48 kHz is assumed
    /// then, which biases toward under-warning rather than spooking users
    /// about hi-res setups they don't have.
    static func preflightSizeEstimate(
        rotationSeconds: Int?,
        channelCount: Int,
        bitDepth: Int,
        splitOutput: Bool,
        sampleRate: Int
    ) -> PreflightSizeEstimate? {
        guard let recordingCadence = rotationSeconds, recordingCadence > 0, channelCount > 0 else { return nil }
        // Split mode writes one channel per file; single mode puts every
        // channel in one file. The cap is per file.
        let channelsPerFile = splitOutput ? 1 : channelCount
        let rate = sampleRate > 0 ? sampleRate : 48_000
        let bytesPerFile =
            Int64(channelsPerFile) * Int64(bitDepth / 8) * Int64(rate) * Int64(recordingCadence)
        guard bytesPerFile > wavMaxFileBytes else { return nil }
        return PreflightSizeEstimate(bytesPerFile: bytesPerFile, sampleRateKnown: sampleRate > 0)
    }

    /// Whether to announce a pre-flight warning (notification and log) for
    /// the session just starting. A fresh start always does. An internal
    /// restart of a live session (device change, a setting applied while
    /// recording, a sample-rate change) only does when the projection
    /// changed: the user was already told about the one it would repeat.
    static func shouldAnnouncePreflightWarning(
        current: PreflightSizeEstimate?,
        previous: PreflightSizeEstimate?,
        isRestart: Bool
    ) -> Bool {
        guard current != nil else { return false }
        return !isRestart || current != previous
    }
}
