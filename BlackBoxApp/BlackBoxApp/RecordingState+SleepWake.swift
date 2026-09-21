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
        guard SleepWakePolicy.shouldResumeOnWake(wasInterrupted: wasSleepInterrupted) else { return }
        wasSleepInterrupted = false
        Self.log.info("Wake: attempting to resume recording")
        Task { [weak self] in
            try? await Task.sleep(for: .milliseconds(1500))
            guard let self, !isRecording else { return }
            // DOLL-443: await the outcome — the old fire-and-forget start()
            // + isRecording read always took the failure branch, posting
            // "Resume Failed" even for successful resumes.
            if await startAndWait() {
                postNotification(
                    title: String(localized: "Recording Resumed"),
                    body: String(localized: "Recording resumed after wake."),
                    identifier: "wake-resumed"
                )
            } else {
                postNotification(
                    title: String(localized: "Resume Failed"),
                    body: String(localized: "Could not restart recording after wake. Check your audio device."),
                    identifier: "wake-failed"
                )
            }
        }
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
        guard SleepWakePolicy.shouldResumeOnWake(wasInterrupted: wasSleepInterrupted) else { return }
        wasSleepInterrupted = false
        Self.log.info("Fast User Switch: attempting to resume recording")
        Task { [weak self] in
            try? await Task.sleep(for: .milliseconds(1500))
            guard let self, !isRecording else { return }
            // DOLL-443: await the outcome (see handleDidWake).
            if await startAndWait() {
                postNotification(
                    title: String(localized: "Recording Resumed"),
                    body: String(localized: "Recording resumed after session switch."),
                    identifier: "session-resumed"
                )
            } else {
                postNotification(
                    title: String(localized: "Resume Failed"),
                    body: String(localized: "Could not restart recording after session switch."),
                    identifier: "session-failed"
                )
            }
        }
    }
}
