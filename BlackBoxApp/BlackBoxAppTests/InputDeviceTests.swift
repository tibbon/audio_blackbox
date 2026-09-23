import XCTest

@testable import BlackBox_Audio_Recorder

nonisolated final class InputDeviceTests: StandardDefaultsTestCase {
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

    /// The menu names the device the session opened, captured at start. It
    /// used to resolve the live device list, so plugging in the chosen
    /// device mid-session (the engine still on the default it fell back to)
    /// renamed the device being recorded.
    @MainActor
    func testTheSessionKeepsTheDeviceItStartedOn() {
        let recorder = RecordingState()
        UserDefaults.standard.set("Scarlett", forKey: SettingsKeys.inputDevice)
        recorder.availableDevices = ["Mic"]
        recorder.systemDefaultDeviceName = "Mic"
        recorder.configSnapshot = recorder.captureConfigSnapshot()

        recorder.availableDevices = ["Mic", "Scarlett"]

        XCTAssertEqual(recorder.configSnapshot?.deviceName, "Mic")
    }
}
