import Foundation
import AppKit
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

    init() {
        bridge = RustBridge()
        refreshDevices()
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
    }
}
