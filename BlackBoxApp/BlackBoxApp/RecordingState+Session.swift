import AppKit
import Foundation

import struct os.Logger

extension RecordingState {
    // MARK: - Actions

    func toggle() {
        if isRecording {
            stop()
        } else {
            start()
        }
    }

    /// Fire-and-forget start for synchronous callers (menu button, hotkey,
    /// notification action) whose UI already observes `isRecording` /
    /// `errorMessage` for the outcome. The spawned Task ends naturally;
    /// app termination cancels in-flight Tasks via structured-concurrency
    /// cooperation, so no explicit Task.cancel is required from
    /// applicationShouldTerminate.
    func start() {
        // A deliberate start supersedes a pending resume-on-wake (DOLL-182).
        cancelPendingResume()
        Task { @MainActor in
            await self.startAndWait()
        }
    }

    /// Start recording and report whether a recording is active once the
    /// attempt resolves. DOLL-443: `start()` returns before its internal
    /// permission await does, so callers that branched on `isRecording`
    /// immediately afterwards (auto-record notification, wake / session
    /// resume) always read stale `false` — wake-resume posted "Resume
    /// Failed" even when the resume succeeded a beat later. Those callers
    /// must await this instead.
    @discardableResult
    func startAndWait() async -> Bool {
        // Debounce: rapid double-start (e.g. hotkey held, accessibility
        // automation) would otherwise launch two requestAccess flows in
        // parallel. isRecording stays false across the permission await —
        // for as long as the user leaves the permission dialog open — so
        // the isStartingRecording in-flight flag is what blocks re-entry
        // across that window (DOLL-459); the isRecording guard alone only
        // covered re-entry from the same MainActor turn.
        guard !isRecording, !isStartingRecording else { return isRecording }
        isStartingRecording = true
        errorMessage = nil
        defer { isStartingRecording = false }
        if await checkMicrophonePermission() {
            startRecordingInternal()
        } else {
            errorMessage = String(localized: "Microphone access denied. Open System Settings to allow access.")
            statusText = String(localized: "Error")
        }
        return isRecording
    }

    private func startRecordingInternal() {
        // DOLL-459: defense in depth — re-check after the permission await.
        // The guard in start() ran before the suspension; if a session began
        // through another path while the dialog was up, a second
        // bridge.startRecording() would fail and its error branch would mark
        // the LIVE recording as idle (unstoppable from the UI).
        guard !isRecording else { return }

        // DOLL-464: for first-launch users (onboarding incomplete at init,
        // so the eager request was skipped) this is the in-context moment
        // to ask — a recording is starting, so stop/pause notifications now
        // matter. No-op on every other launch (auth requested at init,
        // DOLL-134) and after the first call.
        requestNotificationAuthIfNeeded()

        // DOLL-233: snapshot the config once at start so the per-tick
        // computations downstream (rotation countdown, file-size estimate,
        // preflight warning) read from in-memory fields instead of
        // hitting UserDefaults 5-7 times per second.
        configSnapshot = captureConfigSnapshot()

        // DOLL-220: warn before we kick off the engine if the math says
        // the per-file size will blow past the 4 GiB WAV-header cap. The
        // engine still proceeds — the file just gets clamped — but the
        // user gets notification and menu signal so they can adjust.
        evaluatePreflightFileSizeWarning()

        // Stop monitoring first — recording will take over the audio stream
        if isMonitoring {
            stopMonitoring()
        }

        let result = bridge.startRecording()
        if result.isSuccess {
            isRecording = true
            recordingStartTime = Date()
            // "Recording" (no trailing ellipsis or M:SS) is now a stable
            // string — the live elapsed time is rendered separately via
            // Text(_, style: .timer) so this value only changes on a
            // gate-idle transition. Keeps the menu from re-rendering
            // every second and resetting hover state.
            statusText = String(localized: "Recording")
            wasGateIdle = false
            lastReportedWriteErrors = 0
            writeErrorsCount = 0
            isLowBatteryWarning = false
            batteryNotificationFired = false
            batteryCheckTick = 0
            // DOLL-213: clear any stale post-Stop summary when a new
            // recording begins; the just-started session is the new
            // "current," and the old summary is no longer relevant.
            lastRecordingDurationText = nil
            // preflightSizeWarning is intentionally NOT cleared here —
            // evaluatePreflightFileSizeWarning() runs just before
            // bridge.startRecording() and already populates it (or nils
            // it) for the current session. Clearing it here would wipe
            // the warning the moment the engine acknowledged the start.
            startTimer()
            beginPreventingSleep()
            refreshMeterChannelNumbers()
            Self.log.info("Recording started")
            NSAccessibility.post(
                element: NSApp as Any,
                notification: .announcementRequested,
                userInfo: [.announcement: String(localized: "Recording started")]
            )
        } else {
            // DOLL-448: release sleep prevention if this start was a
            // restart of a live session (restartIfRecording) — the token
            // from the original beginPreventingSleep would otherwise leak.
            // No-op on a fresh start (no token yet).
            endPreventingSleep()
            isRecording = false
            recordingStartTime = nil
            let detail = bridge.lastError
            let err: String
            switch result {
            case .audioDevice:
                err = String(localized: "No audio input device found. Check System Settings \u{203A} Sound.")

            case .config:
                let reason = detail ?? String(localized: "invalid settings")
                err = String(localized: "Configuration error: \(reason)")

            case .io:
                err = String(localized: "Recording failed: disk error")

            default:
                err = detail ?? String(localized: "Failed to start recording")
            }
            setTransientError(err)
            Self.log.error("Failed to start recording (code \(result.rawValue)): \(err)")
        }
    }

    func stop(reason: SleepWakePolicy.StopReason = .user) {
        let sessionDuration = recordingStartTime.map { Date().timeIntervalSince($0) } ?? 0
        stopTimer()
        let result = bridge.stopRecording()
        // The FFI takes the recorder out of the handle before finalizing, so an
        // error from stopRecording usually means the engine stopped anyway and
        // only the finalize failed. Ask the engine instead of assuming it is
        // still running: treating every error as "still recording" left the
        // UI on "Recording" with no status poll, and resume-on-wake then
        // restarted it (for example onto a full disk).
        let outcome = SessionPolicy.stopOutcome(
            stopSucceeded: result.isSuccess,
            engineStillRecording: !result.isSuccess && bridge.isRecording
        )
        let failure = result.isSuccess ? nil : bridge.lastError ?? String(localized: "Failed to stop recording")
        if let failure {
            Self.log.error("Failed to stop recording (code \(result.rawValue)): \(failure)")
        }
        switch outcome {
        case .stillRecording:
            // Keep the session's timer (and with it the status poll) and its
            // sleep prevention: the engine is still writing.
            startTimer()
            setTransientError(failure ?? String(localized: "Failed to stop recording"))

        case .stopped, .stoppedWithError:
            endPreventingSleep()
            finishStoppedSession(reason: reason, sessionDuration: sessionDuration, succeeded: outcome == .stopped)
            if let failure { setTransientError(failure) }
        }
    }

    /// UI and bookkeeping teardown once the engine has stopped, whether or
    /// not its final flush succeeded.
    private func finishStoppedSession(
        reason: SleepWakePolicy.StopReason,
        sessionDuration: TimeInterval,
        succeeded: Bool
    ) {
        isRecording = false
        recordingStartTime = nil
        peakLevels = []
        errorMessage = nil
        statusText = String(localized: "Ready")
        // DOLL-351: a clean stop clears any flapping-restart bookkeeping.
        streamRestartCount = 0
        lastStreamRestart = nil
        // DOLL-213: surface a transient "last recording" summary for
        // 30 s so the user gets confirmation of what just finished.
        // Captured here (before the durations resets to 0) and
        // displayed as a menu block with a Show in Finder button.
        showLastRecordingSummary(sessionDuration: sessionDuration)
        writeErrorsCount = 0
        isLowBatteryWarning = false
        batteryNotificationFired = false
        batteryCheckTick = 0
        preflightSizeWarning = nil
        currentFileSizeText = nil
        configSnapshot = nil
        wasGateIdle = false
        // DOLL-182: a user stop cancels any pending resume-on-wake.
        // Without this, a manual stop within the 1.5s deferred-resume
        // window after sleep/wake or session resign/activate would let
        // the deferred start() resurrect a recording the user
        // explicitly stopped. The sleep-interruption stop is exempt —
        // it just SET the flag, and clearing it here made
        // resume-on-wake dead code (DOLL-442).
        if SleepWakePolicy.stopCancelsPendingResume(reason) {
            wasSleepInterrupted = false
            // The flag is already consumed once the wake handler has run;
            // the resume it scheduled must be cancelled too.
            cancelPendingResume()
        }
        Self.log.info("Recording stopped")
        NSAccessibility.post(
            element: NSApp as Any,
            notification: .announcementRequested,
            userInfo: [.announcement: String(localized: "Recording stopped")]
        )

        // Track successful sessions >5 min for App Store review prompt
        if succeeded, sessionDuration > 300 {
            let key = SettingsKeys.successfulRecordingSessions
            UserDefaults.standard.set(UserDefaults.standard.integer(forKey: key) + 1, forKey: key)
        }

        // Resume monitoring if the meter window is still open
        if isMeterWindowOpen {
            startMonitoring()
        }
    }

    /// Finalize current WAV files and immediately start a new recording session
    /// with the updated config. No-op if not currently recording.
    ///
    /// `whileStopped` runs after the engine has finalized its files and
    /// before the new session starts — the one window in which it is safe to
    /// release something the old session was using (the output folder's
    /// security scope).
    ///
    /// Returns whether a recording is running afterwards: `false` when there
    /// was none to restart or the new session failed to start (the failure
    /// is already surfaced through `setTransientError`).
    @discardableResult
    func restartIfRecording(reason: String, whileStopped: (() -> Void)? = nil) -> Bool {
        guard isRecording else { return false }
        Self.log.info("Config changed while recording (\(reason)) — finalizing and restarting")
        stopTimer()
        _ = bridge.stopRecording()
        whileStopped?()
        // The engine is stopped; reflect it before startRecordingInternal,
        // whose double-start guard (DOLL-459) would otherwise see the stale
        // true and return without restarting — leaving the engine stopped
        // while the UI showed "Recording" until the next status poll
        // flagged it as an unexpected stop. Sleep prevention is left in
        // place: the session continues if the restart succeeds, and a
        // failed restart releases it in startRecordingInternal's error
        // branch (DOLL-448).
        isRecording = false
        recordingStartTime = nil
        peakLevels = []
        lastReportedWriteErrors = 0
        writeErrorsCount = 0
        // Battery state survives restart since the underlying hardware
        // state is unchanged. Reset notification so a future cross of
        // the threshold can fire fresh.
        batteryCheckTick = 0
        startRecordingInternal()
        return isRecording
    }

    /// Engine-side session teardown (DOLL-448): the engine has already
    /// stopped (or refused to restart), so `stop()` — which calls
    /// `bridge.stopRecording()` again — is not appropriate. These paths
    /// used to flip `isRecording = false` directly and leaked the
    /// `beginPreventingSleep` activity token: after a device disconnect
    /// or unexpected engine stop, the Mac could never idle-sleep again
    /// until the app quit or a later recording was stopped manually.
    func markRecordingEnded() {
        endPreventingSleep()
        isRecording = false
        recordingStartTime = nil
        // Mirror stop()'s teardown of per-session UI state — without this,
        // engine-initiated stops left stale meters, a frozen current-file
        // size, and a dead config snapshot on screen (DOLL-448).
        peakLevels = []
        currentFileSizeText = nil
        configSnapshot = nil
        // And, like stop(), hand the audio stream back to the level meter
        // if its window is still open.
        if isMeterWindowOpen {
            startMonitoring()
        }
    }

    // MARK: - Sleep prevention

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

    // MARK: - Devices

    func refreshDevices() {
        availableDevices = RustBridge.listInputDevices()
        systemDefaultDeviceName = RustBridge.defaultInputDeviceName()
    }

    /// CoreAudio posts several notifications for one plug or unplug (the
    /// device list, the default input); refresh once they settle.
    func scheduleDeviceRefresh() {
        deviceRefreshTask?.cancel()
        deviceRefreshTask = Task { [weak self] in
            try? await Task.sleep(for: .milliseconds(300))
            guard !Task.isCancelled, let self else { return }
            deviceRefreshTask = nil
            refreshDevices()
        }
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

    // MARK: - Config snapshot (DOLL-233)

    /// Read the live UserDefaults values once and freeze them for the
    /// duration of the recording. The per-tick callbacks
    /// (`computeRotationCountdown`, `computeCurrentFileSize`,
    /// `evaluatePreflightFileSizeWarning`) read this snapshot rather
    /// than hitting UserDefaults 5-7 times every second.
    private func captureConfigSnapshot() -> RecordingConfigSnapshot {
        let defaults = UserDefaults.standard
        let bitDepthValue = defaults.integer(forKey: SettingsKeys.bitDepth)
        return RecordingConfigSnapshot(
            continuousMode: defaults.object(forKey: SettingsKeys.continuousMode) as? Bool ?? false,
            recordingCadence: defaults.integer(forKey: SettingsKeys.recordingCadence),
            channelCount: countChannels(defaults.string(forKey: SettingsKeys.audioChannels) ?? "1"),
            bitDepth: bitDepthValue > 0 ? bitDepthValue : 24,
            outputMode: defaults.string(forKey: SettingsKeys.outputMode) ?? "split"
        )
    }

    // MARK: - Last-recording summary (DOLL-213)

    /// Format a TimeInterval as "M:SS" or "H:MM:SS" to match the live
    /// "Recording 12:34" status format the user just saw counting up.
    private static func formatRecordedDuration(_ seconds: TimeInterval) -> String {
        let total = Int(seconds)
        let hours = total / 3600
        let minutes = (total % 3600) / 60
        let secs = total % 60
        return hours > 0
            ? String(format: "%d:%02d:%02d", hours, minutes, secs)
            : String(format: "%d:%02d", minutes, secs)
    }

    /// Show the post-Stop summary for a session that ran, then clear it after
    /// 30 s unless a newer summary or a new recording replaced it.
    private func showLastRecordingSummary(sessionDuration: TimeInterval) {
        guard sessionDuration > 0 else { return }
        lastRecordingDurationText = Self.formatRecordedDuration(sessionDuration)
        let snapshot = lastRecordingDurationText
        // DOLL-230: explicit @MainActor on the Task closure even
        // though RecordingState is class-level @MainActor — under
        // strict-concurrency the inherited isolation rules are
        // subtle and this makes the mutation-after-await safe by
        // construction regardless of how the surrounding class
        // is later refactored.
        Task { @MainActor [weak self] in
            try? await Task.sleep(for: .seconds(30))
            guard let self,
                lastRecordingDurationText == snapshot,
                !isRecording
            else { return }
            lastRecordingDurationText = nil
        }
    }

    /// Dismiss the post-Stop summary block early — called when the user
    /// clicks the Show in Finder action in the summary (they've now
    /// taken action on it, no need to keep showing it).
    func dismissLastRecordingSummary() {
        lastRecordingDurationText = nil
    }

    // MARK: - Pre-flight 4 GiB warning (DOLL-220)

    /// WAV header `data` chunk is `u32`, so a single file maxes out at
    /// 4 GiB - 1. DOLL-204 catches this on finalize and logs / clamps;
    /// DOLL-220 catches it before we burn through hours of recording.
    private static let wavMaxFileBytes = Int64(UInt32.max)

    /// Inspect the current configuration and set `preflightSizeWarning`
    /// (plus a notification + log line) when the projected per-file
    /// bytes-per-rotation exceeds the 4 GiB WAV cap. Only meaningful for
    /// continuous mode — single mode has no rotation interval to bound
    /// the file with, so we leave it alone there.
    /// DOLL-233: reads from the cached snapshot, populated immediately
    /// before this method runs in startRecordingInternal.
    private func evaluatePreflightFileSizeWarning() {
        preflightSizeWarning = nil

        guard let snapshot = configSnapshot, snapshot.continuousMode else { return }
        let cadence = snapshot.recordingCadence
        guard cadence > 0 else { return }
        let channels = snapshot.channelCount
        guard channels > 0 else { return }

        let bytesPerSample = snapshot.bitDepth / 8
        // In split mode each file holds one channel; in single mode all
        // channels share a file. The cap applies per-file, so we project
        // for the most populated file we'll create.
        let channelsPerFile = snapshot.outputMode == "split" ? 1 : channels

        // No reliable sample-rate signal until cpal connects, so fall
        // back to 48 kHz when we haven't seen a session yet. This biases
        // the warning toward false negatives — we'd rather under-warn
        // than spook users about hypothetical hi-res setups they don't
        // actually have.
        let estSampleRate = sampleRate > 0 ? sampleRate : 48_000

        let bytesPerFile =
            Int64(channelsPerFile)
            * Int64(bytesPerSample)
            * Int64(estSampleRate)
            * Int64(cadence)

        guard bytesPerFile > Self.wavMaxFileBytes else { return }

        let gigabytes = Double(bytesPerFile) / 1_073_741_824.0
        // DOLL-439/#40: localizable + locale-aware decimal (the GB value is
        // pre-formatted so the String Catalog key carries a %@, not a "."-only %.1f).
        let rateNote = sampleRate > 0 ? "" : String(localized: " (estimated at 48 kHz)")
        let gbText = gigabytes.formatted(.number.precision(.fractionLength(1)))
        let msg = String(
            localized:
                "Each rotation will produce roughly \(gbText) GB\(rateNote). WAV files are capped at 4 GB — players may fail to import or truncate. Reduce the rotation interval, sample rate, channels, or bit depth."
        )
        preflightSizeWarning = msg
        Self.log.warning("Pre-flight 4 GiB cap warning: \(msg)")
        notifyUser(
            title: String(localized: "Large file warning"),
            message: msg,
            identifier: "preflight-4gb-warning"
        )
    }
}
