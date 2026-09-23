import AppKit

/// Handles system-initiated termination (logout, restart, shutdown) and quit
/// requests from other apps by gracefully finalizing any active recording
/// before allowing the app to quit.
///
/// Also prevents SwiftUI from terminating the app when the last Window scene closes,
/// which is a known issue with MenuBarExtra + Window combinations.
///
/// Main-actor-isolated. Most notifications below are consumed by a main-actor
/// Task (`for await` over `NotificationCenter.notifications(named:)`), so the
/// handlers are compiler-proven main-actor code. `willPowerOff` and
/// `willSleep` are the exception: they must be handled inside the post
/// (see `observeSynchronously`).
@MainActor
class AppDelegate: NSObject, NSApplicationDelegate {
    weak var recorder: RecordingState?

    /// Set to true before calling NSApp.terminate() from Quit menu items.
    /// Prevents SwiftUI's spurious terminate-on-last-window-close from killing the app.
    var explicitQuit = false

    /// One Task per observed notification, cancelled in `applicationWillTerminate`
    /// so the observations end with the app instead of outliving the delegate.
    private var notificationTasks: [Task<Void, Never>] = []

    /// Block-based observers for the notifications handled synchronously,
    /// removed in `applicationWillTerminate`.
    private var synchronousObservers: [(center: NotificationCenter, token: any NSObjectProtocol)] = []

    func applicationDidFinishLaunching(_: Notification) {
        // Ensure we start as an accessory app (menu bar only, no Dock icon).
        NSApp.setActivationPolicy(.accessory)
        installSystemObservers(
            workspaceCenter: NSWorkspace.shared.notificationCenter,
            appCenter: NotificationCenter.default
        )
    }

    /// Subscribe to the power, sleep, session and activation notifications.
    /// Separate from `applicationDidFinishLaunching` so tests can drive it
    /// with private notification centers.
    func installSystemObservers(workspaceCenter wsnc: NotificationCenter, appCenter: NotificationCenter) {
        // System shutdown/logout fires willPowerOff before applicationShouldTerminate.
        // Mark it as explicit so we cooperate with the system instead of blocking.
        observeSynchronously(NSWorkspace.willPowerOffNotification, on: wsnc) { [weak self] in
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

        // Synchronous for the same reason: the system can sleep as soon as
        // the post returns, before a later main-actor turn would stop and
        // finalize the recording.
        observeSynchronously(NSWorkspace.willSleepNotification, on: wsnc) { [weak self] in
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
        observe(NSApplication.didBecomeActiveNotification, on: appCenter) { [weak self] in
            self?.recorder?.refreshNotificationAuthorization()
        }
    }

    func applicationWillTerminate(_: Notification) {
        for task in notificationTasks {
            task.cancel()
        }
        notificationTasks.removeAll()
        for observer in synchronousObservers {
            observer.center.removeObserver(observer.token)
        }
        synchronousObservers.removeAll()
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

    /// Run `handler` inside the notification post itself, before the poster
    /// continues. An `observe` Task runs its handler on a later main-actor
    /// turn: for willPowerOff that turn can come after AppKit has already
    /// asked `applicationShouldTerminate`, which then sees
    /// `explicitQuit == false` and vetoes the logout or shutdown; for
    /// willSleep the Mac can be asleep before the recording is finalized.
    private func observeSynchronously(
        _ name: Notification.Name,
        on center: NotificationCenter,
        handler: @escaping @MainActor () -> Void
    ) {
        // queue: nil runs the block synchronously on the posting thread.
        let token = center.addObserver(forName: name, object: nil, queue: nil) { _ in
            guard Thread.isMainThread else {
                // Not expected (NSWorkspace posts these on the main thread),
                // but never assume isolation we don't have: fall back to a hop.
                Task { @MainActor in handler() }
                return
            }
            // SAFETY: the guard above proves this block is running on the main
            // thread, which is the main actor's executor, so running the
            // main-actor handler here cannot race main-actor state.
            // swiftlint:disable:next assume_isolated - checked Thread.isMainThread on the line above; the post must be handled before it returns
            MainActor.assumeIsolated { handler() }
        }
        synchronousObservers.append((center, token))
    }

    func applicationShouldTerminateAfterLastWindowClosed(_: NSApplication) -> Bool {
        false
    }

    func applicationShouldTerminate(_: NSApplication) -> NSApplication.TerminateReply {
        shouldTerminate(currentAppleEvent: NSAppleEventManager.shared().currentAppleEvent)
    }

    /// `applicationShouldTerminate`, with the Apple event being handled (if
    /// any) passed in so tests can supply one.
    func shouldTerminate(currentAppleEvent: NSAppleEventDescriptor?) -> NSApplication.TerminateReply {
        // SwiftUI triggers NSApp.terminate() when the last Window scene closes.
        // Block that — we're a menu bar app and should stay alive. A quit
        // Apple event is not that: it is another process asking us to quit
        // (Activity Monitor's Quit, an installer, `osascript -e 'quit app'`),
        // and cancelling it left the app running with nothing telling the
        // sender why.
        guard explicitQuit || Self.isQuitAppleEvent(currentAppleEvent) else {
            NSApp.setActivationPolicy(.accessory)
            return .terminateCancel
        }

        // Explicit quit (user, system or another app) — finalize recordings
        // gracefully. stop() is synchronous: the files are finalized before
        // this returns, so terminating now loses nothing.
        if let recorder, recorder.isRecording {
            recorder.stop()
        }
        recorder?.releaseOutputDirAccess()
        return .terminateNow
    }

    /// Whether `event` is the core `quit` Apple event ('aevt'/'quit'), which
    /// is how other processes, the Dock and logout ask an app to quit.
    /// SwiftUI's last-window terminate calls `terminate(_:)` directly, outside
    /// any Apple event, so it never matches.
    nonisolated static func isQuitAppleEvent(_ event: NSAppleEventDescriptor?) -> Bool {
        guard let event else { return false }
        return event.eventClass == kCoreEventClass && event.eventID == kAEQuitApplication
    }
}
