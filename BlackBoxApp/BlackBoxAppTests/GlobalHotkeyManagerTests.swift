import Carbon
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
