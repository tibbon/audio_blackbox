import Foundation
import Observation

// A declaration import (its own swift-format group) because a plain `import os.log`
// can't satisfy both linters: swift-format orders imports by ASCII (lowercase last)
// while swiftlint's sorted_imports is case-insensitive. The RecordingState+*.swift
// extensions use the same form.
import struct os.Logger

/// Observable state for the menu bar UI, wrapping the Rust audio engine via FFI.
///
/// Every public stored property here is a SwiftUI binding target. Views observe
/// these via `@Observable` change tracking; updates land on the main thread
/// (the class is `@MainActor`-isolated) so binding reads are race-free.
///
/// This file holds the stored state, `init`, and the launch-time restore
/// helpers. Behavior lives in `RecordingState+*.swift` extensions grouped by
/// concern: Session (start/stop), StatusPoll (the 1 Hz engine poll),
/// Monitoring (level meter), SleepWake, Notifications, OutputDirectory.
/// Swift's `private` does not reach across files, so the session bookkeeping
/// those extensions share is internal (DOLL-653); nothing outside
/// `RecordingState` should read or write it.
@MainActor
@Observable
final class RecordingState {
    /// `true` while a recording session is active. Flips on a successful
    /// `start()` and clears on `stop()` or any FFI-reported failure.
    /// Drives the menu bar icon, the Start/Stop button, and the menu's
    /// "currently recording" caption.
    var isRecording = false {
        didSet { syncMeterTimer() }  // DOLL-374: gate the meter poll on activity
    }

    /// `true` while the level meter is actively pulling peak levels from
    /// the audio engine without persisting to disk. Mutually exclusive
    /// with `isRecording` in practice — starting recording stops monitoring.
    var isMonitoring = false {
        didSet { syncMeterTimer() }  // DOLL-374: gate the meter poll on activity
    }

    /// `true` from the moment `start()` passes its guard until the start
    /// attempt resolves (success, failure, or permission denial).
    /// `isRecording` stays false across the mic-permission await inside
    /// `start()`'s Task — which can suspend for the entire user-facing
    /// permission dialog — so this in-flight flag is what blocks a second
    /// `start()` (hotkey + menu click, or a hotkey during the dialog) from
    /// enqueueing a second engine start whose failure path would mark the
    /// live recording as idle and make it unstoppable from the UI (DOLL-459).
    var isStartingRecording = false

    /// Short status string for the menu's headline row ("Ready",
    /// "Recording...", "Error", elapsed time during a session). Always
    /// non-empty; defaults to "Ready" pre-launch.
    var statusText = String(localized: "Ready")

    /// Latest user-visible error, or `nil` when the app is healthy.
    /// Set by `setTransientError(_:)` (which auto-clears after a delay)
    /// or by hard failures like denied output-folder access. SwiftUI
    /// renders this in a red caption directly below `statusText`.
    var errorMessage: String?

    /// Names of input devices CoreAudio currently exposes. Populated by
    /// `refreshDevices()` at init, whenever CoreAudio reports a device-list
    /// or default-input change, and on "Refresh Devices". Empty until
    /// refresh completes; the menu shows "No Input Devices" in that case.
    var availableDevices: [String] = []

    /// Refreshes `availableDevices` when devices come and go.
    var deviceListObserver: AudioDeviceListObserver?
    /// Coalesces a burst of device notifications into one refresh.
    var deviceRefreshTask: Task<Void, Never>?

    /// The actual device the system default resolves to (e.g. "MacBook
    /// Pro Microphone"), refreshed alongside `availableDevices`. nil if
    /// CoreAudio has no default input device. DOLL-215: lets the menu and
    /// Settings show "System Default (resolved name)" instead of a
    /// literal that tells the user nothing.
    var systemDefaultDeviceName: String?

    /// Per-channel peak amplitude in linear scale, 0.0...1.0. Updated at
    /// ~30 Hz while a recording or monitoring session is active, and
    /// only when the meter window is open (the timer is paused otherwise
    /// to avoid pointless FFI calls). Empty until the first poll lands.
    var peakLevels: [Float] = []

    /// The 1-based device channel each `peakLevels` entry belongs to, worked
    /// out when a recording or monitoring session starts. The meter labels
    /// bars with these; it falls back to positions if the counts disagree.
    var meterChannelNumbers: [Int] = []

    /// Active capture sample rate in Hz, or `0` when no session is running.
    /// Persisted to UserDefaults at session start so the meter window can
    /// label its grid before the next session brings the engine up.
    var sampleRate: Int = UserDefaults.standard.integer(forKey: SettingsKeys.lastSampleRate)

    /// `true` once `UNUserNotificationCenter` reports authorization granted,
    /// `false` when the user denied or hasn't yet responded. Updated by the
    /// init-time auth request and re-checked when the app becomes active
    /// (so granting in System Settings is picked up without a relaunch).
    /// Observed by UI that needs to fall back when notifications are off
    /// (DOLL-185).
    var notificationsAuthorized: Bool = false

    /// Tracks whether the level meter window is currently visible. Setting
    /// this starts/stops the meter polling timer and (when not recording)
    /// the underlying monitoring stream.
    var isMeterWindowOpen: Bool = false {
        didSet {
            if isMeterWindowOpen {
                if !isRecording {
                    startMonitoring()
                }
            } else {
                // A closed window starts visible again when reopened.
                isMeterWindowOccluded = false
                if isMonitoring {
                    stopMonitoring()
                }
            }
            syncMeterTimer()
        }
    }

    /// Whether the meter window is currently occluded — covered by another
    /// window, minimized, or on an inactive Space. DOLL-348: SwiftUI's
    /// `onDisappear` does NOT fire for occlusion, so without this the 30 Hz
    /// poll keeps waking the CPU to cross the FFI boundary and redraw a window
    /// the user can't see, suppressing App Nap on battery. Driven by the
    /// window's `didChangeOcclusionStateNotification`. The monitoring stream is
    /// deliberately left tied to window-open (not occlusion) so quickly
    /// covering/uncovering the window doesn't thrash the cpal stream.
    var isMeterWindowOccluded: Bool = false {
        didSet { syncMeterTimer() }
    }

    let bridge: RustBridge
    /// When the active session started. Exposed publicly (read-only via
    /// the encapsulation of the surrounding mutation paths) so menu views
    /// can pass it to `Text(_, style: .timer)` which auto-ticks without
    /// triggering `@Observable` re-renders — fixing the highlight-reset
    /// bug where the per-tick `statusText` rewrite was resetting hover
    /// state on every open menu.
    var recordingStartTime: Date?
    var timerTask: Task<Void, Never>?
    var meterTimerTask: Task<Void, Never>?

    // wasSleepInterrupted (declared below) is set by both `handleWillSleep`
    // and `handleSessionDidResignActive` when their `SleepWakePolicy`
    // decision is `.pauseForResume`. It's cleared by `handleDidWake`,
    // `handleSessionDidBecomeActive`, and any `stop(reason: .user)`
    // (DOLL-182 — without that last reset, a manual stop inside the 1.5s
    // deferred-resume window let the resume Task resurrect a recording
    // the user explicitly stopped). The sleep-interruption stop passes
    // `.sleepInterruption` and leaves the flag alone (DOLL-442).
    var securityScopedURL: URL?
    var lastReportedWriteErrors: Int = 0

    // DOLL-351: cap rapid stream-error auto-restarts so a flapping device
    // (enumerates then immediately faults) can't spin an endless
    // stop/start/finalize loop. Restarts within the window below count toward
    // the cap; a restart after a stable run resets the counter. The cap and
    // window constants live with recoverFromStreamError in +StatusPoll.
    var streamRestartCount = 0
    var lastStreamRestart: Date?

    /// Total samples dropped since the active recording started. Mirrors
    /// `status.write_errors` from the engine, surfaced for UI display
    /// (DOLL-223). 0 means clean; non-zero means the writer fell behind
    /// the audio thread at some point. Reset to 0 on stop and restart so
    /// the value reflects the *current* recording, not lifetime totals.
    var writeErrorsCount: Int = 0

    /// True when a recording is running on battery power that's dropped
    /// below the macOS-equivalent "low battery" threshold of 20%
    /// (DOLL-225). Reset when the user plugs in or stops the recording.
    var isLowBatteryWarning: Bool = false

    /// Non-nil when the configuration at recording-start time will
    /// produce a per-file WAV bigger than the 4 GiB header cap, set by
    /// `evaluatePreflightFileSizeWarning` (DOLL-220). The recording
    /// proceeds — RIFF size is clamped via the existing DOLL-204 cap —
    /// but the user is informed before they accumulate hours of audio
    /// that downstream tools may refuse to read. Cleared on stop and
    /// when a fresh recording starts with safe settings.
    var preflightSizeWarning: String?

    /// The projection behind the last pre-flight evaluation, so a restart
    /// of a live session re-announces the warning only when it changed.
    @ObservationIgnored var lastPreflightEstimate: SessionPolicy.PreflightSizeEstimate?

    /// Wall-clock date the *current* file's rotation cycle ends —
    /// `recordingStartTime + ceil(elapsed / cadence) × cadence` — for the
    /// menu's `Text(_, style: .timer)` countdown (DOLL-214 v2). Computed
    /// on demand because the formula is deterministic from the public
    /// start time + cadence snapshot; avoiding per-tick @Observable
    /// writes fixes the menu-highlight-resets-every-second bug.
    var nextRotationDate: Date? {
        guard isRecording,
            let start = recordingStartTime,
            let snapshot = configSnapshot,
            snapshot.continuousMode,
            snapshot.recordingCadence > 0
        else { return nil }
        let cadence = TimeInterval(snapshot.recordingCadence)
        let elapsed = Date().timeIntervalSince(start)
        let cyclesCompleted = floor(elapsed / cadence)
        return start.addingTimeInterval((cyclesCompleted + 1) * cadence)
    }

    /// Estimated current-file size for the meter window header (DOLL-217
    /// v2 — relocated from the menu to fix the menu-flicker bug).
    /// Updated alongside `statusText` every duration tick; only consumed
    /// by `MeterView` which is a regular window and doesn't suffer the
    /// menu's hover-reset issue.
    var currentFileSizeText: String?

    /// Formatted duration of the just-stopped recording, shown as a
    /// transient summary block in the menu for ~30 s after Stop (DOLL-213)
    /// so the user gets a moment of "yes, that happened" feedback before
    /// the menu reverts to its idle state. Nil when no summary is active.
    var lastRecordingDurationText: String?

    /// Snapshot of the config values the per-tick UI computations need —
    /// captured at recording-start so updateDuration doesn't re-read
    /// UserDefaults 5-7 times per second for values that can only
    /// change via Settings (which already triggers restartIfRecording
    /// in the common case, refreshing this snapshot for the next tick).
    /// DOLL-233.
    struct RecordingConfigSnapshot {
        var continuousMode: Bool
        var recordingCadence: Int
        var channelCount: Int
        var bitDepth: Int
        var outputMode: String
    }
    var configSnapshot: RecordingConfigSnapshot?

    /// Polling counter for battery checks — `updateDuration` ticks every
    /// 1 s; we check the power source every 30 ticks so the IOKit
    /// query overhead is negligible and the warning latency stays
    /// reasonable (max ~30 s after threshold crossing).
    var batteryCheckTick: Int = 0

    /// One-shot guard so the user only gets a single low-battery
    /// notification per recording — the menu caption stays visible
    /// for ongoing reinforcement, but we don't spam the system tray.
    var batteryNotificationFired = false

    /// Tracks the previous tick's gate-idle state so the menu-flicker
    /// fix can write `statusText` only on the gate_idle↔active
    /// transition rather than every tick (the elapsed-time string is
    /// now rendered via Text(_, style: .timer)).
    var wasGateIdle = false
    var peakBuffer = [Float](repeating: 0, count: 255)
    var meterPollCount: Int = 0
    var meterPollTotalNs: UInt64 = 0
    var activityToken: (any NSObjectProtocol)?
    var wasSleepInterrupted = false
    /// The deferred resume scheduled by a wake / session-active handler;
    /// cancelled by a user stop or start (DOLL-182).
    var pendingResumeTask: Task<Void, Never>?

    /// Bookmark-restore Task (DOLL-181). Stored so auto-record can `await`
    /// it before starting, preventing a race where auto-record fires with
    /// the default output dir because the bookmark Task hadn't completed
    /// yet. The Task includes any "Output Directory Unavailable" prompt, so
    /// it completes only after the user has answered it, and it also runs
    /// launch-time crash recovery. Every start (`startAndWait`) awaits it.
    /// `nil` only in tests, where init returns before kicking it off.
    private(set) var bookmarkRestoreTask: Task<Void, Never>?

    /// `nonisolated` so completion handlers that run off the main actor (e.g. the
    /// UNUserNotificationCenter authorization callback) can log; Logger is Sendable.
    nonisolated static let log = Logger(
        subsystem: "com.dollhousemediatech.blackbox",
        category: "RecordingState"
    )

    /// The out-of-the-box default recordings directory, inside the app's
    /// sandbox container (`~/Library/Containers/<bundle-id>/Data/Documents/
    /// BlackBox Recordings`). DOLL-344: the previous default of
    /// `~/Music/BlackBox Recordings` is NOT writable under the App Store
    /// sandbox without the `assets.music` entitlement, and a non-user-selected
    /// URL cannot carry a `.withSecurityScope` bookmark — so recording was
    /// broken out of the box. The container is always writable with no
    /// entitlement and no security scope, so this is the safe default.
    static var defaultOutputDir: URL {
        let docs =
            (try? FileManager.default.url(
                for: .documentDirectory,
                in: .userDomainMask,
                appropriateFor: nil,
                create: false
            ))
            ?? FileManager.default.homeDirectoryForCurrentUser
            .appendingPathComponent("Documents", isDirectory: true)
        return docs.appendingPathComponent("BlackBox Recordings", isDirectory: true)
    }

    /// Enable verbose logging to macOS Console. Toggle via UserDefaults key "debugLogging".
    /// Cached to avoid a UserDefaults lookup on every 30 Hz meter tick.
    var debugLogging: Bool = UserDefaults.standard.bool(forKey: SettingsKeys.debugLogging)

    /// True when running inside an XCTest host — skips hardware-dependent init.
    private static let isTesting = NSClassFromString("XCTestCase") != nil

    /// Set once `requestNotificationAuth()` has run this launch, so the
    /// deferred first-launch request (DOLL-464) fires at most once.
    var hasRequestedNotificationAuth = false

    // swiftlint:disable weak_delegate - UNUserNotificationCenter.delegate is weak, so this must be the owning reference
    /// Delegate that handles notification action responses (e.g. "Restart Recording").
    /// Stored as an instance property to keep the delegate alive.
    let notificationDelegate = NotificationDelegate()
    // swiftlint:enable weak_delegate

    init() {
        bridge = RustBridge()
        notificationDelegate.recorder = self
        guard !Self.isTesting else { return }
        refreshDevices()
        deviceListObserver = AudioDeviceListObserver { [weak self] in
            self?.scheduleDeviceRefresh()
        }
        // DOLL-114: defer bookmark restoration off the launch path. The
        // synchronous URL+startAccessingSecurityScopedResource+setConfig
        // chain hit disk / IPC and delayed first menu-bar appearance.
        // Defer to a background Task so the menu bar appears with default
        // config; the real bookmarked path lands a moment later.
        //
        // DOLL-181: stash the Task so auto-record can `await` it before
        // calling `start()`. The old code raced — a 500 ms sleep wasn't
        // enough to guarantee the bookmark Task had completed first, and
        // a slow restore would auto-record into the sandbox default dir.
        // Recovery of crash-interrupted files rides on the same Task: it
        // needs the folder's scope, and must finish before any start.
        bookmarkRestoreTask = Task { [weak self] in
            self?.restoreOutputDirBookmark()
            await self?.recoverInterruptedRecordings()
        }
        restoreSavedSettings()
        restoreGlobalHotkey()

        // DOLL-464: request notification authorization at launch ONLY once
        // onboarding has completed (i.e. any launch after the first). On a
        // true first launch the eager request stacked a third permission
        // prompt on top of the onboarding window and the mic prompt — the
        // in-context pattern the HIG warns against. First-launch users get
        // the request when their first recording starts instead (see
        // startRecordingInternal), the first moment a notification could
        // matter. DOLL-134 is preserved: auto-record requires completed
        // onboarding, so its launch notification still finds auth already
        // requested here, and auth status stays sticky across launches.
        if UserDefaults.standard.bool(forKey: SettingsKeys.hasCompletedOnboarding) {
            requestNotificationAuth()
        }

        // Auto-record on launch if enabled (skip if onboarding not complete)
        if UserDefaults.standard.bool(forKey: SettingsKeys.hasCompletedOnboarding)
            && UserDefaults.standard.bool(forKey: SettingsKeys.autoRecord)
        {
            Task { [weak self] in
                // Wait for bookmark restoration before starting — without this,
                // auto-record would race the bookmark restore Task and may write
                // to the sandbox default directory instead of the user's chosen
                // folder (DOLL-181).
                await self?.bookmarkRestoreTask?.value
                try? await Task.sleep(for: .milliseconds(500))
                guard let self else { return }
                // DOLL-443: await the outcome — reading isRecording right
                // after a fire-and-forget start() races the permission await.
                if await startAndWait() {
                    postNotification(
                        title: String(localized: "Recording Started"),
                        body: String(localized: "BlackBox started recording automatically."),
                        identifier: "auto-record-started"
                    )
                }
            }
        }
    }

    // MARK: - Global Hotkey

    /// Restore and register the saved global keyboard shortcut.
    private func restoreGlobalHotkey() {
        let manager = GlobalHotkeyManager.shared
        manager.action = { [weak self] in
            self?.toggle()
        }
        if let shortcut = manager.loadSaved(), !manager.register(shortcut) {
            Self.log.warning(
                "Saved hotkey \(shortcut.displayString, privacy: .public) failed to register on launch"
            )
            // DOLL-184: surface the failure to the user instead of relying
            // on the log. The menu's existing errorMessage Label renders
            // this; the transient timer clears it after a while so the
            // user isn't permanently nagged.
            setTransientError(
                String(
                    localized:
                        "Shortcut \(shortcut.displayString) couldn't be registered — another app may be using it. Pick a new shortcut in Settings."
                )
            )
        }
    }

    // MARK: - Settings Persistence

    /// Restore all saved audio settings from UserDefaults and push to Rust engine.
    /// Called once at init, before auto-record fires.
    private func restoreSavedSettings() {
        let config = SavedEngineConfig.restore(from: UserDefaults.standard)
        debugLogging = UserDefaults.standard.bool(forKey: SettingsKeys.debugLogging)

        if !config.isEmpty {
            bridge.setConfig(config)
        }
    }
}
