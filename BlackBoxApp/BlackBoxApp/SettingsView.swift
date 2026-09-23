import StoreKit
import SwiftUI

struct SettingsView: View {
    var recorder: RecordingState
    @Environment(\.requestReview) private var requestReview

    // DOLL-253: the window is pinned to content size (.windowResizability
    // (.contentSize) in BlackBoxApp), and the user can't widen it. A hard
    // 480pt width truncated/awkwardly-wrapped the captions and picker labels
    // at large Dynamic Type sizes. @ScaledMetric grows the pinned width with
    // the user's text size (≈480 at default, wider at accessibility sizes)
    // so content keeps its proportions instead of cramming into 480pt.
    @ScaledMetric(relativeTo: .body) private var contentWidth: CGFloat = 480

    var body: some View {
        TabView {
            RecordingSettingsTab(recorder: recorder)
                .tabItem {
                    Label("Recording", systemImage: "mic")
                }

            OutputSettingsTab(recorder: recorder)
                .tabItem {
                    Label("Output", systemImage: "folder")
                }

            GeneralSettingsTab(recorder: recorder)
                .tabItem {
                    Label("General", systemImage: "slider.horizontal.3")
                }
        }
        .frame(minWidth: contentWidth, maxWidth: contentWidth, minHeight: 450)
        .background(SettingsWindowConfigurator())
        .onAppear {
            promptForReviewIfReady()
        }
    }

    private func promptForReviewIfReady() {
        let sessions = UserDefaults.standard.integer(forKey: SettingsKeys.successfulRecordingSessions)
        guard sessions >= 3 else { return }
        guard !UserDefaults.standard.bool(forKey: SettingsKeys.hasPromptedForReview) else { return }
        UserDefaults.standard.set(true, forKey: SettingsKeys.hasPromptedForReview)
        Task {
            try? await Task.sleep(for: .seconds(1))
            requestReview()
        }
    }
}

/// Disables minimize and zoom buttons on the Settings window per Apple HIG.
/// Uses viewDidMoveToWindow to configure once, not on every SwiftUI render.
private struct SettingsWindowConfigurator: NSViewRepresentable {
    func makeNSView(context _: Context) -> NSView { SettingsConfiguratorView() }
    func updateNSView(_: NSView, context _: Context) {
        // The window buttons are configured once in viewDidMoveToWindow; nothing varies per render.
    }
}

private final class SettingsConfiguratorView: NSView {
    private var configured = false

    override func viewDidMoveToWindow() {
        super.viewDidMoveToWindow()
        guard !configured, let window else { return }
        configured = true
        window.standardWindowButton(.miniaturizeButton)?.isEnabled = false
        window.standardWindowButton(.zoomButton)?.isEnabled = false
    }
}

/// Show a confirmation dialog before changing settings during an active recording.
/// Calls `onRestart` if the user confirms, or `onCancel` if they dismiss.
/// Internal (not private) because RecordingSettingsTab and OutputSettingsTab
/// live in their own files.
func confirmSettingsChange(
    reason: String,
    onRestart: () -> Void,
    onCancel: (() -> Void)? = nil
) {
    let alert = NSAlert()
    alert.messageText = String(localized: "Restart Recording?")
    alert.informativeText = String(localized: "Changing \(reason) will finalize the current file and start a new one.")
    alert.alertStyle = .informational
    alert.addButton(withTitle: String(localized: "Restart"))
    alert.addButton(withTitle: String(localized: "Cancel"))
    // DOLL-359: this dialog guards a disruptive action (finalize + split the
    // in-progress recording), so Cancel — not Restart — must be the default
    // that Return triggers. Matches confirmResetAllSettings / quitApp.
    alert.buttons.first?.keyEquivalent = ""  // Restart: no longer the default
    alert.buttons.last?.keyEquivalent = "\r"  // Cancel: default (Return)
    NSApp.activate()
    if alert.runModal() == .alertFirstButtonReturn {
        onRestart()
    } else {
        onCancel?()
    }
}

/// Apply a setting the engine only reads when a session starts (the
/// recorder holds a copy of the config): immediately when idle, and while
/// recording only after the user confirms a restart, which finalizes the
/// current file and starts a new one with the change. Pushing the config
/// alone mid-recording silently did nothing until the next session.
///
/// Returns `false` when the user cancelled; the caller reverts its control.
@discardableResult
func applySessionSetting(recorder: RecordingState, reason: String, apply: () -> Void) -> Bool {
    guard recorder.isRecording else {
        apply()
        return true
    }
    var confirmed = false
    confirmSettingsChange(reason: reason) {
        apply()
        recorder.restartIfRecording(reason: "\(reason) changed")
        confirmed = true
    }
    return confirmed
}

// SettingsKeys moved to SettingsKeys.swift (DOLL-203).
