import BlackBoxFFI
import Foundation
import IOKit.ps

import struct os.Logger

extension RecordingState {
    // MARK: - Duration Timer

    func startTimer() {
        timerTask = Task { [weak self] in
            while !Task.isCancelled {
                try? await Task.sleep(for: .seconds(1))
                guard !Task.isCancelled else { break }
                self?.updateDuration()
            }
        }
    }

    func stopTimer() {
        timerTask?.cancel()
        timerTask = nil
    }

    private func updateDuration() {
        guard let start = recordingStartTime else { return }

        // Menu-flicker fix: previously this method assigned a fresh
        // "Recording M:SS" to `statusText` every second, which is an
        // `@Observable` write that forced the dropdown to re-render and
        // reset the user's hover/keyboard highlight. The elapsed time
        // is now rendered by `Text(recordingStartTime, style: .timer)`
        // in the menu, which auto-ticks internally without writing
        // back to our observable state. Same approach via
        // `nextRotationDate` for the rotation countdown.
        let elapsed = Int(Date().timeIntervalSince(start))

        // DOLL-217: estimated current-file size from elapsed × bytes/sec.
        currentFileSizeText = computeCurrentFileSize(elapsed: elapsed)

        // DOLL-225: check battery every 30 ticks (~30 s) — IOKit calls
        // are cheap but not free, and a long recording shouldn't pay them
        // every second when the state changes at most every few minutes.
        batteryCheckTick += 1
        if batteryCheckTick >= 30 {
            batteryCheckTick = 0
            checkBatteryState()
        }

        // Check status from Rust engine (lightweight C struct, no JSON)
        if let status = bridge.getStatusFlags() {
            applyEngineStatus(status)
        }
    }

    /// Act on one engine status poll. `EngineStatusPolicy.action` picks at
    /// most one terminal condition, in priority order (unexpected stop,
    /// sample-rate change, stream error, write failure, low disk, excessive
    /// drops); otherwise the tick publishes the counters.
    private func applyEngineStatus(_ status: StatusFlags) {
        let action = EngineStatusPolicy.action(isRecording: isRecording, status: status)
        if action == .handleUnexpectedStop {
            handleUnexpectedEngineStop()
            return
        }
        updateGateIdleStatus(status.gate_idle)
        // DOLL-223: publish for UI even below the thresholds that warn or
        // stop, so sub-500-sample drops don't happen invisibly.
        let writeErrors = Int(status.write_errors)
        writeErrorsCount = writeErrors

        switch action {
        case .handleUnexpectedStop:
            return  // handled above, before the gate-idle update

        case .restartForSampleRateChange:
            restartForSampleRateChange()

        case .recoverFromStreamError:
            recoverFromStreamError()

        case .stopForWriteFailure:
            stopForWriteFailure()

        case .stopForLowDiskSpace:
            stopForLowDiskSpace()

        case .stopForExcessiveWriteErrors:
            stopForExcessiveWriteErrors(writeErrors)

        case .keepRecording:
            reportNewWriteErrors(writeErrors)
            // Sample rate — update for file size estimates in settings
            let rate = Int(status.sample_rate)
            if rate > 0, rate != sampleRate {
                sampleRate = rate
                UserDefaults.standard.set(rate, forKey: SettingsKeys.lastSampleRate)
            }
        }
    }

    /// Log and surface dropped samples, only when new drops occurred (the
    /// engine counter is cumulative).
    private func reportNewWriteErrors(_ writeErrors: Int) {
        let newDrops = writeErrors - lastReportedWriteErrors
        guard newDrops > 0 else { return }
        lastReportedWriteErrors = writeErrors
        Self.log.warning("Write errors: \(newDrops) new samples dropped (\(writeErrors) total)")
        if writeErrors > 500 {
            errorMessage = String(localized: "Audio quality degraded \u{2014} some data was lost")
        }
    }

    /// The engine reports it is no longer recording while the UI thinks it
    /// is (device disconnect, etc.): tear the session down and tell the user.
    private func handleUnexpectedEngineStop() {
        stopTimer()
        markRecordingEnded()
        let msg = bridge.lastError ?? String(localized: "Recording stopped unexpectedly")
        setTransientError(msg)
        Self.log.error("Recording stopped unexpectedly: \(msg)")
        notifyUser(title: String(localized: "Recording Stopped"), message: msg)
    }

    /// DOLL-216: surface the silence-gate idle state as "Armed
    /// (waiting for signal)". Updated only on transitions (not
    /// every tick) so the menu doesn't re-render needlessly —
    /// the live elapsed time is now rendered by
    /// Text(date, style: .timer) in the menu directly.
    private func updateGateIdleStatus(_ gateIdle: Bool) {
        if gateIdle != wasGateIdle {
            wasGateIdle = gateIdle
            statusText =
                gateIdle
                ? String(localized: "Armed (waiting for signal)")
                : String(localized: "Recording")
        }
    }

    private func restartForSampleRateChange() {
        Self.log.warning("Sample rate changed on device — finalizing and restarting")
        guard restartIfRecording(reason: "sample rate changed") else {
            // The restart failed (startRecordingInternal already set the
            // error): say the recording stopped, not that it was restarted.
            let msg = String(
                localized: """
                    Your audio device's sample rate changed and recording could not be restarted. \
                    Check the device and start recording again.
                    """
            )
            Self.log.error("Restart after sample-rate change failed")
            notifyUser(title: String(localized: "Recording Stopped"), message: msg)
            return
        }
        notifyUser(
            title: String(localized: "Sample Rate Changed"),
            message: String(
                localized: "Your audio device's sample rate changed. Recording was restarted automatically."
            ),
            identifier: "sample-rate-changed"
        )
    }

    private func stopForWriteFailure() {
        stop()
        let msg = String(
            localized: """
                Recording stopped: unable to write to disk. \
                Free up space or check the output folder's permissions, then try again.
                """
        )
        setTransientError(msg)
        Self.log.error("Write failure — stopping recording")
        notifyUser(title: String(localized: "Recording Stopped"), message: msg)
    }

    private func stopForLowDiskSpace() {
        stop()
        let msg = String(localized: "Your disk is almost full. Free up space and try again.")
        setTransientError(msg)
        Self.log.error("Disk space low, stopping recording")
        notifyUser(title: String(localized: "Recording Stopped"), message: msg)
    }

    /// Auto-stop if excessive (>48000 samples dropped across all channels)
    private func stopForExcessiveWriteErrors(_ writeErrors: Int) {
        stop()
        let msg = String(
            localized: """
                Recording quality degraded \u{2014} your Mac may be under heavy load. \
                Try closing other applications.
                """
        )
        setTransientError(msg)
        Self.log.error("Excessive write errors (\(writeErrors)), stopping recording")
        notifyUser(title: String(localized: "Recording Stopped"), message: msg)
    }

    /// Finalize the current files after an audio-stream error and restart on
    /// the next available device — or stop for good once the DOLL-351
    /// flapping cap is hit. Split out of `updateDuration` so the 1 Hz status
    /// poll stays readable.
    private func recoverFromStreamError() {
        Self.log.error("Stream error detected — finalizing files and attempting restart")
        stopTimer()
        _ = bridge.stopRecording()
        peakLevels = []
        lastReportedWriteErrors = 0
        writeErrorsCount = 0
        isLowBatteryWarning = false
        batteryNotificationFired = false
        batteryCheckTick = 0
        // preflightSizeWarning intentionally preserved: the config
        // hasn't changed on stream-error recovery, so the warning
        // is still valid for the restarted file.

        // DOLL-351: flapping-device guard. Count restarts that happen
        // close together; a restart after a stable run resets the
        // counter. Once the cap is hit, stop for real instead of
        // looping. The 1 Hz status poll naturally spaces attempts ~1s
        // apart, which is the effective backoff.
        let now = Date()
        let restart = EngineStatusPolicy.streamRestart(
            previousCount: streamRestartCount,
            lastRestart: lastStreamRestart,
            now: now
        )
        streamRestartCount = restart.count
        lastStreamRestart = now

        if !restart.isAllowed {
            stopAfterRepeatedStreamErrors()
            return
        }
        restartOnNextAvailableDevice()
    }

    /// The DOLL-351 cap was hit: end the session instead of restarting again.
    private func stopAfterRepeatedStreamErrors() {
        markRecordingEnded()
        streamRestartCount = 0
        lastStreamRestart = nil
        let msg = String(
            localized: """
                Your audio device keeps failing. \
                Recording stopped \u{2014} check the device and try again.
                """
        )
        setTransientError(msg)
        Self.log.error("Stream-error restart cap reached — stopping instead of restarting again")
        notifyUser(title: String(localized: "Recording Stopped"), message: msg)
    }

    private func restartOnNextAvailableDevice() {
        if bridge.startRecording().isSuccess {
            // Restarted successfully (e.g., System Default fell back to built-in mic)
            recordingStartTime = Date()
            statusText = String(localized: "Recording")
            startTimer()
            Self.log.info("Recording restarted on available device")
            notifyUser(
                title: String(localized: "Device Changed"),
                message: String(
                    localized: "Your audio device changed. Recording continued on the next available device."
                ),
                identifier: "device-changed"
            )
        } else {
            // No device available — stop for real
            markRecordingEnded()
            let msg = String(
                localized: """
                    Your audio device was disconnected and no alternative is available. \
                    Check your connections and try again.
                    """
            )
            setTransientError(msg)
            notifyUser(title: String(localized: "Recording Stopped"), message: msg)
        }
    }

    // MARK: - Rotation countdown (DOLL-214)
    // Note: the per-tick string formatter was replaced by the
    // `nextRotationDate` computed property + `Text(_, style: .timer)` in
    // the menu — see the menu-flicker fix. Keeping the section heading
    // so future grep / DOLL-214 archaeology lands on the right spot.

    // MARK: - Current file size estimate (DOLL-217)

    /// Estimate the current WAV file's size from elapsed-in-cycle ×
    /// bytes-per-second. Uses last-seen sample rate (falls back to
    /// 48 kHz when the engine hasn't reported one yet) and the snapshot
    /// of bit depth + channel count + output mode captured at start.
    /// Returns nil for misconfigured states so the menu hides the line.
    /// DOLL-233: reads from the cached snapshot, not UserDefaults.
    private func computeCurrentFileSize(elapsed: Int) -> String? {
        guard let snapshot = configSnapshot, snapshot.channelCount > 0 else { return nil }

        let bytesPerSample = snapshot.bitDepth / 8
        let channelsPerFile = snapshot.outputMode == "split" ? 1 : snapshot.channelCount

        let estSampleRate = sampleRate > 0 ? sampleRate : 48_000
        let bytesPerSecond = estSampleRate * bytesPerSample * channelsPerFile

        // Continuous mode rotates every cadence seconds, so "current
        // file" is the bytes accumulated since the most recent boundary.
        // Single mode has no rotation — the file grows from start.
        let elapsedInFile: Int
        if snapshot.continuousMode, snapshot.recordingCadence > 0 {
            elapsedInFile = elapsed % snapshot.recordingCadence
        } else {
            elapsedInFile = elapsed
        }

        let bytes = Int64(bytesPerSecond) * Int64(elapsedInFile)
        return Self.formatFileSize(bytes)
    }

    /// Human-readable bytes with an "~" estimate hint. DOLL-377: use the
    /// locale-aware binary byte-count format style instead of a hardcoded
    /// "%.1f GB" with a "." separator, so a de_DE/fr_FR user sees "1,5 GB".
    private static func formatFileSize(_ bytes: Int64) -> String {
        "~" + bytes.formatted(.byteCount(style: .binary))
    }

    // MARK: - Battery Monitoring (DOLL-225)

    /// Threshold below which we warn that the current recording is at
    /// risk of being cut off by a system shutdown. 20 % matches macOS's
    /// own "battery low" alert level.
    private static let lowBatteryThreshold = 20

    /// Poll IOKit for the current internal battery state and flip
    /// `isLowBatteryWarning` if we're discharging below the threshold.
    /// On Macs with no internal battery (Mac mini, Studio, Pro) there's
    /// nothing to warn about — we just leave the flag false.
    private func checkBatteryState() {
        guard let state = currentBatteryState() else {
            // No internal battery, or IOKit query failed — clear any
            // stale warning rather than leaving it pinned on.
            if isLowBatteryWarning {
                isLowBatteryWarning = false
            }
            batteryNotificationFired = false
            return
        }

        let shouldWarn = !state.onACPower && state.percent <= Self.lowBatteryThreshold
        if shouldWarn {
            isLowBatteryWarning = true
            if !batteryNotificationFired {
                batteryNotificationFired = true
                notifyUser(
                    title: String(localized: "Battery Low"),
                    message: String(
                        localized:
                            "BlackBox is recording on battery (\(state.percent)%). Plug in soon to avoid an unexpected stop."
                    ),
                    identifier: "battery-low"
                )
                Self.log.warning("Battery low while recording: \(state.percent)% on battery")
            }
        } else {
            // Plugged back in or charge recovered — clear the warning so
            // the user knows they're safe again. Allow a fresh notification
            // if the cycle repeats.
            if isLowBatteryWarning {
                isLowBatteryWarning = false
                batteryNotificationFired = false
            }
        }
    }

    /// Read the current internal-battery percent and AC-power flag, or
    /// `nil` on desktops / when IOKit returns nothing usable.
    private func currentBatteryState() -> (percent: Int, onACPower: Bool)? {
        guard let infoRef = IOPSCopyPowerSourcesInfo()?.takeRetainedValue() else {
            return nil
        }
        guard let sourcesRef = IOPSCopyPowerSourcesList(infoRef)?.takeRetainedValue() else {
            return nil
        }
        let sources = sourcesRef as [CFTypeRef]
        for source in sources {
            guard
                let desc = IOPSGetPowerSourceDescription(infoRef, source)?
                    .takeUnretainedValue() as? [String: Any]
            else {
                continue
            }
            // Skip non-internal sources (e.g. UPS) — we only care about
            // the laptop's own battery for "you're going to lose power."
            guard (desc[kIOPSTypeKey] as? String) == kIOPSInternalBatteryType else {
                continue
            }
            let percent = desc[kIOPSCurrentCapacityKey] as? Int ?? 0
            let powerState = desc[kIOPSPowerSourceStateKey] as? String
            let onAC = powerState == kIOPSACPowerValue
            return (percent, onAC)
        }
        return nil
    }
}
