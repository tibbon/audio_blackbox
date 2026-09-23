import XCTest

@testable import BlackBox_Audio_Recorder

nonisolated final class InputDeviceTests: XCTestCase {
    func testConnectedChosenDeviceIsShown() {
        XCTAssertEqual(
            resolvedInputDeviceName(selected: "Scarlett", available: ["Mic", "Scarlett"], systemDefault: "Mic"),
            "Scarlett"
        )
    }

    /// The engine records from the system default when the chosen device is
    /// unplugged, so that is the device to show.
    func testDisconnectedChosenDeviceShowsTheDefaultInUse() {
        XCTAssertEqual(
            resolvedInputDeviceName(selected: "Scarlett", available: ["Mic"], systemDefault: "Mic"),
            "Mic"
        )
    }

    func testNoChoiceShowsTheSystemDefault() {
        XCTAssertEqual(resolvedInputDeviceName(selected: "", available: ["Mic"], systemDefault: "Mic"), "Mic")
        XCTAssertNil(resolvedInputDeviceName(selected: "", available: [], systemDefault: nil))
    }
}
