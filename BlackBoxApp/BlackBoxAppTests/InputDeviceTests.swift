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

    /// A menu pick applies the device once. The open Settings picker then
    /// sees the same value and must not apply it again: that restarted a
    /// live recording a second time.
    @MainActor
    func testReselectingTheAppliedDeviceDoesNothing() {
        let recorder = RecordingState()
        recorder.appliedInputDevice = ""

        XCTAssertTrue(recorder.selectDevice("Scarlett"))
        XCTAssertEqual(recorder.appliedInputDevice, "Scarlett")
        XCTAssertEqual(UserDefaults.standard.string(forKey: SettingsKeys.inputDevice), "Scarlett")
        XCTAssertEqual(recorder.bridge.getConfig()?["input_device"] as? String, "Scarlett")

        XCTAssertFalse(recorder.selectDevice("Scarlett"))
        XCTAssertTrue(recorder.selectDevice(""))
    }
}
