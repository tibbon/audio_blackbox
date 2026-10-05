import AppKit
import Foundation

import struct os.Logger

extension RecordingState {
    // MARK: - Actions

    /// The menu button and the hotkey. Stop pressed while a start (or a
    /// restart) is still in flight ends the session as soon as the engine
    /// returns from the start; it used to reach here as a second start,
    /// which the in-flight guard dropped (DOLL-659). Pressed while a stop
    /// is running, it does nothing.
    func toggle() {
        switch sessionPhase {
        case .idle:
            start()

        case .starting, .recording:
            stop()

        case .stopping:
            break
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
        // Taken now so a stop pressed while this start waits below (on the
        // restore, the permission dialog or the engine) ends it.
        let generation = sessionGeneration
        // A start from the menu or hotkey right at launch must not beat the
        // restore Task: it would record into the default folder, and crash
        // recovery would finalize this session's live .recording.wav.
        await bookmarkRestoreTask?.value
        // A start cancelled while it waited (app termination) doesn't go on
        // to the permission prompt and the engine.
        guard !Task.isCancelled else { return isRecording }
        if await checkMicrophonePermission() {
            await startRecordingInternal(generation: generation)
        } else {
            errorMessage = String(localized: "Microphone access denied. Open System Settings to allow access.")
            statusText = String(localized: "Error")
        }
        return isRecording
    }

    /// `isRestart`: this continues a live session (restartIfRecording)
    /// rather than starting a new one. `generation`: the
    /// `sessionGeneration` the caller started from; a stop since then
    /// ended this session, and the start backs out.
    private func startRecordingInternal(generation: Int, isRestart: Bool = false) async {
        // DOLL-459: defense in depth — re-check after the permission await.
        // The guard in start() ran before the suspension; if a session began
        // through another path while the dialog was up, a second
        // bridge.startRecording() would fail and its error branch would mark
        // the LIVE recording as idle (unstoppable from the UI).
        guard !isRecording, generation == sessionGeneration else { return }

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

        // Stop monitoring first — recording will take over the audio stream
        if isMonitoring {
            stopMonitoring()
        }

        // nil: a stop ended this session while the engine was starting, and
        // that stop has already torn the session down.
        guard let result = await startEngine(generation: generation) else { return }
        if result.isSuccess {
            beginStartedSession(isRestart: isRestart)
        } else {
            reportFailedStart(result)
        }
    }

    /// The engine is recording: set the session up around it.
    private func beginStartedSession(isRestart: Bool) {
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
        // DOLL-220: warn if the math says a file will pass the 4 GiB
        // WAV-header cap. The engine proceeds, splitting the file at
        // 4 GB. This runs only once the engine is running: it used to
        // run before bridge.startRecording() and could show a modal
        // alert, which left the engine stopped until the user clicked
        // OK, with isRecording and isStartingRecording both false so a
        // second start could race in.
        //
        // The estimate needs the rate this stream opened at. The engine
        // publishes it during startRecording(); the status poll only
        // picks it up a second later, so after a sample-rate-change
        // restart the estimate used the old device rate.
        adoptEngineSampleRate()
        evaluatePreflightFileSizeWarning(isRestart: isRestart)
        startTimer()
        beginPreventingSleep()
        refreshMeterChannelNumbers()
        Self.log.info("Recording started")
        NSAccessibility.post(
            element: NSApp as Any,
            notification: .announcementRequested,
            userInfo: [.announcement: String(localized: "Recording started")]
        )
    }

    /// The engine refused to start: clear the session and say why.
    private func reportFailedStart(_ result: BlackBoxError) {
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

    /// Fire-and-forget stop for the menu, the hotkey and the other
    /// synchronous callers. It returns at once; the engine finalizes the
    /// files off the main actor while the menu shows "Stopping…"
    /// (DOLL-659). Paths that must not return before the files are
    /// finalized use `stopSynchronously`; async ones `stopAndWait`.
    func stop(reason: SleepWakePolicy.StopReason = .user) {
        guard let pending = beginStop(reason: reason) else { return }
        Task { await self.completeStop(pending) }
    }

    /// Stop and return once the engine has finalized the files.
    func stopAndWait(reason: SleepWakePolicy.StopReason = .user) async {
        guard let pending = beginStop(reason: reason) else { return }
        await completeStop(pending)
    }

    /// Stop on the main thread, returning only once the engine has
    /// finalized the files: for sleep, power off and session switching,
    /// where the system can suspend the app as soon as the handler returns.
    /// A start in flight in the engine holds the engine's lock, so this
    /// waits for it and stops what it started; a start that reaches the
    /// engine after this stops itself when it returns (the generation
    /// check in `startEngine`). A stop already running off the main actor
    /// is superseded: this one does the teardown.
    func stopSynchronously(reason: SleepWakePolicy.StopReason = .user) {
        cancelResumeIfUserStop(reason)
        let sessionDuration = recordingStartTime.map { Date().timeIntervalSince($0) } ?? 0
        stopTimer()
        _ = endSessionGeneration()
        applyStopResult(engineCalls.stop(), sessionDuration: sessionDuration)
    }

    /// A stop that has ended the session's generation and is waiting for
    /// the engine.
    struct PendingStop {
        let generation: Int
        let sessionDuration: TimeInterval
    }

    /// The synchronous half of an async stop: record the intent at once, so
    /// a second stop and a pending resume both see it, and end the
    /// session's generation so a start in flight backs out. `nil` when a
    /// stop is already running.
    private func beginStop(reason: SleepWakePolicy.StopReason) -> PendingStop? {
        cancelResumeIfUserStop(reason)
        guard !isStoppingRecording else { return nil }
        isStoppingRecording = true
        stopTimer()
        return PendingStop(
            generation: endSessionGeneration(),
            sessionDuration: recordingStartTime.map { Date().timeIntervalSince($0) } ?? 0
        )
    }

    private func completeStop(_ pending: PendingStop) async {
        let result = await runEngine(engineCalls.stop)
        isStoppingRecording = false
        // A synchronous stop (sleep) ran meanwhile and did the teardown.
        guard pending.generation == sessionGeneration else { return }
        applyStopResult(result, sessionDuration: pending.sessionDuration)
    }

    /// DOLL-182: a user stop cancels any pending resume-on-wake. Without
    /// this, a manual stop within the 1.5s deferred-resume window after
    /// sleep/wake or session resign/activate would let the deferred start()
    /// resurrect a recording the user explicitly stopped. The
    /// sleep-interruption stop is exempt — its caller just SET the flag,
    /// and clearing it made resume-on-wake dead code (DOLL-442). Done when
    /// the stop is asked for, not when the engine returns from it.
    private func cancelResumeIfUserStop(_ reason: SleepWakePolicy.StopReason) {
        guard SleepWakePolicy.stopCancelsPendingResume(reason) else { return }
        wasSleepInterrupted = false
        // The flag is already consumed once the wake handler has run;
        // the resume it scheduled must be cancelled too.
        cancelPendingResume()
    }

    private func applyStopResult(_ result: BlackBoxError, sessionDuration: TimeInterval) {
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
            finishStoppedSession(sessionDuration: sessionDuration, succeeded: outcome == .stopped)
            if let failure { setTransientError(failure) }
        }
    }

    /// UI and bookkeeping teardown once the engine has stopped, whether or
    /// not its final flush succeeded.
    private func finishStoppedSession(sessionDuration: TimeInterval, succeeded: Bool) {
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
    /// Returns whether a new session is running afterwards: `false` when
    /// there was none to restart (or one was starting or stopping), the new
    /// session failed to start, or the old one could not be stopped (each
    /// failure is already surfaced through `setTransientError`). In that
    /// last case `isRecording` stays `true`: the old session is still live,
    /// so `whileStopped` does not run. A stop pressed during the restart
    /// ends the session, and this returns `false` without running
    /// `whileStopped` if the old session had not stopped yet.
    @discardableResult
    func restartIfRecording(reason: String, whileStopped: (() -> Void)? = nil) async -> Bool {
        guard sessionPhase == .recording else { return false }
        Self.log.info("Config changed while recording (\(reason)) — finalizing and restarting")
        // The whole restart reads as starting: Stop pressed during it ends
        // the session (DOLL-659), and other starts and restarts wait.
        isStartingRecording = true
        defer { isStartingRecording = false }
        let generation = sessionGeneration
        stopTimer()
        let result = await runEngine(engineCalls.stop)
        // A stop pressed meanwhile ended the session and does the teardown.
        guard generation == sessionGeneration else { return false }
        // Classified like stop(): a failed stop usually means the engine
        // stopped and only the finalize failed, but if it is still
        // recording, running whileStopped would release the live folder's
        // security scope under it, and the restart would then fail and
        // show an idle UI over a running engine.
        let outcome = SessionPolicy.stopOutcome(
            stopSucceeded: result.isSuccess,
            engineStillRecording: !result.isSuccess && bridge.isRecording
        )
        if !result.isSuccess {
            let failure = bridge.lastError ?? String(localized: "Failed to stop recording")
            Self.log.error("Failed to stop recording for a restart (code \(result.rawValue)): \(failure)")
            setTransientError(failure)
        }
        guard SessionPolicy.restartProceeds(after: outcome) else {
            // Keep the session's timer, and with it the status poll.
            startTimer()
            return false
        }
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
        await startRecordingInternal(generation: generation, isRestart: true)
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

    /// Switch the input device, restarting a live recording (or the
    /// monitoring stream) onto it. Returns `false`, doing nothing, when
    /// `name` is already the applied device: a pick in the menu writes the
    /// setting, and an open Settings window's picker then reports the same
    /// change back, which used to restart the recording a second time.
    @discardableResult
    func selectDevice(_ name: String) -> Bool {
        UserDefaults.standard.set(name, forKey: SettingsKeys.inputDevice)
        guard name != appliedInputDevice else { return false }
        appliedInputDevice = name
        bridge.setConfig(["input_device": name])
        if isRecording {
            Task { await restartIfRecording(reason: "device changed") }
        } else if isMonitoring {
            restartMonitoring()
        }
        return true
    }

    // MARK: - Config snapshot (DOLL-233)

    /// Read the live UserDefaults values once and freeze them for the
    /// duration of the recording. The per-tick callbacks
    /// (`computeRotationCountdown`, `computeCurrentFileSize`,
    /// `evaluatePreflightFileSizeWarning`) read this snapshot rather
    /// than hitting UserDefaults 5-7 times every second.
    func captureConfigSnapshot() -> RecordingConfigSnapshot {
        let defaults = UserDefaults.standard
        let bitDepthValue = defaults.integer(forKey: SettingsKeys.bitDepth)
        return RecordingConfigSnapshot(
            continuousMode: defaults.object(forKey: SettingsKeys.continuousMode) as? Bool ?? false,
            recordingCadence: defaults.integer(forKey: SettingsKeys.recordingCadence),
            channelCount: countChannels(defaults.string(forKey: SettingsKeys.audioChannels) ?? "1"),
            bitDepth: bitDepthValue > 0 ? bitDepthValue : 24,
            outputMode: defaults.string(forKey: SettingsKeys.outputMode) ?? "split",
            deviceName: resolvedInputDeviceName(
                selected: defaults.string(forKey: SettingsKeys.inputDevice) ?? "",
                available: availableDevices,
                systemDefault: systemDefaultDeviceName
            )
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

    /// Set `preflightSizeWarning` when the projected per-file bytes per
    /// rotation exceed the 4 GiB WAV cap. The engine starts a new file
    /// before a WAV reaches that size; this tells the user up front that a
    /// rotation will be split into 4 GB files.
    ///
    /// The menu warning is recomputed every time, but the notification and
    /// log line fire only on a fresh start, or on a restart whose projection
    /// changed: a device change or a setting applied mid-recording used to
    /// repeat the same notification every time.
    /// DOLL-233: reads from the cached snapshot, populated by
    /// startRecordingInternal before the engine starts.
    ///
    /// The announcement is a notification plus the menu banner, never a
    /// modal: it is information about a session that is already running,
    /// and a modal on this path blocked the main actor mid-start.
    func evaluatePreflightFileSizeWarning(isRestart: Bool) {
        let estimate = configSnapshot.flatMap { snapshot in
            SessionPolicy.preflightSizeEstimate(
                rotationSeconds: snapshot.continuousMode ? snapshot.recordingCadence : nil,
                channelCount: snapshot.channelCount,
                bitDepth: snapshot.bitDepth,
                splitOutput: snapshot.outputMode == "split",
                sampleRate: sampleRate
            )
        }
        let announce = SessionPolicy.shouldAnnouncePreflightWarning(
            current: estimate,
            previous: lastPreflightEstimate,
            isRestart: isRestart
        )
        lastPreflightEstimate = estimate

        guard let estimate else {
            preflightSizeWarning = nil
            return
        }
        let gigabytes = Double(estimate.bytesPerFile) / 1_073_741_824.0
        // DOLL-439/#40: localizable + locale-aware decimal (the GB value is
        // pre-formatted so the String Catalog key carries a %@, not a "."-only %.1f).
        let rateNote = estimate.sampleRateKnown ? "" : String(localized: " (estimated at 48 kHz)")
        let gbText = gigabytes.formatted(.number.precision(.fractionLength(1)))
        let msg = String(
            localized:
                "Each rotation will produce roughly \(gbText) GB\(rateNote). WAV files are capped at 4 GB, so BlackBox will start a new file each time one reaches 4 GB. Shorten the rotation interval if you want evenly sized files."
        )
        preflightSizeWarning = msg
        guard announce else { return }
        Self.log.warning("Pre-flight 4 GiB cap warning: \(msg)")
        postNotification(
            title: String(localized: "Large file warning"),
            body: msg,
            identifier: "preflight-4gb-warning"
        )
    }
}
