import Foundation

import struct os.Logger

extension RecordingState {
    // MARK: - Sleep / Wake

    func handleWillSleep() {
        let behavior = UserDefaults.standard.string(forKey: SettingsKeys.sleepBehavior) ?? "resume"
        let action = SleepWakePolicy.sleepAction(isRecording: isRecording, behavior: behavior)
        switch action {
        case .ignore:
            return

        case .pauseForResume:
            wasSleepInterrupted = true
            postNotification(
                title: String(localized: "Recording Paused"),
                body: String(localized: "Your Mac is going to sleep. Recording will resume on wake."),
                identifier: "sleep-paused"
            )

        case .stop:
            postNotification(
                title: String(localized: "Recording Stopped"),
                body: String(localized: "Your Mac is going to sleep."),
                identifier: "recording-stopped"
            )
        }
        // .pauseForResume just set wasSleepInterrupted; stop() must not
        // clear it or handleDidWake never resumes (DOLL-442).
        stop(reason: action == .pauseForResume ? .sleepInterruption : .user)
        Self.log.info("Sleep: stopped recording (behavior=\(behavior))")
    }

    func handleDidWake() {
        scheduleResume(
            logMessage: "Wake: attempting to resume recording",
            resumed: (String(localized: "Recording resumed after wake."), "wake-resumed"),
            failed: (
                String(localized: "Could not restart recording after wake. Check your audio device."),
                "wake-failed"
            )
        )
    }

    func handleSessionDidResignActive() {
        let action = SleepWakePolicy.sessionResignAction(isRecording: isRecording)
        guard action == .pauseForResume else { return }
        wasSleepInterrupted = true
        stop(reason: .sleepInterruption)
        Self.log.info("Fast User Switch: stopped recording for resume on return")
        postNotification(
            title: String(localized: "Recording Paused"),
            body: String(localized: "User session switched. Recording will resume when you return."),
            identifier: "session-paused"
        )
    }

    func handleSessionDidBecomeActive() {
        scheduleResume(
            logMessage: "Fast User Switch: attempting to resume recording",
            resumed: (String(localized: "Recording resumed after session switch."), "session-resumed"),
            failed: (String(localized: "Could not restart recording after session switch."), "session-failed")
        )
    }

    /// Cancel a deferred resume that has not started yet. A user stop or
    /// start owns the recording state from then on (DOLL-182).
    func cancelPendingResume() {
        pendingResumeTask?.cancel()
        pendingResumeTask = nil
    }

    /// Resume an interrupted recording 1.5 s from now (the audio device needs
    /// a moment after wake), unless the user stops or starts a recording in
    /// the meantime.
    ///
    /// The flag is consumed here, before the delay, so the resume is tracked
    /// by `pendingResumeTask` instead: a user stop or start cancels it
    /// (`cancelPendingResume()`). Checking only `!isRecording` after the
    /// delay let a start-then-stop inside the window be resumed anyway.
    private func scheduleResume(
        logMessage: String,
        resumed: (body: String, identifier: String),
        failed: (body: String, identifier: String)
    ) {
        guard SleepWakePolicy.shouldResumeOnWake(wasInterrupted: wasSleepInterrupted) else { return }
        wasSleepInterrupted = false
        Self.log.info("\(logMessage, privacy: .public)")
        pendingResumeTask?.cancel()
        pendingResumeTask = Task { [weak self] in
            try? await Task.sleep(for: .milliseconds(1500))
            guard !Task.isCancelled, let self else { return }
            pendingResumeTask = nil
            guard !isRecording else { return }
            // DOLL-443: await the outcome — the old fire-and-forget start()
            // + isRecording read always took the failure branch, posting
            // "Resume Failed" even for successful resumes.
            if await startAndWait() {
                postNotification(
                    title: String(localized: "Recording Resumed"),
                    body: resumed.body,
                    identifier: resumed.identifier
                )
            } else {
                postNotification(
                    title: String(localized: "Resume Failed"),
                    body: failed.body,
                    identifier: failed.identifier
                )
            }
        }
    }
}
