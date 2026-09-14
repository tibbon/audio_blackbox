import AppKit

/// Handles system-initiated termination (logout, restart, shutdown) by gracefully
/// finalizing any active recording before allowing the app to quit.
///
/// Also prevents SwiftUI from terminating the app when the last Window scene closes,
/// which is a known issue with MenuBarExtra + Window combinations.
///
/// Main-actor-isolated: each notification below is consumed by a main-actor
/// Task (`for await` over `NotificationCenter.notifications(named:)`), so the
/// handlers are compiler-proven main-actor code rather than relying on a
/// `queue: .main` delivery convention the type checker can't see.
@MainActor
class AppDelegate: NSObject, NSApplicationDelegate {
    weak var recorder: RecordingState?

    /// Set to true before calling NSApp.terminate() from Quit menu items.
    /// Prevents SwiftUI's spurious terminate-on-last-window-close from killing the app.
    var explicitQuit = false

    /// One Task per observed notification, cancelled in `applicationWillTerminate`
    /// so the observations end with the app instead of outliving the delegate.
    private var notificationTasks: [Task<Void, Never>] = []

    func applicationDidFinishLaunching(_: Notification) {
        // Ensure we start as an accessory app (menu bar only, no Dock icon).
        NSApp.setActivationPolicy(.accessory)

        // System shutdown/logout fires willPowerOff before applicationShouldTerminate.
        // Mark it as explicit so we cooperate with the system instead of blocking.
        let wsnc = NSWorkspace.shared.notificationCenter

        observe(NSWorkspace.willPowerOffNotification, on: wsnc) { [weak self] in
            // DOLL-183: drain the recording directly here, not in the later
            // applicationShouldTerminate dispatch. macOS gives ~5s for
            // shutdown; if SwiftUI is slow to deliver
            // applicationShouldTerminate (other apps holding the run loop,
            // scene teardown), the recording can be killed before finalize
            // and the WAV header is left without correct RIFF/data sizes.
            // stop() is fast and the subsequent applicationShouldTerminate
            // will no-op on the already-stopped recorder.
            self?.explicitQuit = true
            if let recorder = self?.recorder, recorder.isRecording {
                recorder.stop()
            }
        }

        observe(NSWorkspace.willSleepNotification, on: wsnc) { [weak self] in
            self?.recorder?.handleWillSleep()
        }

        observe(NSWorkspace.didWakeNotification, on: wsnc) { [weak self] in
            self?.recorder?.handleDidWake()
        }

        observe(NSWorkspace.sessionDidResignActiveNotification, on: wsnc) { [weak self] in
            self?.recorder?.handleSessionDidResignActive()
        }

        observe(NSWorkspace.sessionDidBecomeActiveNotification, on: wsnc) { [weak self] in
            self?.recorder?.handleSessionDidBecomeActive()
        }

        // DOLL-185: re-check notification authorization when the app
        // becomes active. NSApplication.didBecomeActiveNotification
        // fires when the user clicks back into the app after granting
        // permission in System Settings, so the recorder picks up the
        // new state without a relaunch.
        observe(NSApplication.didBecomeActiveNotification, on: NotificationCenter.default) { [weak self] in
            self?.recorder?.refreshNotificationAuthorization()
        }
    }

    func applicationWillTerminate(_: Notification) {
        for task in notificationTasks {
            task.cancel()
        }
        notificationTasks.removeAll()
    }

    /// Run `handler` on the main actor each time `name` is posted to `center`.
    private func observe(
        _ name: Notification.Name,
        on center: NotificationCenter,
        handler: @escaping @MainActor () -> Void
    ) {
        notificationTasks.append(
            Task {
                for await _ in center.notifications(named: name) {
                    handler()
                }
            }
        )
    }

    func applicationShouldTerminateAfterLastWindowClosed(_: NSApplication) -> Bool {
        false
    }

    func applicationShouldTerminate(_: NSApplication) -> NSApplication.TerminateReply {
        // SwiftUI triggers NSApp.terminate() when the last Window scene closes.
        // Block that — we're a menu bar app and should stay alive.
        guard explicitQuit else {
            NSApp.setActivationPolicy(.accessory)
            return .terminateCancel
        }

        // Explicit quit (user or system) — finalize recordings gracefully.
        if let recorder, recorder.isRecording {
            recorder.stop()
        }
        recorder?.releaseOutputDirAccess()
        return .terminateNow
    }
}
