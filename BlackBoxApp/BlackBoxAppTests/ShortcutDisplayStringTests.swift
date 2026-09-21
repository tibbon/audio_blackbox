import Carbon
import XCTest

@testable import BlackBox_Audio_Recorder

// Test cases opt out of the module's MainActor default: XCTest's inherited
// initializers and setUp/tearDown are nonisolated, and an isolated subclass
// can't override them. Tests that touch main-actor types are marked @MainActor.

// MARK: - Shortcut display names

/// Locks in the key-code → name table behind `Shortcut.displayString`
/// (DOLL-653 turned `keyCodeToString`'s switch into a dictionary). The expected
/// names are the ones the original switch returned. A shortcut with no
/// modifiers displays exactly the key's name, so these tests reach the private
/// `keyCodeToString` through `displayString` without widening its access.
nonisolated final class ShortcutDisplayStringTests: XCTestCase {
    @MainActor
    func testLetters() {
        assertNames([
            (kVK_ANSI_A, "A"),
            (kVK_ANSI_M, "M"),
            (kVK_ANSI_R, "R"),
            (kVK_ANSI_Z, "Z"),
        ])
    }

    @MainActor
    func testDigits() {
        assertNames([
            (kVK_ANSI_0, "0"),
            (kVK_ANSI_1, "1"),
            (kVK_ANSI_5, "5"),
            (kVK_ANSI_9, "9"),
        ])
    }

    @MainActor
    func testFunctionKeys() {
        assertNames([
            (kVK_F1, "F1"),
            (kVK_F5, "F5"),
            (kVK_F10, "F10"),
            (kVK_F12, "F12"),
        ])
    }

    @MainActor
    func testNamedKeys() {
        assertNames([
            (kVK_Space, "Space"),
            (kVK_Return, "Return"),
            (kVK_Tab, "Tab"),
            (kVK_Delete, "Delete"),
            (kVK_Escape, "Esc"),
        ])
    }

    /// The original switch had no case for arrows or punctuation, so they
    /// display as the "Key<n>" fallback.
    @MainActor
    func testArrowsAndPunctuationUseFallback() {
        let codes = [
            kVK_LeftArrow,
            kVK_RightArrow,
            kVK_UpArrow,
            kVK_DownArrow,
            kVK_ANSI_Comma,
            kVK_ANSI_Period,
            kVK_ANSI_Slash,
            kVK_ANSI_Minus,
        ]
        for code in codes {
            XCTAssertEqual(displayName(code), "Key\(code)")
        }
    }

    @MainActor
    func testUnknownKeyCodeUsesFallback() {
        XCTAssertEqual(displayName(0xFFFF), "Key65535")
        XCTAssertEqual(displayName(200), "Key200")
    }

    @MainActor
    func testModifiersPrecedeKeyInFixedOrder() {
        let all = GlobalHotkeyManager.Shortcut(
            keyCode: UInt32(kVK_ANSI_R),
            carbonModifiers: UInt32(cmdKey | shiftKey | optionKey | controlKey)
        )
        XCTAssertEqual(all.displayString, "⌃⌥⇧⌘R")

        let cmdShift = GlobalHotkeyManager.Shortcut(
            keyCode: UInt32(kVK_F5),
            carbonModifiers: UInt32(cmdKey | shiftKey)
        )
        XCTAssertEqual(cmdShift.displayString, "⇧⌘F5")
    }

    // MARK: - Helpers

    @MainActor
    private func displayName(_ keyCode: Int) -> String {
        GlobalHotkeyManager.Shortcut(keyCode: UInt32(keyCode), carbonModifiers: 0).displayString
    }

    @MainActor
    private func assertNames(
        _ expected: [(keyCode: Int, name: String)],
        file: StaticString = #filePath,
        line: UInt = #line
    ) {
        for (keyCode, name) in expected {
            XCTAssertEqual(displayName(keyCode), name, "key code \(keyCode)", file: file, line: line)
        }
    }
}
