import AppKit
import Foundation

import class AVFoundation.AVCaptureDevice
import struct os.Logger

extension RecordingState {
    // MARK: - Monitoring

    func startMonitoring() {
        Task { @MainActor in
            guard await self.checkMicrophonePermission() else { return }
            // DOLL-459: the permission await can suspend across user
            // interaction; re-check the record/monitor mutual exclusion
            // afterwards so a stale monitor task doesn't grab the audio
            // stream out from under an active (or in-flight) recording. The
            // meter window may also have closed meanwhile; starting then
            // left a monitoring stream (and the mic indicator) running with
            // no window to stop it.
            guard
                SessionPolicy.shouldStartMonitoring(
                    meterWindowOpen: self.isMeterWindowOpen,
                    isRecording: self.isRecording,
                    isStartingRecording: self.isStartingRecording,
                    isMonitoring: self.isMonitoring
                )
            else { return }
            let result = self.bridge.startMonitoring()
            if result.isSuccess {
                self.isMonitoring = true
                self.refreshMeterChannelNumbers()
                Self.log.info("Audio monitoring started")
            } else {
                Self.log.error(
                    "Failed to start monitoring (code \(result.rawValue)): \(self.bridge.lastError ?? "unknown")"
                )
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

    /// Work out which device channels the session's peak levels belong to,
    /// so the meter can label bars "Ch 3" and "Ch 4" for a 3,4 selection
    /// instead of "Ch 1" and "Ch 2". The engine falls back to the system
    /// default when the chosen device is missing, and so does this.
    func refreshMeterChannelNumbers() {
        let defaults = UserDefaults.standard
        let device = defaults.string(forKey: SettingsKeys.inputDevice) ?? ""
        var count = (try? RustBridge.getDeviceChannelCount(deviceName: device).get()) ?? 0
        if count == 0, !device.isEmpty {
            count = (try? RustBridge.getDeviceChannelCount(deviceName: "").get()) ?? 0
        }
        meterChannelNumbers = recordedChannelNumbers(
            spec: defaults.string(forKey: SettingsKeys.audioChannels) ?? "1",
            deviceChannelCount: count
        )
    }

    // MARK: - Microphone Permission

    func checkMicrophonePermission() async -> Bool {
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
        // DOLL-438: AppKit takes plain String (not LocalizedStringKey), so these
        // are wrapped in String(localized:) to enter the String Catalog.
        alert.messageText = String(localized: "Microphone Access Required")
        alert.informativeText = String(
            localized: """
                BlackBox needs microphone access to record audio. \
                You can allow access in System Settings > Privacy & Security > Microphone.
                """
        )
        alert.alertStyle = .warning
        alert.addButton(withTitle: String(localized: "Open System Settings"))
        alert.addButton(withTitle: String(localized: "Cancel"))

        NSApp.activate(ignoringOtherApps: true)
        if alert.runModal() == .alertFirstButtonReturn {
            if let url = URL(string: "x-apple.systempreferences:com.apple.preference.security?Privacy_Microphone") {
                NSWorkspace.shared.open(url)
            }
        }
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

    /// Run the 30 Hz meter poll only while the window is open AND visible AND
    /// there's actually a signal source (recording or monitoring). DOLL-374:
    /// without the activity check, opening the meter while monitoring fails to
    /// start (no device / denied) left the timer waking the CPU 30x/s forever
    /// even though updatePeakLevels early-returns every tick. The isRecording /
    /// isMonitoring didSet hooks re-run this when a source comes up or goes away.
    func syncMeterTimer() {
        if isMeterWindowOpen, !isMeterWindowOccluded, isRecording || isMonitoring {
            startMeterTimer()
        } else {
            stopMeterTimer()
        }
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
        case .success(let channelCount):
            count = channelCount

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
            for i in 0..<count where abs(peakBuffer[i] - peakLevels[i]) > 0.001 {
                changed = true
                break
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
}
