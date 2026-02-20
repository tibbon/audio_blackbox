import Foundation
import AppKit
import AVFoundation
import Combine
import os.log

/// Observable state for the menu bar UI, wrapping the Rust audio engine via FFI.
@MainActor
final class RecordingState: ObservableObject {
    @Published var isRecording = false
    @Published var statusText = "Ready"
    @Published var errorMessage: String?
    @Published var availableDevices: [String] = []
    @Published var peakLevels: [Double] = []
    @Published var sampleRate: Int = UserDefaults.standard.integer(forKey: "lastSampleRate")
    @Published var isMeterWindowOpen: Bool = false {
        didSet {
            if isMeterWindowOpen {
                startMeterTimer()
            } else {
                stopMeterTimer()
            }
        }
    }

    let bridge: RustBridge
    private var recordingStartTime: Date?
    private var timer: Timer?
    private var meterTimer: Timer?
    private var securityScopedURL: URL?
    private var lastReportedWriteErrors: Int = 0

    private static let bookmarkKey = "outputDirBookmark"
    private static let log = Logger(subsystem: "com.dollhousemediatech.blackbox", category: "RecordingState")

    /// Enable verbose logging to macOS Console. Toggle via UserDefaults key "debugLogging".
    private var debugLogging: Bool { UserDefaults.standard.bool(forKey: "debugLogging") }

    init() {
        bridge = RustBridge()
        refreshDevices()
        restoreOutputDirBookmark()
        restoreSavedSettings()

        // Auto-record on launch if enabled (skip if onboarding not complete)
        if UserDefaults.standard.bool(forKey: SettingsKeys.hasCompletedOnboarding)
            && UserDefaults.standard.bool(forKey: SettingsKeys.autoRecord)
        {
            DispatchQueue.main.asyncAfter(deadline: .now() + 0.5) { [weak self] in
                self?.start()
            }
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
                self.errorMessage = "Microphone access denied. Open System Settings to grant permission."
                self.statusText = "Error"
            }
        }
    }

    private func startRecordingInternal() {
        if bridge.startRecording() {
            isRecording = true
            recordingStartTime = Date()
            statusText = "Recording..."
            lastReportedWriteErrors = 0
            startTimer()
            Self.log.info("Recording started")
        } else {
            let err = bridge.lastError ?? "Failed to start recording"
            errorMessage = err
            statusText = "Error"
            Self.log.error("Failed to start recording: \(err)")
        }
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
        alert.informativeText = "BlackBox needs microphone access to record audio. Please grant access in System Settings > Privacy & Security > Microphone."
        alert.alertStyle = .warning
        alert.addButton(withTitle: "Open System Settings")
        alert.addButton(withTitle: "Cancel")

        if alert.runModal() == .alertFirstButtonReturn {
            if let url = URL(string: "x-apple.systempreferences:com.apple.preference.security?Privacy_Microphone") {
                NSWorkspace.shared.open(url)
            }
        }
    }

    /// Show an NSAlert for critical errors that require the user's attention.
    private func showCriticalAlert(title: String, message: String) {
        let alert = NSAlert()
        alert.messageText = title
        alert.informativeText = message
        alert.alertStyle = .critical
        alert.addButton(withTitle: "OK")
        NSApp.activate(ignoringOtherApps: true)
        alert.runModal()
    }

    func stop() {
        stopTimer()
        if bridge.stopRecording() {
            isRecording = false
            recordingStartTime = nil
            peakLevels = []
            statusText = "Ready"
            Self.log.info("Recording stopped")
        } else {
            let err = bridge.lastError ?? "Failed to stop recording"
            errorMessage = err
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
            config["audio_channels"] = channels
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

        if !config.isEmpty {
            bridge.setConfig(config)
        }
    }

    // MARK: - Duration Timer

    private func startTimer() {
        timer = Timer.scheduledTimer(withTimeInterval: 1, repeats: true) { [weak self] _ in
            Task { @MainActor in
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
            Task { @MainActor in
                self?.updatePeakLevels()
            }
        }
    }

    private func stopMeterTimer() {
        meterTimer?.invalidate()
        meterTimer = nil
    }

    private func updatePeakLevels() {
        guard isRecording else {
            peakLevels = []
            return
        }
        let peaks = bridge.getPeakLevels()
        peakLevels = peaks.map { Double($0) }
    }

    private func updateDuration() {
        // Check if Rust engine stopped recording unexpectedly (device disconnect, etc.)
        if isRecording && !bridge.isRecording {
            stopTimer()
            isRecording = false
            recordingStartTime = nil
            let msg = bridge.lastError ?? "Recording stopped unexpectedly"
            errorMessage = msg
            statusText = "Error"
            Self.log.error("Recording stopped unexpectedly: \(msg)")
            showCriticalAlert(title: "Recording Stopped", message: msg)
            return
        }

        guard let start = recordingStartTime else { return }
        let elapsed = Int(Date().timeIntervalSince(start))
        let hours = elapsed / 3600
        let minutes = (elapsed % 3600) / 60
        let seconds = elapsed % 60
        if hours > 0 {
            statusText = String(format: "Recording %d:%02d:%02d", hours, minutes, seconds)
        } else {
            statusText = String(format: "Recording %d:%02d", minutes, seconds)
        }

        // Check status from Rust engine
        if let status = bridge.getStatus() {
            if debugLogging {
                Self.log.debug("Status poll: \(String(describing: status))")
            }

            // Audio stream error — device disconnected or driver failure
            if let streamError = status["stream_error"] as? Bool, streamError {
                stop()
                let msg = "Audio device disconnected or encountered an error."
                errorMessage = msg
                statusText = "Error"
                Self.log.error("Stream error detected, stopping recording")
                showCriticalAlert(title: "Recording Stopped", message: msg)
                return
            }
            // Disk space low — stop recording gracefully
            if let diskLow = status["disk_space_low"] as? Bool, diskLow {
                stop()
                let msg = "Disk space is low. Free up space and try again."
                errorMessage = msg
                statusText = "Disk Full"
                Self.log.error("Disk space low, stopping recording")
                showCriticalAlert(title: "Recording Stopped", message: msg)
                return
            }
            // Write errors — cumulative counter from Rust engine
            if let writeErrors = status["write_errors"] as? Int {
                let newDrops = writeErrors - lastReportedWriteErrors

                if writeErrors > 48_000 {
                    // Auto-stop if excessive (>48000 ≈ 1 second at 48kHz)
                    stop()
                    let msg = "Excessive audio data loss (\(writeErrors) samples dropped). Your system may be under heavy load."
                    errorMessage = msg
                    statusText = "Error"
                    Self.log.error("Excessive write errors (\(writeErrors)), stopping recording")
                    showCriticalAlert(title: "Recording Stopped", message: msg)
                    return
                } else if newDrops > 0 {
                    // Only log/display when NEW drops occur (counter is cumulative)
                    lastReportedWriteErrors = writeErrors
                    Self.log.warning("Write errors: \(newDrops) new samples dropped (\(writeErrors) total)")
                    if writeErrors > 500 {
                        errorMessage = "\(writeErrors) audio samples dropped"
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
    func saveOutputDirBookmark(for url: URL) {
        do {
            let bookmarkData = try url.bookmarkData(
                options: .withSecurityScope,
                includingResourceValuesForKeys: nil,
                relativeTo: nil
            )
            UserDefaults.standard.set(bookmarkData, forKey: Self.bookmarkKey)

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
                Self.log.info("Restored output directory: \(url.path)\(isStale ? " (stale, refreshing)" : "")")
            } else {
                Self.log.warning("Failed to access security-scoped resource: \(url.path)")
            }
            if isStale {
                saveOutputDirBookmark(for: url)
            }
        } catch {
            Self.log.error("Failed to restore bookmark: \(error.localizedDescription)")
            UserDefaults.standard.removeObject(forKey: Self.bookmarkKey)
        }
    }

    /// Release security-scoped resource access.
    func releaseOutputDirAccess() {
        securityScopedURL?.stopAccessingSecurityScopedResource()
        securityScopedURL = nil
    }
}
