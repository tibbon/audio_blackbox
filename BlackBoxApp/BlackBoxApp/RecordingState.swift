import Foundation
import AppKit
import AVFoundation
import Combine

/// Observable state for the menu bar UI, wrapping the Rust audio engine via FFI.
@MainActor
final class RecordingState: ObservableObject {
    @Published var isRecording = false
    @Published var statusText = "Ready"
    @Published var errorMessage: String?
    @Published var availableDevices: [String] = []

    let bridge: RustBridge
    private var recordingStartTime: Date?
    private var timer: Timer?
    private var securityScopedURL: URL?

    private static let bookmarkKey = "outputDirBookmark"

    init() {
        bridge = RustBridge()
        refreshDevices()
        restoreOutputDirBookmark()

        // Auto-record on launch if enabled
        if UserDefaults.standard.bool(forKey: "autoRecord") {
            // Delay slightly to let the app finish launching
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
            startTimer()
        } else {
            errorMessage = bridge.lastError ?? "Failed to start recording"
            statusText = "Error"
        }
    }

    // MARK: - Microphone Permission

    private func checkMicrophonePermission(completion: @escaping (Bool) -> Void) {
        switch AVCaptureDevice.authorizationStatus(for: .audio) {
        case .authorized:
            completion(true)
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

    func stop() {
        stopTimer()
        if bridge.stopRecording() {
            isRecording = false
            recordingStartTime = nil
            statusText = "Ready"
        } else {
            errorMessage = bridge.lastError ?? "Failed to stop recording"
        }
    }

    func openOutputDir() {
        let config = bridge.getConfig()
        let dir = config?["output_dir"] as? String ?? "recordings"

        let url: URL
        if dir.hasPrefix("/") {
            url = URL(fileURLWithPath: dir)
        } else {
            // Relative path — resolve from current working directory
            let cwd = FileManager.default.currentDirectoryPath
            url = URL(fileURLWithPath: cwd).appendingPathComponent(dir)
        }

        // Create directory if it doesn't exist
        try? FileManager.default.createDirectory(at: url, withIntermediateDirectories: true)
        NSWorkspace.shared.open(url)
    }

    func refreshDevices() {
        availableDevices = RustBridge.listInputDevices()
    }

    func selectDevice(_ name: String) {
        bridge.setConfig(["input_device": name])
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

    private func updateDuration() {
        // Check if Rust engine stopped recording unexpectedly (device disconnect, etc.)
        if isRecording && !bridge.isRecording {
            stopTimer()
            isRecording = false
            recordingStartTime = nil
            errorMessage = bridge.lastError ?? "Recording stopped unexpectedly"
            statusText = "Error"
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

        // Check for write errors from Rust engine
        if let status = bridge.getStatus(),
           let writeErrors = status["write_errors"] as? Int,
           writeErrors > 0 {
            errorMessage = "\(writeErrors) audio samples dropped (buffer overflow or write error)"
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
        } catch {
            errorMessage = "Failed to save directory bookmark: \(error.localizedDescription)"
        }
    }

    /// Restore the security-scoped bookmark on launch.
    private func restoreOutputDirBookmark() {
        guard let data = UserDefaults.standard.data(forKey: Self.bookmarkKey) else { return }
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
            }
            if isStale {
                // Re-save the bookmark to refresh it
                saveOutputDirBookmark(for: url)
            }
        } catch {
            // Bookmark invalid — user will need to re-select directory
            UserDefaults.standard.removeObject(forKey: Self.bookmarkKey)
        }
    }

    /// Release security-scoped resource access.
    func releaseOutputDirAccess() {
        securityScopedURL?.stopAccessingSecurityScopedResource()
        securityScopedURL = nil
    }
}
