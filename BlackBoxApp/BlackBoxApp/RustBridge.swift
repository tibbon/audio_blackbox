import Foundation
import BlackBoxFFI

/// Typed error codes matching the Rust FFI BLACKBOX_ERR_* constants.
enum BlackBoxError: Int32, Error {
    case ok = 0
    case invalidHandle = -1
    case audioDevice = -2
    case config = -3
    case io = -4
    case lockPoisoned = -5
    /// Reserved (DOLL-128). No FFI function currently returns -6 — the
    /// catch_unwind path that produced it was removed in DOLL-90. Kept
    /// to maintain a stable mapping if a future error claims the slot.
    case `internal` = -6
    case diskSpaceLow = -7
    case invalidArg = -8
    case unknown = -99

    init(code: Int32) {
        self = BlackBoxError(rawValue: code) ?? .unknown
    }

    var isSuccess: Bool { self == .ok }
}

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

    /// Start recording. Returns a typed error code.
    @discardableResult
    func startRecording() -> BlackBoxError {
        guard let handle = handle else { return .invalidHandle }
        return BlackBoxError(code: blackbox_start_recording(handle))
    }

    /// Stop recording. Returns a typed error code.
    @discardableResult
    func stopRecording() -> BlackBoxError {
        guard let handle = handle else { return .invalidHandle }
        return BlackBoxError(code: blackbox_stop_recording(handle))
    }

    /// Whether recording is currently active.
    var isRecording: Bool {
        guard let handle = handle else { return false }
        return blackbox_is_recording(handle)
    }

    // MARK: - Monitoring Control

    /// Start audio monitoring (levels without recording). Returns a typed error code.
    @discardableResult
    func startMonitoring() -> BlackBoxError {
        guard let handle = handle else { return .invalidHandle }
        return BlackBoxError(code: blackbox_start_monitoring(handle))
    }

    /// Stop audio monitoring. Returns a typed error code.
    @discardableResult
    func stopMonitoring() -> BlackBoxError {
        guard let handle = handle else { return .invalidHandle }
        return BlackBoxError(code: blackbox_stop_monitoring(handle))
    }

    /// Whether audio monitoring is currently active.
    var isMonitoring: Bool {
        guard let handle = handle else { return false }
        return blackbox_is_monitoring(handle)
    }

    // MARK: - Status & Configuration

    /// Get lightweight status flags (no JSON, no string allocation).
    func getStatusFlags() -> StatusFlags? {
        guard let handle = handle else { return nil }
        var flags = StatusFlags()
        if blackbox_get_status_flags(handle, &flags) == BLACKBOX_OK {
            return flags
        }
        return nil
    }

    /// Get the current configuration as a dictionary.
    func getConfig() -> [String: Any]? {
        guard let handle = handle else { return nil }
        return readJSON { blackbox_get_config_json(handle) } as? [String: Any]
    }

    /// Update configuration with the given dictionary.
    /// Only fields present in the dictionary are updated.
    @discardableResult
    func setConfig(_ config: [String: Any]) -> BlackBoxError {
        // DOLL-105: distinguish handle nullity from JSON-encoding failure.
        // The previous combined `guard` returned `.invalidHandle` for both,
        // misleading any caller that inspects the error code.
        guard let handle = handle else { return .invalidHandle }
        guard let jsonData = try? JSONSerialization.data(withJSONObject: config),
              let jsonString = String(data: jsonData, encoding: .utf8) else {
            return .config
        }
        return BlackBoxError(code: jsonString.withCString { blackbox_set_config_json(handle, $0) })
    }

    /// Get the last error message, or nil if no error.
    var lastError: String? {
        guard let handle = handle else { return nil }
        return readString { blackbox_get_last_error(handle) }
    }

    // MARK: - Peak Levels (lightweight, no JSON)

    /// Write peak levels into a caller-provided buffer. Returns the channel
    /// count on success, or a typed `BlackBoxError` on failure (DOLL-125).
    /// Zero-allocation path for the meter polling loop.
    func fillPeakLevels(into buffer: inout [Float]) -> Result<Int, BlackBoxError> {
        guard let handle = handle else { return .failure(.invalidHandle) }
        let count = blackbox_get_peak_levels(handle, &buffer, Int32(buffer.count))
        if count >= 0 {
            return .success(Int(count))
        }
        // Negative codes from Rust are typed errors — surface them rather
        // than collapsing to 0 (which the prior `max(Int(count), 0)` did,
        // hiding lock-poison + invalid-arg + invalid-handle).
        return .failure(BlackBoxError(code: count))
    }

    // MARK: - Device Enumeration

    /// Get the input channel count for a device by name.
    /// Pass empty string for the system default device.
    /// Returns the channel count on success, or a typed `BlackBoxError`
    /// (DOLL-125) — distinguishes BLACKBOX_ERR_AUDIO_DEVICE (-2, "device
    /// missing/unreadable") from BLACKBOX_ERR_INVALID_ARG (-8, "fix your
    /// buffer / invalid UTF-8") rather than collapsing both to nil.
    static func getDeviceChannelCount(deviceName: String) -> Result<Int, BlackBoxError> {
        let count: Int32
        if deviceName.isEmpty {
            count = blackbox_get_device_channel_count(nil)
        } else {
            count = deviceName.withCString { blackbox_get_device_channel_count($0) }
        }
        if count >= 0 {
            return .success(Int(count))
        }
        return .failure(BlackBoxError(code: count))
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
    /// Parses directly from the C buffer to avoid an intermediate Swift String copy.
    private func readJSON(_ call: () -> UnsafeMutablePointer<CChar>?) -> Any? {
        guard let ptr = call() else { return nil }
        defer { blackbox_free_string(ptr) }
        let len = strlen(ptr)
        let data = Data(bytes: ptr, count: len)
        return try? JSONSerialization.jsonObject(with: data)
    }
}
