import Foundation
import BlackBoxFFI

/// Swift wrapper around the BlackBox Rust FFI, providing safe memory management
/// and Swift-native types.
final class RustBridge {
    private var handle: OpaquePointer?

    /// Create a bridge with the given configuration dictionary.
    /// Pass nil for default configuration.
    init(config: [String: Any]? = nil) {
        if let config = config,
           let jsonData = try? JSONSerialization.data(withJSONObject: config),
           let jsonString = String(data: jsonData, encoding: .utf8) {
            handle = jsonString.withCString { blackbox_create($0) }
        } else {
            handle = blackbox_create(nil)
        }
    }

    deinit {
        if let handle = handle {
            blackbox_destroy(handle)
        }
    }

    // MARK: - Recording Control

    /// Start recording. Returns true on success.
    @discardableResult
    func startRecording() -> Bool {
        guard let handle = handle else { return false }
        return blackbox_start_recording(handle) == 0
    }

    /// Stop recording. Returns true on success.
    @discardableResult
    func stopRecording() -> Bool {
        guard let handle = handle else { return false }
        return blackbox_stop_recording(handle) == 0
    }

    /// Whether recording is currently active.
    var isRecording: Bool {
        guard let handle = handle else { return false }
        return blackbox_is_recording(handle)
    }

    // MARK: - Monitoring Control

    /// Start audio monitoring (levels without recording). Returns true on success.
    @discardableResult
    func startMonitoring() -> Bool {
        guard let handle = handle else { return false }
        return blackbox_start_monitoring(handle) == 0
    }

    /// Stop audio monitoring. Returns true on success.
    @discardableResult
    func stopMonitoring() -> Bool {
        guard let handle = handle else { return false }
        return blackbox_stop_monitoring(handle) == 0
    }

    /// Whether audio monitoring is currently active.
    var isMonitoring: Bool {
        guard let handle = handle else { return false }
        return blackbox_is_monitoring(handle)
    }

    // MARK: - Status & Configuration

    /// Get the current status as a dictionary.
    func getStatus() -> [String: Any]? {
        guard let handle = handle else { return nil }
        return readJSON { blackbox_get_status_json(handle) } as? [String: Any]
    }

    /// Get the current configuration as a dictionary.
    func getConfig() -> [String: Any]? {
        guard let handle = handle else { return nil }
        return readJSON { blackbox_get_config_json(handle) } as? [String: Any]
    }

    /// Update configuration with the given dictionary.
    /// Only fields present in the dictionary are updated.
    @discardableResult
    func setConfig(_ config: [String: Any]) -> Bool {
        guard let handle = handle,
              let jsonData = try? JSONSerialization.data(withJSONObject: config),
              let jsonString = String(data: jsonData, encoding: .utf8) else {
            return false
        }
        return jsonString.withCString { blackbox_set_config_json(handle, $0) } == 0
    }

    /// Get the last error message, or nil if no error.
    var lastError: String? {
        guard let handle = handle else { return nil }
        return readString { blackbox_get_last_error(handle) }
    }

    // MARK: - Peak Levels (lightweight, no JSON)

    /// Write peak levels into a caller-provided buffer. Returns the channel count.
    /// Zero-allocation path for the meter polling loop.
    func fillPeakLevels(into buffer: inout [Float]) -> Int {
        guard let handle = handle else { return 0 }
        let count = blackbox_get_peak_levels(handle, &buffer, Int32(buffer.count))
        return max(Int(count), 0)
    }

    // MARK: - Device Enumeration

    /// Get the input channel count for a device by name.
    /// Pass empty string for the system default device.
    /// Returns nil on error.
    static func getDeviceChannelCount(deviceName: String) -> Int? {
        let count: Int32
        if deviceName.isEmpty {
            count = blackbox_get_device_channel_count(nil)
        } else {
            count = deviceName.withCString { blackbox_get_device_channel_count($0) }
        }
        return count > 0 ? Int(count) : nil
    }

    /// List available input device names.
    static func listInputDevices() -> [String] {
        guard let ptr = blackbox_list_input_devices() else { return [] }
        defer { blackbox_free_string(ptr) }
        guard let str = String(cString: ptr, encoding: .utf8) else { return [] }
        guard let data = str.data(using: .utf8),
              let array = try? JSONSerialization.jsonObject(with: data) as? [String] else {
            return []
        }
        return array
    }

    // MARK: - Private Helpers

    /// Read a C string from an FFI call, freeing it after conversion.
    private func readString(_ call: () -> UnsafeMutablePointer<CChar>?) -> String? {
        guard let ptr = call() else { return nil }
        defer { blackbox_free_string(ptr) }
        return String(cString: ptr, encoding: .utf8)
    }

    /// Read a JSON C string from an FFI call, parse it, and free the C string.
    private func readJSON(_ call: () -> UnsafeMutablePointer<CChar>?) -> Any? {
        guard let str = readString(call),
              let data = str.data(using: .utf8) else { return nil }
        return try? JSONSerialization.jsonObject(with: data)
    }
}
