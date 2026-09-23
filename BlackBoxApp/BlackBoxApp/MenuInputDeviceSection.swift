import SwiftUI

/// The menu-bar menu's input-device picker, or a no-devices notice with a
/// Refresh action. Each branch ends with its own Divider.
struct MenuInputDeviceSection: View {
    var recorder: RecordingState
    @AppStorage(SettingsKeys.inputDevice) private var selectedDevice: String = ""

    var body: some View {
        if !recorder.availableDevices.isEmpty {
            Menu("Input Device") {
                // DOLL-215: surface the resolved device name so users know
                // what "System Default" maps to right now.
                let defaultLabel: String =
                    recorder.systemDefaultDeviceName
                    .map { String(localized: "System Default (\($0))") } ?? String(localized: "System Default")
                Toggle(
                    defaultLabel,
                    isOn: Binding(
                        get: { selectedDevice.isEmpty },
                        set: { newValue in
                            if newValue { recorder.selectDevice("") }
                        }
                    )
                )

                Divider()

                ForEach(recorder.availableDevices, id: \.self) { device in
                    Toggle(
                        device,
                        isOn: Binding(
                            get: { selectedDevice == device },
                            set: { newValue in
                                if newValue { recorder.selectDevice(device) }
                            }
                        )
                    )
                }
            }

            Divider()
        } else {
            Text("No Input Devices")
                .accessibilityLabel("No input devices available")

            // DOLL-387: the menu is the primary surface where the user notices
            // no devices; offer inline recovery after they plug hardware in,
            // rather than making them open Settings just to re-scan.
            Button("Refresh Devices") {
                recorder.refreshDevices()
            }

            Divider()
        }
    }
}
