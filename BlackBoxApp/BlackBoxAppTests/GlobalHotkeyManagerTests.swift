import Carbon
import SwiftUI
import XCTest

@testable import BlackBox_Audio_Recorder

nonisolated final class GlobalHotkeyManagerTests: XCTestCase {
    /// A rebind to a combination someone else owns must leave the working
    /// shortcut registered: the caller does not save the failed one, so
    /// Settings keeps showing the old shortcut and it has to keep working.
    @MainActor
    func testFailedRebindKeepsThePreviousShortcut() throws {
        let manager = GlobalHotkeyManager.shared
        let modifiers = UInt32(cmdKey | optionKey | controlKey | shiftKey)
        let working = GlobalHotkeyManager.Shortcut(keyCode: UInt32(kVK_F13), carbonModifiers: modifiers)
        let taken = GlobalHotkeyManager.Shortcut(keyCode: UInt32(kVK_F14), carbonModifiers: modifiers)

        // Occupy `taken` outside the manager, as another app would.
        var blocker: EventHotKeyRef?
        let status = RegisterEventHotKey(
            taken.keyCode,
            taken.carbonModifiers,
            EventHotKeyID(signature: OSType(0x5445_5354), id: 9),  // "TEST"
            GetApplicationEventTarget(),
            0,
            &blocker
        )
        try XCTSkipUnless(status == noErr, "could not occupy the test combination (OSStatus \(status))")
        defer {
            if let blocker { UnregisterEventHotKey(blocker) }
            manager.unregister()
        }

        XCTAssertTrue(manager.register(working))
        let rebound = manager.register(taken)
        try XCTSkipIf(rebound, "Carbon accepted a duplicate registration, so the failure can't be provoked here")

        XCTAssertEqual(manager.currentShortcut, working, "the previous shortcut must be registered again")
    }
}

/// The Settings / onboarding shortcut recorder's capture lifecycle.
nonisolated final class ShortcutRecorderTests: StandardDefaultsTestCase {
    /// Backing storage for the recorder's bindings, as a view's @State would be.
    @MainActor
    final class Bindings {
        var label = "None"
        var isRecording = false
        var error: String?

        func recorder() -> ShortcutRecorderButton {
            ShortcutRecorderButton(
                shortcutLabel: Binding(get: { self.label }, set: { self.label = $0 }),
                isRecording: Binding(get: { self.isRecording }, set: { self.isRecording = $0 }),
                error: Binding(get: { self.error }, set: { self.error = $0 })
            )
        }
    }

    /// Tearing the button down mid-capture (the onboarding step swapped out,
    /// the Settings window closed) ends the capture. It used to only remove
    /// the key monitor, leaving the binding true, so the button came back
    /// stuck on "Press shortcut…".
    @MainActor
    func testDismantlingMidCaptureEndsTheCapture() {
        let state = Bindings()
        let coordinator = ShortcutRecorderButton.Coordinator(parent: state.recorder())
        coordinator.startRecording()
        XCTAssertTrue(state.isRecording)

        ShortcutRecorderButton.dismantleNSView(ShortcutRecorderNSButton(), coordinator: coordinator)

        XCTAssertFalse(state.isRecording)
        XCTAssertNil(coordinator.localMonitor)
    }
}

/// Onboarding's shortcut step is opt-in: ⌘⇧R is the browsers' hard reload,
/// so nothing is registered or saved unless the user chooses a shortcut.
nonisolated final class OnboardingShortcutTests: StandardDefaultsTestCase {
    private static let hyper = UInt32(cmdKey | optionKey | controlKey | shiftKey)

    /// Reaching the step and finishing (or skipping) onboarding without
    /// choosing leaves no global shortcut. The step used to register and
    /// save ⌘⇧R as soon as it appeared.
    @MainActor
    func testFinishingOnboardingWithoutChoosingSetsNoShortcut() {
        let manager = GlobalHotkeyManager.shared
        manager.save(nil)
        manager.unregister()

        // What the step does when it appears.
        XCTAssertEqual(OnboardingView.KeyboardShortcutStep.savedShortcutLabel(), String(localized: "None"))
        // What "Start Using BlackBox" and "Skip Setup" write.
        _ = OnboardingSettings.applyRecordingMode(continuous: true, silenceGate: true, to: .standard)
        _ = OnboardingSettings.applySkip(to: .standard)

        XCTAssertNil(manager.currentShortcut, "no hotkey may be registered")
        XCTAssertNil(manager.loadSaved(), "no hotkey may be saved for launch restore")
    }

    /// A shortcut saved earlier shows on "Run Setup Again" and stays saved.
    @MainActor
    func testRerunShowsTheSavedShortcutAndKeepsIt() {
        let manager = GlobalHotkeyManager.shared
        let saved = GlobalHotkeyManager.Shortcut(keyCode: UInt32(kVK_F13), carbonModifiers: Self.hyper)
        manager.save(saved)

        XCTAssertEqual(OnboardingView.KeyboardShortcutStep.savedShortcutLabel(), saved.displayString)
        _ = OnboardingSettings.applyRecordingMode(continuous: true, silenceGate: true, to: .standard)
        XCTAssertEqual(manager.loadSaved(), saved)
    }

    /// Clicking "Use ⌘⇧R" registers it and saves it so launch restore
    /// brings it back.
    @MainActor
    func testChoosingTheSuggestedShortcutRegistersAndSavesIt() throws {
        let manager = GlobalHotkeyManager.shared
        manager.save(nil)
        manager.unregister()
        defer { manager.unregister() }

        let suggested = GlobalHotkeyManager.suggestedShortcut
        XCTAssertEqual(suggested.displayString, "⇧⌘R")
        let chosen = OnboardingView.KeyboardShortcutStep.chooseSuggestedShortcut()
        try XCTSkipUnless(chosen, "\(suggested.displayString) is owned by another app on this machine")

        XCTAssertEqual(manager.currentShortcut, suggested)
        XCTAssertEqual(manager.loadSaved(), suggested)
    }

    /// A combination that can't be registered is not saved: it would never
    /// fire, and launch restore would only report the failure.
    @MainActor
    func testAShortcutThatFailsToRegisterIsNotSaved() throws {
        let manager = GlobalHotkeyManager.shared
        manager.save(nil)
        let taken = GlobalHotkeyManager.Shortcut(keyCode: UInt32(kVK_F15), carbonModifiers: Self.hyper)
        var blocker: EventHotKeyRef?
        let status = RegisterEventHotKey(
            taken.keyCode,
            taken.carbonModifiers,
            EventHotKeyID(signature: OSType(0x5445_5354), id: 10),  // "TEST"
            GetApplicationEventTarget(),
            0,
            &blocker
        )
        try XCTSkipUnless(status == noErr, "could not occupy the test combination (OSStatus \(status))")
        defer {
            if let blocker { UnregisterEventHotKey(blocker) }
            manager.unregister()
        }

        let saved = manager.registerAndSave(taken)
        try XCTSkipIf(saved, "Carbon accepted a duplicate registration, so the failure can't be provoked here")
        XCTAssertNil(manager.loadSaved())
    }
}
