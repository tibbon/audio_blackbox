import Foundation
import AppKit
import AVFoundation
import Combine
import os.log
import UserNotifications

/// Observable state for the menu bar UI, wrapping the Rust audio engine via FFI.
@MainActor
final class RecordingState: ObservableObject {
    @Published var isRecording = false
    @Published var isMonitoring = false
    @Published var statusText = "Ready"
    @Published var errorMessage: String?
    @Published var availableDevices: [String] = []
    @Published var peakLevels: [Float] = []
    @Published var sampleRate: Int = UserDefaults.standard.integer(forKey: "lastSampleRate")
    @Published var isMeterWindowOpen: Bool = false {
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
    private var timer: Timer?
    private var meterTimer: Timer?
    private var securityScopedURL: URL?
    private var lastReportedWriteErrors: Int = 0
    private var hasRequestedNotificationAuth = false
    private var peakBuffer = [Float](repeating: 0, count: 255)
    private var meterPollCount: Int = 0
    private var meterPollTotalNs: UInt64 = 0

    private static let bookmarkKey = "outputDirBookmark"
    private static let log = Logger(subsystem: "com.dollhousemediatech.blackbox", category: "RecordingState")

    /// Enable verbose logging to macOS Console. Toggle via UserDefaults key "debugLogging".
    /// Cached to avoid a UserDefaults lookup on every 30 Hz meter tick.
    private var debugLogging: Bool = UserDefaults.standard.bool(forKey: "debugLogging")

    init() {
        bridge = RustBridge()
        refreshDevices()
        restoreOutputDirBookmark()
        restoreSavedSettings()
        restoreGlobalHotkey()

        // Auto-record on launch if enabled (skip if onboarding not complete)
        if UserDefaults.standard.bool(forKey: SettingsKeys.hasCompletedOnboarding)
            && UserDefaults.standard.bool(forKey: SettingsKeys.autoRecord)
        {
            DispatchQueue.main.asyncAfter(deadline: .now() + 0.5) { [weak self] in
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

    // MARK: - Global Hotkey

    /// Restore and register the saved global keyboard shortcut.
    private func restoreGlobalHotkey() {
        let manager = GlobalHotkeyManager.shared
        manager.action = { [weak self] in
            self?.toggle()
        }
        if let shortcut = manager.loadSaved() {
            manager.register(shortcut)
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
        errorMessage = nil
        checkMicrophonePermission { [weak self] granted in
            guard let self else { return }
            if granted {
                self.startRecordingInternal()
            } else {
                self.errorMessage = "Microphone access denied. Open System Settings to allow access."
                self.statusText = "Error"
            }
        }
    }

    private func startRecordingInternal() {
        // Request notification permission on first recording start (contextually relevant)
        if !hasRequestedNotificationAuth {
            hasRequestedNotificationAuth = true
            requestNotificationAuth()
        }

        // Stop monitoring first — recording will take over the audio stream
        if isMonitoring {
            stopMonitoring()
        }

        if bridge.startRecording() {
            isRecording = true
            recordingStartTime = Date()
            statusText = "Recording..."
            lastReportedWriteErrors = 0
            startTimer()
            Self.log.info("Recording started")
            NSAccessibility.post(element: NSApp as Any, notification: .announcementRequested,
                                 userInfo: [.announcement: "Recording started"])
        } else {
            isRecording = false
            recordingStartTime = nil
            let err = bridge.lastError ?? "Failed to start recording"
            setTransientError(err)
            Self.log.error("Failed to start recording: \(err)")
        }
    }

    // MARK: - Monitoring

    func startMonitoring() {
        checkMicrophonePermission { [weak self] granted in
            guard let self else { return }
            if granted {
                if self.bridge.startMonitoring() {
                    self.isMonitoring = true
                    Self.log.info("Audio monitoring started")
                } else {
                    Self.log.error("Failed to start monitoring: \(self.bridge.lastError ?? "unknown")")
                }
            }
        }
    }

    func stopMonitoring() {
        if bridge.stopMonitoring() {
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

    private func checkMicrophonePermission(completion: @escaping (Bool) -> Void) {
        switch AVCaptureDevice.authorizationStatus(for: .audio) {
        case .authorized:
            Task { @MainActor in
                completion(true)
            }
        case .notDetermined:
            AVCaptureDevice.requestAccess(for: .audio) { granted in
                Task { @MainActor in
                    completion(granted)
                }
            }
        case .denied, .restricted:
            showMicrophonePermissionAlert()
            completion(false)
        @unknown default:
            completion(false)
        }
    }

    private func showMicrophonePermissionAlert() {
        let alert = NSAlert()
        alert.messageText = "Microphone Access Required"
        alert.informativeText = "BlackBox needs microphone access to record audio. You can allow access in System Settings > Privacy & Security > Microphone."
        alert.alertStyle = .warning
        alert.addButton(withTitle: "Open System Settings")
        alert.addButton(withTitle: "Cancel")

        if alert.runModal() == .alertFirstButtonReturn {
            if let url = URL(string: "x-apple.systempreferences:com.apple.preference.security?Privacy_Microphone") {
                NSWorkspace.shared.open(url)
            }
        }
    }

    // MARK: - Notifications

    private func requestNotificationAuth() {
        let center = UNUserNotificationCenter.current()
        center.requestAuthorization(options: [.alert, .sound]) { _, _ in }

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
        DispatchQueue.main.asyncAfter(deadline: .now() + 30) { [weak self] in
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
        if bridge.stopRecording() {
            isRecording = false
            recordingStartTime = nil
            peakLevels = []
            errorMessage = nil
            statusText = "Ready"
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
            Self.log.error("Failed to stop recording: \(err)")
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

        try? FileManager.default.createDirectory(at: url, withIntermediateDirectories: true)
        NSWorkspace.shared.open(url)
    }

    func refreshDevices() {
        availableDevices = RustBridge.listInputDevices()
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
        timer = Timer.scheduledTimer(withTimeInterval: 1, repeats: true) { [weak self] _ in
            MainActor.assumeIsolated {
                self?.updateDuration()
            }
        }
    }

    private func stopTimer() {
        timer?.invalidate()
        timer = nil
    }

    // MARK: - Meter Timer (fast polling for level meter window)

    private func startMeterTimer() {
        guard meterTimer == nil else { return }
        meterTimer = Timer.scheduledTimer(withTimeInterval: 1.0 / 30.0, repeats: true) { [weak self] _ in
            MainActor.assumeIsolated {
                self?.updatePeakLevels()
            }
        }
    }

    private func stopMeterTimer() {
        meterTimer?.invalidate()
        meterTimer = nil
    }

    private func updatePeakLevels() {
        let debug = debugLogging
        let start: ContinuousClock.Instant? = debug ? .now : nil

        guard isRecording || isMonitoring else {
            if !peakLevels.isEmpty { peakLevels = [] }
            return
        }

        let count = bridge.fillPeakLevels(into: &peakBuffer)

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
            peakLevels = Array(peakBuffer.prefix(count))
        }

        if let start {
            let elapsed = ContinuousClock.now - start
            meterPollTotalNs += UInt64(elapsed.components.attoseconds / 1_000_000_000)
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
        // Check if Rust engine stopped recording unexpectedly (device disconnect, etc.)
        if isRecording && !bridge.isRecording {
            stopTimer()
            isRecording = false
            recordingStartTime = nil
            let msg = bridge.lastError ?? "Recording stopped unexpectedly"
            setTransientError(msg)
            Self.log.error("Recording stopped unexpectedly: \(msg)")
            notifyUser(title: "Recording Stopped", message: msg)
            return
        }

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

        // Check status from Rust engine
        if let status = bridge.getStatus() {
            // Show "Waiting for audio..." when silence gate is idle
            if let gateIdle = status["gate_idle"] as? Bool, gateIdle {
                statusText = "Waiting for audio\u{2026}"
            }

            if debugLogging {
                Self.log.debug("Status poll: \(String(describing: status))")
            }

            // Sample rate changed on the audio device — restart to pick up new rate
            // so the WAV header matches the actual audio data.
            if let rateChanged = status["sample_rate_changed"] as? Bool, rateChanged {
                Self.log.warning("Sample rate changed on device — finalizing and restarting")
                restartIfRecording(reason: "sample rate changed")
                notifyUser(title: "Sample Rate Changed",
                          message: "Your audio device's sample rate changed. Recording was restarted automatically.",
                          identifier: "sample-rate-changed")
                return
            }

            // Audio stream error — device disconnected or driver failure.
            // Finalize current files, then try to restart on the next available device.
            if let streamError = status["stream_error"] as? Bool, streamError {
                Self.log.error("Stream error detected — finalizing files and attempting restart")
                stopTimer()
                _ = bridge.stopRecording()
                peakLevels = []
                lastReportedWriteErrors = 0

                if bridge.startRecording() {
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
            if let diskLow = status["disk_space_low"] as? Bool, diskLow {
                stop()
                let msg = "Your disk is almost full. Free up space and try again."
                setTransientError(msg)
                Self.log.error("Disk space low, stopping recording")
                notifyUser(title: "Recording Stopped", message: msg)
                return
            }
            // Write errors — cumulative counter from Rust engine
            if let writeErrors = status["write_errors"] as? Int {
                let newDrops = writeErrors - lastReportedWriteErrors

                if writeErrors > 48_000 {
                    // Auto-stop if excessive (>48000 ≈ 1 second at 48kHz)
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
            }

            // Sample rate — update for file size estimates in settings
            if let rate = status["sample_rate"] as? Int, rate > 0, rate != sampleRate {
                sampleRate = rate
                UserDefaults.standard.set(rate, forKey: "lastSampleRate")
            }
        } else if debugLogging {
            Self.log.debug("getStatus() returned nil")
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
    private func restoreOutputDirBookmark() {
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
        DispatchQueue.main.async { [weak self] in
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
