import BlackBoxFFI
import Foundation

/// Pure decisions behind the 1 Hz engine status poll
/// (`RecordingState.applyEngineStatus`), extracted so the priority order and
/// the DOLL-351 flapping cap can be unit-tested without a live engine (the
/// same approach as `SleepWakePolicy`).
///
/// `nonisolated`: pure functions with no shared state, so the unit tests can
/// call them without hopping to the main actor.
nonisolated enum EngineStatusPolicy {
    /// Cumulative dropped samples at which a recording is stopped as
    /// unusable ("heavy load").
    static let excessiveWriteErrors = 48_000

    /// DOLL-351: stream-error restarts allowed back to back before giving up.
    static let maxConsecutiveStreamRestarts = 3

    /// DOLL-351: restarts closer together than this count toward the cap; a
    /// restart after a longer stable run starts the count again.
    static let streamRestartWindow: TimeInterval = 10

    /// What one status poll asks the app to do. At most one terminal action
    /// runs per tick.
    enum Action: Equatable {
        /// The engine stopped on its own (device disconnect, etc.).
        case handleUnexpectedStop
        /// The device changed sample rate; restart so the WAV header matches.
        case restartForSampleRateChange
        /// The audio stream failed; finalize and try the next device.
        case recoverFromStreamError
        /// Writes keep failing (disk full, folder unwritable) (DOLL-437).
        case stopForWriteFailure
        /// Free space fell below the configured minimum.
        case stopForLowDiskSpace
        /// Too many samples dropped for the recording to be usable.
        case stopForExcessiveWriteErrors
        /// Nothing terminal: keep recording and publish the counters.
        case keepRecording
    }

    /// The action for one poll, in priority order: an unexpected stop, then
    /// sample-rate change, stream error, write failure (checked before low
    /// disk so the cause is reported accurately, DOLL-437), low disk, and
    /// excessive drops.
    static func action(isRecording: Bool, status: StatusFlags) -> Action {
        if isRecording && !status.is_recording { return .handleUnexpectedStop }
        if status.sample_rate_changed { return .restartForSampleRateChange }
        if status.stream_error { return .recoverFromStreamError }
        if status.write_failed { return .stopForWriteFailure }
        if status.disk_space_low { return .stopForLowDiskSpace }
        if status.write_errors > UInt64(excessiveWriteErrors) { return .stopForExcessiveWriteErrors }
        return .keepRecording
    }

    /// The DOLL-351 bookkeeping for one stream-error restart attempt.
    struct StreamRestart: Equatable {
        /// Restarts in the current burst, including this one.
        let count: Int
        /// Whether this restart may run; `false` means stop for good.
        let isAllowed: Bool
    }

    /// Count a stream-error restart at `now`, given the burst so far.
    static func streamRestart(previousCount: Int, lastRestart: Date?, now: Date) -> StreamRestart {
        let count: Int
        if let lastRestart, now.timeIntervalSince(lastRestart) < streamRestartWindow {
            count = previousCount + 1
        } else {
            count = 1
        }
        return StreamRestart(count: count, isAllowed: count <= maxConsecutiveStreamRestarts)
    }
}
