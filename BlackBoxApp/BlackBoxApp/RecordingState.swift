import Foundation
import AppKit
import AVFoundation
import Observation
import os.log
import UserNotifications

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
enum SleepWakePolicy {
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
}

/// Observable state for the menu bar UI, wrapping the Rust audio engine via FFI.
///
/// Every public stored property here is a SwiftUI binding target. Views observe
/// these via `@Observable` change tracking; updates land on the main thread
/// (the class is `@MainActor`-isolated) so binding reads are race-free.
@MainActor
@Observable final class RecordingState {
    /// `true` while a recording session is active. Flips on a successful
    /// `start()` and clears on `stop()` or any FFI-reported failure.
    /// Drives the menu bar icon, the Start/Stop button, and the menu's
    /// "currently recording" caption.
    var isRecording = false

    /// `true` while the level meter is actively pulling peak levels from
    /// the audio engine without persisting to disk. Mutually exclusive
    /// with `isRecording` in practice — starting recording stops monitoring.
    var isMonitoring = false

    /// Short status string for the menu's headline row ("Ready",
    /// "Recording...", "Error", elapsed time during a session). Always
    /// non-empty; defaults to "Ready" pre-launch.
    var statusText = "Ready"

    /// Latest user-visible error, or `nil` when the app is healthy.
    /// Set by `setTransientError(_:)` (which auto-clears after a delay)
    /// or by hard failures like denied output-folder access. SwiftUI
    /// renders this in a red caption directly below `statusText`.
    var errorMessage: String?

    /// Names of input devices CoreAudio currently exposes. Populated by
    /// `refreshDevices()` at init and on user-triggered "Refresh Devices".
    /// Empty until refresh completes; the menu shows "No Input Devices"
    /// in that case.
    var availableDevices: [String] = []

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

    /// Active capture sample rate in Hz, or `0` when no session is running.
    /// Persisted to UserDefaults at session start so the meter window can
    /// label its grid before the next session brings the engine up.
    var sampleRate: Int = UserDefaults.standard.integer(forKey: "lastSampleRate")

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
                startMeterTimer()
                if !isRecording {
                    startMonitoring()
                }
            } else {
                stopMeterTimer()
                if isMonitoring {
                    stopMonitoring()
                }
            }
        }
    }

    let bridge: RustBridge
    private var recordingStartTime: Date?
    private var timerTask: Task<Void, Never>?
    private var meterTimerTask: Task<Void, Never>?

    // wasSleepInterrupted (declared below) is set by both `handleWillSleep`
    // and `handleSessionDidResignActive` when their `SleepWakePolicy`
    // decision is `.pauseForResume`. It's cleared by `handleDidWake`,
    // `handleSessionDidBecomeActive`, and `stop()` (DOLL-182 — without
    // that last reset, a manual stop inside the 1.5s deferred-resume
    // window let the resume Task resurrect a recording the user
    // explicitly stopped).
    private var securityScopedURL: URL?
    private var lastReportedWriteErrors: Int = 0
    private var peakBuffer = [Float](repeating: 0, count: 255)
    private var meterPollCount: Int = 0
    private var meterPollTotalNs: UInt64 = 0
    private var activityToken: (any NSObjectProtocol)?
    private var wasSleepInterrupted = false

    /// Bookmark-restore Task (DOLL-181). Stored so auto-record can `await`
    /// it before starting, preventing a race where auto-record fires with
    /// the default output dir because the bookmark Task hadn't completed
    /// yet. `nil` until init kicks the Task off; `nil` after restoration
    /// completes (we never read it later so dropping the reference is fine).
    private var bookmarkRestoreTask: Task<Void, Never>?

    private static let bookmarkKey = "outputDirBookmark"
    private static let log = Logger(subsystem: "com.dollhousemediatech.blackbox", category: "RecordingState")

    /// Enable verbose logging to macOS Console. Toggle via UserDefaults key "debugLogging".
    /// Cached to avoid a UserDefaults lookup on every 30 Hz meter tick.
    private var debugLogging: Bool = UserDefaults.standard.bool(forKey: "debugLogging")

    /// True when running inside an XCTest host — skips hardware-dependent init.
    private static let isTesting = NSClassFromString("XCTestCase") != nil

    init() {
        bridge = RustBridge()
        guard !Self.isTesting else { return }
        refreshDevices()
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
        bookmarkRestoreTask = Task { [weak self] in
            await self?.restoreOutputDirBookmark()
        }
        restoreSavedSettings()
        restoreGlobalHotkey()

        // Request notification authorization eagerly at launch (DOLL-134).
        // Previously this was deferred to first manual `startRecordingInternal`,
        // so the very-first auto-record-on-launch notification fired before
        // auth and was silently dropped. Auth status is sticky across
        // launches; calling once here is a no-op on subsequent runs.
        requestNotificationAuth()

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
                self.start()
                if self.isRecording {
                    self.postNotification(title: "Recording Started",
                                          body: "BlackBox started recording automatically.",
                                          identifier: "auto-record-started")
                }
            }
        }
    }

    // MARK: - Sleep / Wake

    private func beginPreventingSleep() {
        guard activityToken == nil else { return }
        let idleDisabled = UserDefaults.standard.object(forKey: SettingsKeys.preventSleep) as? Bool ?? true
        var opts: ProcessInfo.ActivityOptions = .userInitiated  // always prevent App Nap
        if SleepWakePolicy.shouldPreventSleep(settingEnabled: idleDisabled) {
            opts.insert(.idleSystemSleepDisabled)
        }
        activityToken = ProcessInfo.processInfo.beginActivity(
            options: opts,
            reason: "BlackBox is recording audio"
        )
        Self.log.info("Sleep prevention: appNap=always idleSleep=\(idleDisabled)")
    }

    private func endPreventingSleep() {
        guard let token = activityToken else { return }
        ProcessInfo.processInfo.endActivity(token)
        activityToken = nil
        Self.log.info("Sleep prevention disabled")
    }

    func handleWillSleep() {
        let behavior = UserDefaults.standard.string(forKey: SettingsKeys.sleepBehavior) ?? "resume"
        let action = SleepWakePolicy.sleepAction(isRecording: isRecording, behavior: behavior)
        switch action {
        case .ignore:
            return
        case .pauseForResume:
            wasSleepInterrupted = true
            postNotification(title: "Recording Paused",
                             body: "Your Mac is going to sleep. Recording will resume on wake.",
                             identifier: "sleep-paused")
        case .stop:
            postNotification(title: "Recording Stopped",
                             body: "Your Mac is going to sleep.",
                             identifier: "recording-stopped")
        }
        stop()
        Self.log.info("Sleep: stopped recording (behavior=\(behavior))")
    }

    func handleDidWake() {
        guard SleepWakePolicy.shouldResumeOnWake(wasInterrupted: wasSleepInterrupted) else { return }
        wasSleepInterrupted = false
        Self.log.info("Wake: attempting to resume recording")
        Task { [weak self] in
            try? await Task.sleep(for: .milliseconds(1500))
            guard let self, !self.isRecording else { return }
            self.start()
            if self.isRecording {
                self.postNotification(title: "Recording Resumed",
                                      body: "Recording resumed after wake.",
                                      identifier: "wake-resumed")
            } else {
                self.postNotification(title: "Resume Failed",
                                      body: "Could not restart recording after wake. Check your audio device.",
                                      identifier: "wake-failed")
            }
        }
    }

    func handleSessionDidResignActive() {
        let action = SleepWakePolicy.sessionResignAction(isRecording: isRecording)
        guard action == .pauseForResume else { return }
        wasSleepInterrupted = true
        stop()
        Self.log.info("Fast User Switch: stopped recording for resume on return")
        postNotification(title: "Recording Paused",
                         body: "User session switched. Recording will resume when you return.",
                         identifier: "session-paused")
    }

    func handleSessionDidBecomeActive() {
        guard SleepWakePolicy.shouldResumeOnWake(wasInterrupted: wasSleepInterrupted) else { return }
        wasSleepInterrupted = false
        Self.log.info("Fast User Switch: attempting to resume recording")
        Task { [weak self] in
            try? await Task.sleep(for: .milliseconds(1500))
            guard let self, !self.isRecording else { return }
            self.start()
            if self.isRecording {
                self.postNotification(title: "Recording Resumed",
                                      body: "Recording resumed after session switch.",
                                      identifier: "session-resumed")
            } else {
                self.postNotification(title: "Resume Failed",
                                      body: "Could not restart recording after session switch.",
                                      identifier: "session-failed")
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
                "Shortcut \(shortcut.displayString) couldn't be registered — another app may be using it. Pick a new shortcut in Settings."
            )
        }
    }

    // MARK: - Actions

    func toggle() {
        if isRecording {
            stop()
        } else {
            start()
        }
    }

    func start() {
        // Debounce: rapid double-start (e.g. hotkey held, accessibility
        // automation) would otherwise launch two requestAccess flows in
        // parallel. The Task hop below means isRecording stays false during
        // the await, so this guard is the only thing protecting against
        // re-entry from the same MainActor turn.
        guard !isRecording else { return }
        errorMessage = nil
        Task { @MainActor in
            // The Task scope ends naturally when this function returns;
            // app termination cancels in-flight Tasks via Swift's
            // structured-concurrency cooperation, so no explicit
            // Task.cancel is required from applicationShouldTerminate.
            if await self.checkMicrophonePermission() {
                self.startRecordingInternal()
            } else {
                self.errorMessage = "Microphone access denied. Open System Settings to allow access."
                self.statusText = "Error"
            }
        }
    }

    private func startRecordingInternal() {
        // Notification authorization was requested at init() time (DOLL-134),
        // so we don't need a lazy request here.

        // Stop monitoring first — recording will take over the audio stream
        if isMonitoring {
            stopMonitoring()
        }

        let result = bridge.startRecording()
        if result.isSuccess {
            isRecording = true
            recordingStartTime = Date()
            statusText = "Recording..."
            lastReportedWriteErrors = 0
            startTimer()
            beginPreventingSleep()
            Self.log.info("Recording started")
            NSAccessibility.post(element: NSApp as Any, notification: .announcementRequested,
                                 userInfo: [.announcement: "Recording started"])
        } else {
            isRecording = false
            recordingStartTime = nil
            let detail = bridge.lastError
            let err: String
            switch result {
            case .audioDevice:
                err = "No audio input device found. Check System Settings \u{203A} Sound."
            case .config:
                err = "Configuration error: \(detail ?? "invalid settings")"
            case .io:
                err = "Recording failed: disk error"
            default:
                err = detail ?? "Failed to start recording"
            }
            setTransientError(err)
            Self.log.error("Failed to start recording (code \(result.rawValue)): \(err)")
        }
    }

    // MARK: - Monitoring

    func startMonitoring() {
        Task { @MainActor in
            guard await self.checkMicrophonePermission() else { return }
            let result = self.bridge.startMonitoring()
            if result.isSuccess {
                self.isMonitoring = true
                Self.log.info("Audio monitoring started")
            } else {
                Self.log.error("Failed to start monitoring (code \(result.rawValue)): \(self.bridge.lastError ?? "unknown")")
            }
        }
    }

    func stopMonitoring() {
        if bridge.stopMonitoring().isSuccess {
            isMonitoring = false
            peakLevels = []
            Self.log.info("Audio monitoring stopped")
        }
    }

    /// Restart monitoring to pick up config changes (channels, device).
    /// No-op if not currently monitoring.
    func restartMonitoring() {
        guard isMonitoring else { return }
        stopMonitoring()
        startMonitoring()
    }

    // MARK: - Microphone Permission

    private func checkMicrophonePermission() async -> Bool {
        switch AVCaptureDevice.authorizationStatus(for: .audio) {
        case .authorized:
            return true
        case .notDetermined:
            return await AVCaptureDevice.requestAccess(for: .audio)
        case .denied, .restricted:
            showMicrophonePermissionAlert()
            return false
        @unknown default:
            return false
        }
    }

    private func showMicrophonePermissionAlert() {
        let alert = NSAlert()
        alert.messageText = "Microphone Access Required"
        alert.informativeText = "BlackBox needs microphone access to record audio. You can allow access in System Settings > Privacy & Security > Microphone."
        alert.alertStyle = .warning
        alert.addButton(withTitle: "Open System Settings")
        alert.addButton(withTitle: "Cancel")

        NSApp.activate(ignoringOtherApps: true)
        if alert.runModal() == .alertFirstButtonReturn {
            if let url = URL(string: "x-apple.systempreferences:com.apple.preference.security?Privacy_Microphone") {
                NSWorkspace.shared.open(url)
            }
        }
    }

    // MARK: - Notifications

    private func requestNotificationAuth() {
        let center = UNUserNotificationCenter.current()
        // DOLL-185: capture the granted bool. Without this, a denial
        // silently drops every later postNotification (sleep-paused,
        // recording-stopped, wake events) and the user has no signal.
        center.requestAuthorization(options: [.alert, .sound]) { [weak self] granted, error in
            if let error {
                Self.log.warning("Notification auth request failed: \(error.localizedDescription, privacy: .public)")
            }
            Task { @MainActor in
                self?.notificationsAuthorized = granted
            }
        }

        // Register "Restart Recording" action on recording-stopped notifications
        let restartAction = UNNotificationAction(
            identifier: "restart-recording",
            title: "Restart Recording")
        let category = UNNotificationCategory(
            identifier: "recording-stopped",
            actions: [restartAction],
            intentIdentifiers: [])
        center.setNotificationCategories([category])
        center.delegate = notificationDelegate
    }

    /// Re-query notification authorization. Called from app-becomes-active
    /// so a user who grants permission in System Settings has the app
    /// pick that up without a relaunch (DOLL-185).
    func refreshNotificationAuthorization() {
        UNUserNotificationCenter.current().getNotificationSettings { [weak self] settings in
            let granted = settings.authorizationStatus == .authorized
                || settings.authorizationStatus == .provisional
            Task { @MainActor in
                self?.notificationsAuthorized = granted
            }
        }
    }

    /// Delegate that handles notification action responses (e.g. "Restart Recording").
    /// Stored as an instance property to keep the delegate alive.
    private let notificationDelegate = NotificationDelegate()

    /// Post a notification to Notification Center for events that occur while the app is in the background.
    /// Uses a fixed identifier so new notifications of the same type replace old ones instead of stacking.
    private func postNotification(title: String, body: String, identifier: String = "blackbox-info") {
        let content = UNMutableNotificationContent()
        content.title = title
        content.body = body
        content.sound = isRecording ? nil : .default
        if identifier == "recording-stopped" {
            content.categoryIdentifier = "recording-stopped"
        }
        let request = UNNotificationRequest(identifier: identifier, content: content, trigger: nil)
        UNUserNotificationCenter.current().add(request)
    }

    /// Notify the user of a critical event using the appropriate channel:
    /// modal alert if the app is in the foreground, notification if backgrounded.
    /// Avoids showing both simultaneously.
    private func notifyUser(title: String, message: String, identifier: String = "recording-stopped") {
        if NSApp.isActive {
            showCriticalAlert(title: title, message: message)
        } else {
            postNotification(title: title, body: message, identifier: identifier)
        }
    }

    /// Set a transient error that auto-clears after 30 seconds.
    /// Use for errors that don't require ongoing user action (device disconnect, disk full, etc.).
    private func setTransientError(_ message: String) {
        errorMessage = message
        statusText = "Error"
        Task { [weak self] in
            try? await Task.sleep(for: .seconds(30))
            guard let self, self.errorMessage == message else { return }
            self.errorMessage = nil
            if !self.isRecording { self.statusText = "Ready" }
        }
    }

    /// Show an NSAlert for critical errors that require the user's attention.
    private func showCriticalAlert(title: String, message: String) {
        let alert = NSAlert()
        alert.messageText = title
        alert.informativeText = message
        alert.alertStyle = .warning
        alert.addButton(withTitle: "OK")
        NSApp.activate(ignoringOtherApps: true)
        alert.runModal()
    }

    func stop() {
        let sessionDuration = recordingStartTime.map { Date().timeIntervalSince($0) } ?? 0
        stopTimer()
        let result = bridge.stopRecording()
        endPreventingSleep()
        if result.isSuccess {
            isRecording = false
            recordingStartTime = nil
            peakLevels = []
            errorMessage = nil
            statusText = "Ready"
            // DOLL-182: clear the resume-on-wake flag here. Without this,
            // a manual stop within the 1.5s deferred-resume window after
            // sleep/wake or session resign/activate would let the deferred
            // start() resurrect a recording the user explicitly stopped.
            wasSleepInterrupted = false
            Self.log.info("Recording stopped")
            NSAccessibility.post(element: NSApp as Any, notification: .announcementRequested,
                                 userInfo: [.announcement: "Recording stopped"])

            // Track successful sessions >5 min for App Store review prompt
            if sessionDuration > 300 {
                let key = "successfulRecordingSessions"
                UserDefaults.standard.set(UserDefaults.standard.integer(forKey: key) + 1, forKey: key)
            }

            // Resume monitoring if the meter window is still open
            if isMeterWindowOpen {
                startMonitoring()
            }
        } else {
            let err = bridge.lastError ?? "Failed to stop recording"
            setTransientError(err)
            Self.log.error("Failed to stop recording (code \(result.rawValue)): \(err)")
        }
    }

    func openOutputDir() {
        let config = bridge.getConfig()
        let dir = config?["output_dir"] as? String ?? "recordings"

        let url: URL
        if dir.hasPrefix("/") {
            url = URL(fileURLWithPath: dir)
        } else {
            let cwd = FileManager.default.currentDirectoryPath
            url = URL(fileURLWithPath: cwd).appendingPathComponent(dir)
        }

        // DOLL-114: defer the FileManager + NSWorkspace I/O off the main
        // actor. Both calls hit disk / Launch Services and were
        // synchronously blocking the UI on this user action.
        Task.detached {
            try? FileManager.default.createDirectory(at: url, withIntermediateDirectories: true)
            await MainActor.run { NSWorkspace.shared.open(url) }
        }
    }

    func refreshDevices() {
        availableDevices = RustBridge.listInputDevices()
        systemDefaultDeviceName = RustBridge.defaultInputDeviceName()
    }

    func selectDevice(_ name: String) {
        UserDefaults.standard.set(name, forKey: SettingsKeys.inputDevice)
        bridge.setConfig(["input_device": name])
        if isRecording {
            restartIfRecording(reason: "device changed")
        } else if isMonitoring {
            restartMonitoring()
        }
    }

    /// Finalize current WAV files and immediately start a new recording session
    /// with the updated config. No-op if not currently recording.
    func restartIfRecording(reason: String) {
        guard isRecording else { return }
        Self.log.info("Config changed while recording (\(reason)) — finalizing and restarting")
        stopTimer()
        _ = bridge.stopRecording()
        peakLevels = []
        lastReportedWriteErrors = 0
        startRecordingInternal()
    }

    // MARK: - Settings Persistence

    /// Restore all saved audio settings from UserDefaults and push to Rust engine.
    /// Called once at init, before auto-record fires.
    private func restoreSavedSettings() {
        let defaults = UserDefaults.standard
        var config: [String: Any] = [:]

        if let device = defaults.string(forKey: SettingsKeys.inputDevice), !device.isEmpty {
            config["input_device"] = device
        }
        if let channels = defaults.string(forKey: SettingsKeys.audioChannels) {
            if isLegacyZeroBasedSpec(channels) {
                // Migrate old 0-based spec to 1-based for UserDefaults
                let migrated = channelSpecToOneBased(channels)
                defaults.set(migrated, forKey: SettingsKeys.audioChannels)
                config["audio_channels"] = channels  // Already 0-based, pass directly
            } else {
                config["audio_channels"] = channelSpecToZeroBased(channels)
            }
        }
        config["output_mode"] = defaults.string(forKey: SettingsKeys.outputMode) ?? "split"

        // Silence threshold: reconstruct from enabled flag + threshold value
        let silenceEnabled = defaults.object(forKey: SettingsKeys.silenceEnabled) as? Bool ?? true
        let silenceThreshold = defaults.object(forKey: SettingsKeys.silenceThreshold) as? Double ?? 0.01
        config["silence_threshold"] = silenceEnabled ? silenceThreshold : 0.0

        // Output settings
        let continuousMode = defaults.object(forKey: SettingsKeys.continuousMode) as? Bool ?? false
        config["continuous_mode"] = continuousMode
        let cadence = defaults.integer(forKey: SettingsKeys.recordingCadence)
        if cadence > 0 {
            config["recording_cadence"] = cadence
        }

        // Disk space threshold
        let minDisk = defaults.integer(forKey: SettingsKeys.minDiskSpaceMB)
        if minDisk > 0 {
            config["min_disk_space_mb"] = minDisk
        }

        // Bit depth (0 means not yet set — use Rust default)
        let bitDepth = defaults.integer(forKey: SettingsKeys.bitDepth)
        if bitDepth > 0 {
            config["bits_per_sample"] = bitDepth
        }

        // Silence gate
        if defaults.object(forKey: SettingsKeys.silenceGateEnabled) != nil {
            config["silence_gate_enabled"] = defaults.bool(forKey: SettingsKeys.silenceGateEnabled)
        }
        let gateTimeout = defaults.integer(forKey: SettingsKeys.silenceGateTimeout)
        if gateTimeout > 0 {
            config["silence_gate_timeout_secs"] = gateTimeout
        }

        debugLogging = defaults.bool(forKey: "debugLogging")

        if !config.isEmpty {
            bridge.setConfig(config)
        }
    }

    // MARK: - Duration Timer

    private func startTimer() {
        timerTask = Task { [weak self] in
            while !Task.isCancelled {
                try? await Task.sleep(for: .seconds(1))
                guard !Task.isCancelled else { break }
                self?.updateDuration()
            }
        }
    }

    private func stopTimer() {
        timerTask?.cancel()
        timerTask = nil
    }

    // MARK: - Meter Timer (fast polling for level meter window)

    private func startMeterTimer() {
        guard meterTimerTask == nil else { return }
        meterTimerTask = Task { [weak self] in
            while !Task.isCancelled {
                try? await Task.sleep(for: .milliseconds(33))
                guard !Task.isCancelled else { break }
                self?.updatePeakLevels()
            }
        }
    }

    private func stopMeterTimer() {
        meterTimerTask?.cancel()
        meterTimerTask = nil
    }

    private func updatePeakLevels() {
        let debug = debugLogging
        let start: ContinuousClock.Instant? = debug ? .now : nil

        guard isRecording || isMonitoring else {
            if !peakLevels.isEmpty { peakLevels = [] }
            return
        }

        // DOLL-125: fillPeakLevels now returns Result so callers can
        // distinguish lock-poison / invalid-arg / invalid-handle from a
        // legitimate empty read. On error, log + leave peakLevels alone
        // (UI keeps showing the last good values rather than collapsing
        // to 0 channels every tick).
        let count: Int
        switch bridge.fillPeakLevels(into: &peakBuffer) {
        case .success(let n):
            count = n
        case .failure(let err):
            Self.log.error("fillPeakLevels failed: \(String(describing: err))")
            return
        }

        // Only publish when values have visibly changed (avoids SwiftUI diffing overhead)
        let needsUpdate: Bool
        if peakLevels.count != count {
            needsUpdate = true
        } else {
            var changed = false
            for i in 0..<count {
                if abs(peakBuffer[i] - peakLevels[i]) > 0.001 {
                    changed = true
                    break
                }
            }
            needsUpdate = changed
        }

        if needsUpdate {
            // DOLL-113: avoid the per-tick `Array(peakBuffer.prefix(count))`
            // alloc + copy. When the channel count is unchanged (the common
            // case in steady-state recording), `replaceSubrange` reuses
            // the existing storage. We still trigger one @Observable
            // notification per call.
            if peakLevels.count == count {
                peakLevels.replaceSubrange(0..<count, with: peakBuffer[0..<count])
            } else {
                // Channel count changed (e.g. recording started/stopped, or
                // device switched mid-session). Realloc is fine here — it
                // happens at most once per state transition, not per tick.
                peakLevels = Array(peakBuffer.prefix(count))
            }
        }

        if let start {
            let elapsed = ContinuousClock.now - start
            let (secs, atto) = elapsed.components
            meterPollTotalNs += UInt64(secs) &* 1_000_000_000 &+ UInt64(atto / 1_000_000_000)
            meterPollCount += 1
            if meterPollCount >= 30 {
                let avgNs = meterPollTotalNs / UInt64(meterPollCount)
                Self.log.info("[MeterPerf] avg=\(avgNs)ns over \(self.meterPollCount) ticks, ch=\(count)")
                meterPollCount = 0
                meterPollTotalNs = 0
            }
        }
    }

    private func updateDuration() {
        guard let start = recordingStartTime else { return }

        // Update elapsed time display
        let elapsed = Int(Date().timeIntervalSince(start))
        let hours = elapsed / 3600
        let minutes = (elapsed % 3600) / 60
        let seconds = elapsed % 60
        let elapsedText = hours > 0
            ? String(format: "Recording %d:%02d:%02d", hours, minutes, seconds)
            : String(format: "Recording %d:%02d", minutes, seconds)
        statusText = elapsedText

        // Check status from Rust engine (lightweight C struct, no JSON)
        if let status = bridge.getStatusFlags() {
            // Check if Rust engine stopped recording unexpectedly (device disconnect, etc.)
            if isRecording && !status.is_recording {
                stopTimer()
                isRecording = false
                recordingStartTime = nil
                let msg = bridge.lastError ?? "Recording stopped unexpectedly"
                setTransientError(msg)
                Self.log.error("Recording stopped unexpectedly: \(msg)")
                notifyUser(title: "Recording Stopped", message: msg)
                return
            }
            // DOLL-216: surface the silence-gate idle state as "Armed
            // (waiting for signal)". The previous "Waiting for audio…"
            // read as a passive failure mode; "Armed" reframes it as
            // ready-and-listening, which matches what the app is doing.
            if status.gate_idle {
                statusText = "Armed (waiting for signal)"
            }

            // Sample rate changed on the audio device — restart to pick up new rate
            // so the WAV header matches the actual audio data.
            if status.sample_rate_changed {
                Self.log.warning("Sample rate changed on device — finalizing and restarting")
                restartIfRecording(reason: "sample rate changed")
                notifyUser(title: "Sample Rate Changed",
                          message: "Your audio device's sample rate changed. Recording was restarted automatically.",
                          identifier: "sample-rate-changed")
                return
            }

            // Audio stream error — device disconnected or driver failure.
            // Finalize current files, then try to restart on the next available device.
            if status.stream_error {
                Self.log.error("Stream error detected — finalizing files and attempting restart")
                stopTimer()
                _ = bridge.stopRecording()
                peakLevels = []
                lastReportedWriteErrors = 0

                if bridge.startRecording().isSuccess {
                    // Restarted successfully (e.g., System Default fell back to built-in mic)
                    recordingStartTime = Date()
                    statusText = "Recording..."
                    startTimer()
                    Self.log.info("Recording restarted on available device")
                    notifyUser(title: "Device Changed",
                              message: "Your audio device changed. Recording continued on the next available device.",
                              identifier: "device-changed")
                } else {
                    // No device available — stop for real
                    isRecording = false
                    recordingStartTime = nil
                    let msg = "Your audio device was disconnected and no alternative is available. Check your connections and try again."
                    setTransientError(msg)
                    notifyUser(title: "Recording Stopped", message: msg)
                }
                return
            }
            // Disk space low — stop recording gracefully
            if status.disk_space_low {
                stop()
                let msg = "Your disk is almost full. Free up space and try again."
                setTransientError(msg)
                Self.log.error("Disk space low, stopping recording")
                notifyUser(title: "Recording Stopped", message: msg)
                return
            }
            // Write errors — cumulative counter from Rust engine
            let writeErrors = Int(status.write_errors)
            let newDrops = writeErrors - lastReportedWriteErrors

            if writeErrors > 48_000 {
                // Auto-stop if excessive (>48000 samples dropped across all channels)
                stop()
                let msg = "Recording quality degraded \u{2014} your Mac may be under heavy load. Try closing other applications."
                setTransientError(msg)
                Self.log.error("Excessive write errors (\(writeErrors)), stopping recording")
                notifyUser(title: "Recording Stopped", message: msg)
                return
            } else if newDrops > 0 {
                // Only log/display when NEW drops occur (counter is cumulative)
                lastReportedWriteErrors = writeErrors
                Self.log.warning("Write errors: \(newDrops) new samples dropped (\(writeErrors) total)")
                if writeErrors > 500 {
                    errorMessage = "Audio quality degraded \u{2014} some data was lost"
                }
            }

            // Sample rate — update for file size estimates in settings
            let rate = Int(status.sample_rate)
            if rate > 0, rate != sampleRate {
                sampleRate = rate
                UserDefaults.standard.set(rate, forKey: "lastSampleRate")
            }
        }
    }

    // MARK: - Security-Scoped Bookmarks

    /// Save a security-scoped bookmark for the chosen output directory.
    /// Creates the directory if it doesn't exist.
    func saveOutputDirBookmark(for url: URL) {
        do {
            try FileManager.default.createDirectory(at: url, withIntermediateDirectories: true)
            let bookmarkData = try url.bookmarkData(
                options: .withSecurityScope,
                includingResourceValuesForKeys: nil,
                relativeTo: nil
            )
            UserDefaults.standard.set(bookmarkData, forKey: Self.bookmarkKey)
            UserDefaults.standard.set(url.path, forKey: SettingsKeys.lastOutputDirPath)

            // Release previous access if any
            securityScopedURL?.stopAccessingSecurityScopedResource()
            securityScopedURL = url

            // Update Rust config with the chosen path
            bridge.setConfig(["output_dir": url.path])
            Self.log.info("Saved output directory bookmark: \(url.path)")
        } catch {
            let err = "Failed to save directory bookmark: \(error.localizedDescription)"
            errorMessage = err
            Self.log.error("\(err)")
        }
    }

    /// Restore the security-scoped bookmark on launch.
    ///
    /// DOLL-114: declared `async` so the bookmark resolution + security
    /// scope acquisition + bridge.setConfig (each of which can hit disk
    /// or IPC) run off the main actor's launch path.
    private func restoreOutputDirBookmark() async {
        guard let data = UserDefaults.standard.data(forKey: Self.bookmarkKey) else {
            Self.log.info("No saved output directory bookmark")
            return
        }
        do {
            var isStale = false
            let url = try URL(
                resolvingBookmarkData: data,
                options: .withSecurityScope,
                relativeTo: nil,
                bookmarkDataIsStale: &isStale
            )
            if url.startAccessingSecurityScopedResource() {
                securityScopedURL = url
                bridge.setConfig(["output_dir": url.path])
                UserDefaults.standard.set(url.path, forKey: SettingsKeys.lastOutputDirPath)
                Self.log.info("Restored output directory: \(url.path)\(isStale ? " (stale, refreshing)" : "")")
            } else {
                Self.log.warning("Failed to access security-scoped resource: \(url.path)")
                promptToReselectOutputDir(failedPath: url.path)
            }
            if isStale {
                saveOutputDirBookmark(for: url)
            }
        } catch {
            Self.log.error("Failed to restore bookmark: \(error.localizedDescription)")
            UserDefaults.standard.removeObject(forKey: Self.bookmarkKey)
            let failedPath = UserDefaults.standard.string(forKey: SettingsKeys.lastOutputDirPath) ?? "the configured directory"
            promptToReselectOutputDir(failedPath: failedPath)
        }
    }

    /// Show an alert asking the user to re-select their output directory when
    /// a security-scoped bookmark can no longer be resolved (e.g. volume unmounted).
    private func promptToReselectOutputDir(failedPath: String) {
        // Defer to next run loop so init() completes before showing UI
        Task { [weak self] in
            guard let self else { return }
            let alert = NSAlert()
            alert.messageText = "Output Directory Unavailable"
            alert.informativeText = "BlackBox can no longer access \"\(failedPath)\". Please select a new output directory, or use the default location."
            alert.alertStyle = .warning
            alert.addButton(withTitle: "Choose Directory\u{2026}")
            alert.addButton(withTitle: "Use Default")
            NSApp.activate(ignoringOtherApps: true)
            if alert.runModal() == .alertFirstButtonReturn {
                let panel = NSOpenPanel()
                panel.canChooseDirectories = true
                panel.canChooseFiles = false
                panel.canCreateDirectories = true
                panel.prompt = "Select"
                panel.message = "Select output directory for recordings"
                if panel.runModal() == .OK, let url = panel.url {
                    self.saveOutputDirBookmark(for: url)
                }
            } else {
                // Use default: ~/Music/BlackBox Recordings
                let musicDir = FileManager.default.homeDirectoryForCurrentUser
                    .appendingPathComponent("Music")
                    .appendingPathComponent("BlackBox Recordings")
                self.saveOutputDirBookmark(for: musicDir)
            }
        }
    }

    /// Release security-scoped resource access.
    func releaseOutputDirAccess() {
        securityScopedURL?.stopAccessingSecurityScopedResource()
        securityScopedURL = nil
    }
}

// MARK: - Notification Action Handler

/// Handles notification action responses (e.g. "Restart Recording" button).
/// Separate class because UNUserNotificationCenterDelegate requires NSObject conformance.
private class NotificationDelegate: NSObject, UNUserNotificationCenterDelegate {
    func userNotificationCenter(
        _ center: UNUserNotificationCenter,
        didReceive response: UNNotificationResponse,
        withCompletionHandler handler: @escaping () -> Void
    ) {
        if response.actionIdentifier == "restart-recording" {
            Task { @MainActor in
                // Find the RecordingState — it's the source of truth for the app
                if let app = NSApp.delegate as? AppDelegate, let recorder = app.recorder {
                    recorder.start()
                }
            }
        }
        handler()
    }

    /// Show notifications even when the app is in the foreground (needed for
    /// notification actions to be accessible).
    func userNotificationCenter(
        _ center: UNUserNotificationCenter,
        willPresent notification: UNNotification,
        withCompletionHandler handler: @escaping (UNNotificationPresentationOptions) -> Void
    ) {
        handler([.banner])
    }
}
