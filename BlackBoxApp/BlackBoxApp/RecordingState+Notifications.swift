import AppKit
import Foundation
import UserNotifications

import struct os.Logger

extension RecordingState {
    // MARK: - Notifications

    /// One-shot wrapper for the deferred first-recording request (DOLL-464).
    /// No-op on launches where init() already requested (onboarding done)
    /// or after the first call; the system dialog itself only ever shows
    /// once per install regardless.
    func requestNotificationAuthIfNeeded() {
        guard !hasRequestedNotificationAuth else { return }
        requestNotificationAuth()
    }

    func requestNotificationAuth() {
        hasRequestedNotificationAuth = true
        let center = UNUserNotificationCenter.current()
        // DOLL-185: capture the granted bool. Without this, a denial
        // silently drops every later postNotification (sleep-paused,
        // recording-stopped, wake events) and the user has no signal.
        center.requestAuthorization(options: [.alert, .sound]) { [weak self] granted, error in
            if let error {
                Self.log.warning("Notification auth request failed: \(error.localizedDescription, privacy: .public)")
            }
            Task { @MainActor in
                self?.notificationsAuthorized = granted
            }
        }

        // Register "Restart Recording" action on recording-stopped notifications
        let restartAction = UNNotificationAction(
            identifier: "restart-recording",
            title: String(localized: "Restart Recording")
        )
        let category = UNNotificationCategory(
            identifier: "recording-stopped",
            actions: [restartAction],
            intentIdentifiers: []
        )
        center.setNotificationCategories([category])
        center.delegate = notificationDelegate
    }

    /// Re-query notification authorization. Called from app-becomes-active
    /// so a user who grants permission in System Settings has the app
    /// pick that up without a relaunch (DOLL-185).
    func refreshNotificationAuthorization() {
        UNUserNotificationCenter.current()
            .getNotificationSettings { [weak self] settings in
                let granted =
                    settings.authorizationStatus == .authorized
                    || settings.authorizationStatus == .provisional
                Task { @MainActor in
                    self?.notificationsAuthorized = granted
                }
            }
    }

    /// Post a notification to Notification Center for events that occur while the app is in the background.
    /// Uses a fixed identifier so new notifications of the same type replace old ones instead of stacking.
    func postNotification(title: String, body: String, identifier: String = "blackbox-info") {
        let content = UNMutableNotificationContent()
        content.title = title
        content.body = body
        content.sound = isRecording ? nil : .default
        if identifier == "recording-stopped" {
            content.categoryIdentifier = "recording-stopped"
        }
        let request = UNNotificationRequest(identifier: identifier, content: content, trigger: nil)
        UNUserNotificationCenter.current().add(request)
    }

    /// Notify the user of a critical event using the appropriate channel:
    /// modal alert if the app is in the foreground, notification if backgrounded.
    /// Avoids showing both simultaneously.
    func notifyUser(title: String, message: String, identifier: String = "recording-stopped") {
        if NSApp.isActive {
            showCriticalAlert(title: title, message: message)
        } else {
            postNotification(title: title, body: message, identifier: identifier)
        }
    }

    /// Set a transient error that auto-clears after 30 seconds.
    /// Use for errors that don't require ongoing user action (device disconnect, disk full, etc.).
    func setTransientError(_ message: String) {
        errorMessage = message
        statusText = String(localized: "Error")
        Task { [weak self] in
            try? await Task.sleep(for: .seconds(30))
            guard let self, errorMessage == message else { return }
            errorMessage = nil
            if !isRecording { statusText = String(localized: "Ready") }
        }
    }

    /// Show an NSAlert for critical errors that require the user's attention.
    private func showCriticalAlert(title: String, message: String) {
        let alert = NSAlert()
        alert.messageText = title
        alert.informativeText = message
        alert.alertStyle = .warning
        alert.addButton(withTitle: String(localized: "OK"))
        NSApp.activate(ignoringOtherApps: true)
        alert.runModal()
    }
}

// MARK: - Notification Action Handler

/// Handles notification action responses (e.g. "Restart Recording" button).
/// Separate class because UNUserNotificationCenterDelegate requires NSObject conformance.
class NotificationDelegate: NSObject, UNUserNotificationCenterDelegate {
    func userNotificationCenter(
        _: UNUserNotificationCenter,
        didReceive response: UNNotificationResponse,
        withCompletionHandler handler: @escaping () -> Void
    ) {
        if response.actionIdentifier == "restart-recording" {
            Task { @MainActor in
                // Find the RecordingState — it's the source of truth for the app
                if let app = NSApp.delegate as? AppDelegate, let recorder = app.recorder {
                    recorder.start()
                }
            }
        }
        handler()
    }

    /// Show notifications even when the app is in the foreground (needed for
    /// notification actions to be accessible).
    func userNotificationCenter(
        _: UNUserNotificationCenter,
        willPresent _: UNNotification,
        withCompletionHandler handler: @escaping (UNNotificationPresentationOptions) -> Void
    ) {
        handler([.banner])
    }
}
