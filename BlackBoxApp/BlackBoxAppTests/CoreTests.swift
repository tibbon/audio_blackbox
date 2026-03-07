import XCTest

@testable import BlackBox_Audio_Recorder

// MARK: - Channel Spec Conversion Tests

final class ChannelSpecTests: XCTestCase {

    // MARK: - channelSpecToZeroBased

    func testSingleChannelToZeroBased() {
        XCTAssertEqual(channelSpecToZeroBased("1"), "0")
        XCTAssertEqual(channelSpecToZeroBased("5"), "4")
    }

    func testMultipleChannelsToZeroBased() {
        XCTAssertEqual(channelSpecToZeroBased("1,3,5"), "0,2,4")
    }

    func testRangeToZeroBased() {
        XCTAssertEqual(channelSpecToZeroBased("1-4"), "0-3")
        XCTAssertEqual(channelSpecToZeroBased("3-5"), "2-4")
    }

    func testMixedSpecToZeroBased() {
        XCTAssertEqual(channelSpecToZeroBased("1,3-5,8"), "0,2-4,7")
    }

    func testEmptySpecToZeroBased() {
        XCTAssertEqual(channelSpecToZeroBased(""), "")
    }

    // MARK: - channelSpecToOneBased

    func testSingleChannelToOneBased() {
        XCTAssertEqual(channelSpecToOneBased("0"), "1")
        XCTAssertEqual(channelSpecToOneBased("4"), "5")
    }

    func testMultipleChannelsToOneBased() {
        XCTAssertEqual(channelSpecToOneBased("0,2,4"), "1,3,5")
    }

    func testRangeToOneBased() {
        XCTAssertEqual(channelSpecToOneBased("0-3"), "1-4")
    }

    func testMixedSpecToOneBased() {
        XCTAssertEqual(channelSpecToOneBased("0,2-4,7"), "1,3-5,8")
    }

    func testEmptySpecToOneBased() {
        XCTAssertEqual(channelSpecToOneBased(""), "")
    }

    // MARK: - Round-trip

    func testZeroBasedRoundTrip() {
        let original = "1,3-5,8"
        let zeroBased = channelSpecToZeroBased(original)
        let backToOne = channelSpecToOneBased(zeroBased)
        XCTAssertEqual(backToOne, original)
    }

    func testOneBasedRoundTrip() {
        let original = "0,2-4,7"
        let oneBased = channelSpecToOneBased(original)
        let backToZero = channelSpecToZeroBased(oneBased)
        XCTAssertEqual(backToZero, original)
    }

    // MARK: - countChannels

    func testCountSingleChannel() {
        XCTAssertEqual(countChannels("1"), 1)
    }

    func testCountMultipleChannels() {
        XCTAssertEqual(countChannels("1,3,5"), 3)
    }

    func testCountRange() {
        XCTAssertEqual(countChannels("1-4"), 4)
    }

    func testCountMixedSpec() {
        XCTAssertEqual(countChannels("1,3-5,8"), 5)
    }

    func testCountEmptySpec() {
        XCTAssertEqual(countChannels(""), 0)
    }

    func testCountDuplicatesDeduped() {
        // "1,1" has duplicates — Set-based counting deduplicates
        XCTAssertEqual(countChannels("1,1"), 1)
    }

    func testCountOverlappingRange() {
        // "1-3,2-4" overlaps — Set deduplicates
        XCTAssertEqual(countChannels("1-3,2-4"), 4)
    }

    func testCountWithWhitespace() {
        XCTAssertEqual(countChannels(" 1 , 3 - 5 , 8 "), 5)
    }

    // MARK: - isLegacyZeroBasedSpec

    func testZeroBasedSingleChannel() {
        XCTAssertTrue(isLegacyZeroBasedSpec("0"))
        XCTAssertTrue(isLegacyZeroBasedSpec("0,2,4"))
    }

    func testZeroBasedRange() {
        XCTAssertTrue(isLegacyZeroBasedSpec("0-3"))
    }

    func testOneBasedIsNotLegacy() {
        XCTAssertFalse(isLegacyZeroBasedSpec("1"))
        XCTAssertFalse(isLegacyZeroBasedSpec("1,3-5,8"))
    }

    func testMixedWithZeroIsLegacy() {
        // If any channel is 0, it's legacy
        XCTAssertTrue(isLegacyZeroBasedSpec("0,3,5"))
    }

    func testEmptySpecIsNotLegacy() {
        XCTAssertFalse(isLegacyZeroBasedSpec(""))
    }
}

// MARK: - BlackBoxError Tests

final class BlackBoxErrorTests: XCTestCase {

    func testKnownErrorCodes() {
        XCTAssertEqual(BlackBoxError(code: 0), .ok)
        XCTAssertEqual(BlackBoxError(code: -1), .invalidHandle)
        XCTAssertEqual(BlackBoxError(code: -2), .audioDevice)
        XCTAssertEqual(BlackBoxError(code: -3), .config)
        XCTAssertEqual(BlackBoxError(code: -4), .io)
        XCTAssertEqual(BlackBoxError(code: -5), .lockPoisoned)
        XCTAssertEqual(BlackBoxError(code: -6), .internal)
        XCTAssertEqual(BlackBoxError(code: -99), .unknown)
    }

    func testUnknownCodeFallsToUnknown() {
        XCTAssertEqual(BlackBoxError(code: 42), .unknown)
        XCTAssertEqual(BlackBoxError(code: -50), .unknown)
    }

    func testIsSuccess() {
        XCTAssertTrue(BlackBoxError.ok.isSuccess)
        XCTAssertFalse(BlackBoxError.invalidHandle.isSuccess)
        XCTAssertFalse(BlackBoxError.audioDevice.isSuccess)
        XCTAssertFalse(BlackBoxError.config.isSuccess)
        XCTAssertFalse(BlackBoxError.io.isSuccess)
        XCTAssertFalse(BlackBoxError.lockPoisoned.isSuccess)
        XCTAssertFalse(BlackBoxError.internal.isSuccess)
        XCTAssertFalse(BlackBoxError.unknown.isSuccess)
    }
}

// MARK: - RustBridge Tests

final class RustBridgeTests: XCTestCase {

    func testCreateWithDefaultConfig() {
        let bridge = RustBridge()
        XCTAssertFalse(bridge.isRecording)
        XCTAssertFalse(bridge.isMonitoring)
    }

    func testCreateWithCustomConfig() {
        let bridge = RustBridge(config: ["output_mode": "single"])
        let config = bridge.getConfig()
        XCTAssertNotNil(config)
        XCTAssertEqual(config?["output_mode"] as? String, "single")
    }

    func testSetAndGetConfig() {
        let bridge = RustBridge()
        bridge.setConfig(["output_mode": "split", "continuous_mode": true])
        let config = bridge.getConfig()
        XCTAssertNotNil(config)
        XCTAssertEqual(config?["output_mode"] as? String, "split")
        XCTAssertEqual(config?["continuous_mode"] as? Bool, true)
    }

    func testSetConfigPartialUpdate() {
        let bridge = RustBridge()
        bridge.setConfig(["output_mode": "single"])
        bridge.setConfig(["continuous_mode": true])
        let config = bridge.getConfig()
        // Both settings should persist
        XCTAssertEqual(config?["output_mode"] as? String, "single")
        XCTAssertEqual(config?["continuous_mode"] as? Bool, true)
    }

    func testSetConfigBitDepth() {
        let bridge = RustBridge()
        for depth in [16, 24, 32] {
            bridge.setConfig(["bits_per_sample": depth])
            let config = bridge.getConfig()
            XCTAssertEqual(config?["bits_per_sample"] as? Int, depth)
        }
    }

    func testLastErrorNilInitially() {
        let bridge = RustBridge()
        XCTAssertNil(bridge.lastError)
    }

    func testFillPeakLevelsWhenNotRecording() {
        let bridge = RustBridge()
        var buffer = [Float](repeating: 0, count: 255)
        let count = bridge.fillPeakLevels(into: &buffer)
        XCTAssertEqual(count, 0)
    }

    func testListInputDevices() throws {
        // CoreAudio device enumeration hangs on CI runners with no audio hardware
        try XCTSkipIf(
            ProcessInfo.processInfo.environment["CI"] != nil,
            "Skipping — no audio hardware on CI"
        )
        let devices = RustBridge.listInputDevices()
        XCTAssertNotNil(devices)
        // Type check — should be [String]
        XCTAssertTrue(type(of: devices) == [String].self)
    }

    func testGetStatusFlagsWhenIdle() {
        let bridge = RustBridge()
        let flags = bridge.getStatusFlags()
        XCTAssertNotNil(flags)
        if let flags {
            XCTAssertFalse(flags.is_recording)
            XCTAssertFalse(flags.stream_error)
            XCTAssertFalse(flags.disk_space_low)
            XCTAssertFalse(flags.sample_rate_changed)
            XCTAssertEqual(flags.write_errors, 0)
        }
    }

    func testStopRecordingWhenNotRecording() {
        let bridge = RustBridge()
        // Should succeed (no-op) rather than crash
        let result = bridge.stopRecording()
        XCTAssertTrue(result.isSuccess)
    }
}

// MARK: - AppDelegate Tests

final class AppDelegateTests: XCTestCase {

    func testExplicitQuitDefaultsFalse() {
        let delegate = AppDelegate()
        XCTAssertFalse(delegate.explicitQuit)
    }

    func testShouldNotTerminateAfterLastWindowClosed() {
        let delegate = AppDelegate()
        XCTAssertFalse(delegate.applicationShouldTerminateAfterLastWindowClosed(NSApplication.shared))
    }

    func testTerminateCancelledWithoutExplicitQuit() {
        let delegate = AppDelegate()
        delegate.explicitQuit = false
        let reply = delegate.applicationShouldTerminate(NSApplication.shared)
        XCTAssertEqual(reply, .terminateCancel)
    }

    func testTerminateAllowedWithExplicitQuit() {
        let delegate = AppDelegate()
        delegate.explicitQuit = true
        let reply = delegate.applicationShouldTerminate(NSApplication.shared)
        XCTAssertEqual(reply, .terminateNow)
    }
}

// MARK: - Settings Keys Completeness Tests

final class SettingsKeysTests: XCTestCase {

    /// Verify all known settings keys have the expected string values.
    /// Catches accidental renames that would orphan stored UserDefaults.
    func testAllKeyValues() {
        XCTAssertEqual(SettingsKeys.inputDevice, "inputDevice")
        XCTAssertEqual(SettingsKeys.audioChannels, "audioChannels")
        XCTAssertEqual(SettingsKeys.outputMode, "outputMode")
        XCTAssertEqual(SettingsKeys.silenceEnabled, "silenceEnabled")
        XCTAssertEqual(SettingsKeys.silenceThreshold, "silenceThreshold")
        XCTAssertEqual(SettingsKeys.continuousMode, "continuousMode")
        XCTAssertEqual(SettingsKeys.recordingCadence, "recordingCadence")
        XCTAssertEqual(SettingsKeys.launchAtLogin, "launchAtLogin")
        XCTAssertEqual(SettingsKeys.autoRecord, "autoRecord")
        XCTAssertEqual(SettingsKeys.minDiskSpaceMB, "minDiskSpaceMB")
        XCTAssertEqual(SettingsKeys.hasCompletedOnboarding, "hasCompletedOnboarding")
        XCTAssertEqual(SettingsKeys.bitDepth, "bitDepth")
        XCTAssertEqual(SettingsKeys.lastOutputDirPath, "lastOutputDirPath")
        XCTAssertEqual(SettingsKeys.silenceGateEnabled, "silenceGateEnabled")
        XCTAssertEqual(SettingsKeys.silenceGateTimeout, "silenceGateTimeout")
        XCTAssertEqual(SettingsKeys.sleepBehavior, "sleepBehavior")
        XCTAssertEqual(SettingsKeys.preventSleep, "preventSleep")
    }
}
