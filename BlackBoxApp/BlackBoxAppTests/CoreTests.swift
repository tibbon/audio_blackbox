import BlackBoxFFI
import Synchronization
import XCTest

@testable import BlackBox_Audio_Recorder

// Test cases opt out of the module's MainActor default: XCTest's inherited
// initializers and setUp/tearDown are nonisolated, and an isolated subclass
// can't override them. Tests that touch main-actor types are marked @MainActor.

// MARK: - Channel Spec Conversion Tests

nonisolated final class ChannelSpecTests: XCTestCase {
    // MARK: - channelCountLabel

    /// The catalog's plural variations pick the singular for one channel.
    func testChannelCountLabelIsPluralAware() {
        XCTAssertEqual(channelCountLabel(1), "1 channel")
        XCTAssertEqual(channelCountLabel(2), "2 channels")
        XCTAssertEqual(channelCountLabel(0), "0 channels")
    }

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

    // MARK: - parseChannelSpec (strict, mirrors the engine's parser)

    func testParseChannelSpecAcceptsListsAndRanges() {
        XCTAssertEqual(parseChannelSpec("1"), [1])
        XCTAssertEqual(parseChannelSpec(" 3 - 5 , 1, 4 "), [1, 3, 4, 5])
        XCTAssertEqual(parseChannelSpec("255"), [255])
    }

    func testParseChannelSpecRejectsWhatTheEngineRejects() {
        for spec in ["", " ", "1-", "-3", "1,,2", "1,", "a", "5-3", "0", "256", "1-256", "1-2-3"] {
            XCTAssertEqual(parseChannelSpec(spec), [], "\"\(spec)\" must be rejected")
        }
    }

    // MARK: - recordedChannelNumbers (meter labels, mirrors recording_channels)

    /// Selecting channels 3 and 4 records two channels; their meter bars are
    /// device channels 3 and 4, not positions 1 and 2.
    func testRecordedChannelsAreTheSelectedDeviceChannels() {
        XCTAssertEqual(recordedChannelNumbers(spec: "3-4", deviceChannelCount: 8), [3, 4])
        XCTAssertEqual(recordedChannelNumbers(spec: "7, 2", deviceChannelCount: 8), [2, 7])
    }

    func testChannelsTheDeviceLacksAreDropped() {
        XCTAssertEqual(recordedChannelNumbers(spec: "2,5,9", deviceChannelCount: 6), [2, 5])
    }

    /// Like the engine: none of the requested channels exist, so it records
    /// every device channel.
    func testNoAvailableChannelFallsBackToAll() {
        XCTAssertEqual(recordedChannelNumbers(spec: "9-10", deviceChannelCount: 2), [1, 2])
    }

    func testInvalidSpecIsChannelOne() {
        XCTAssertEqual(recordedChannelNumbers(spec: "1-", deviceChannelCount: 4), [1])
        XCTAssertEqual(recordedChannelNumbers(spec: "1", deviceChannelCount: 0), [])
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

nonisolated final class BlackBoxErrorTests: XCTestCase {
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

nonisolated final class RustBridgeTests: XCTestCase {
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
        // DOLL-125: fillPeakLevels now returns Result<Int, BlackBoxError>;
        // when not recording, the FFI returns 0 (success) — not an error.
        let result = bridge.fillPeakLevels(into: &buffer)
        switch result {
        case .success(let count):
            XCTAssertEqual(count, 0)

        case .failure(let err):
            XCTFail("expected success(0) when not recording, got error \(err)")
        }
    }

    func testListInputDevices() async throws {
        // CoreAudio device enumeration hangs indefinitely on machines with
        // no audio hardware (CI runners). Race it against a timeout instead
        // of env var detection — xcodebuild test host doesn't inherit shell
        // env vars. The enumeration runs in an unstructured Task off the main
        // actor so a hung CoreAudio call can't pin the test.
        let enumerated = expectation(description: "CoreAudio device enumeration")
        // swiftlint:disable:next discouraged_optional_collection - nil means enumeration has not completed yet
        let devices = Mutex<[String]?>(nil)
        Task { @concurrent in
            let list = RustBridge.listInputDevices()
            devices.withLock { $0 = list }
            enumerated.fulfill()
        }

        if await XCTWaiter().fulfillment(of: [enumerated], timeout: 5) == .timedOut {
            throw XCTSkip("CoreAudio device enumeration timed out — no audio hardware")
        }

        let list = try XCTUnwrap(devices.withLock { $0 })
        XCTAssertTrue(type(of: list) == [String].self)
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

nonisolated final class AppDelegateTests: XCTestCase {
    @MainActor
    func testExplicitQuitDefaultsFalse() {
        let delegate = AppDelegate()
        XCTAssertFalse(delegate.explicitQuit)
    }

    @MainActor
    func testShouldNotTerminateAfterLastWindowClosed() {
        let delegate = AppDelegate()
        XCTAssertFalse(delegate.applicationShouldTerminateAfterLastWindowClosed(NSApplication.shared))
    }

    @MainActor
    func testTerminateCancelledWithoutExplicitQuit() {
        let delegate = AppDelegate()
        delegate.explicitQuit = false
        let reply = delegate.applicationShouldTerminate(NSApplication.shared)
        XCTAssertEqual(reply, .terminateCancel)
    }

    /// willPowerOff must set explicitQuit inside the post: AppKit can ask
    /// applicationShouldTerminate before a later main-actor turn, and a
    /// deferred handler then vetoed logout / shutdown.
    @MainActor
    func testWillPowerOffAllowsTerminationBeforeThePostReturns() {
        let delegate = AppDelegate()
        let workspace = NotificationCenter()
        delegate.installSystemObservers(workspaceCenter: workspace, appCenter: NotificationCenter())

        workspace.post(name: NSWorkspace.willPowerOffNotification, object: nil)

        XCTAssertTrue(delegate.explicitQuit)
        XCTAssertEqual(delegate.applicationShouldTerminate(NSApplication.shared), .terminateNow)
        delegate.applicationWillTerminate(Notification(name: NSApplication.willTerminateNotification))
    }

    /// willSleep must stop (finalize) the recording before the post returns;
    /// the Mac can be asleep before a later main-actor turn.
    @MainActor
    func testWillSleepStopsTheRecordingBeforeThePostReturns() {
        let delegate = AppDelegate()
        let recorder = RecordingState()
        delegate.recorder = recorder
        let workspace = NotificationCenter()
        delegate.installSystemObservers(workspaceCenter: workspace, appCenter: NotificationCenter())
        recorder.isRecording = true

        workspace.post(name: NSWorkspace.willSleepNotification, object: nil)

        XCTAssertFalse(recorder.isRecording)
        recorder.cancelPendingResume()
        delegate.applicationWillTerminate(Notification(name: NSApplication.willTerminateNotification))
    }

    @MainActor
    func testTerminateAllowedWithExplicitQuit() {
        let delegate = AppDelegate()
        delegate.explicitQuit = true
        let reply = delegate.applicationShouldTerminate(NSApplication.shared)
        XCTAssertEqual(reply, .terminateNow)
    }

    private func appleEvent(_ eventClass: AEEventClass, _ eventID: AEEventID) -> NSAppleEventDescriptor {
        NSAppleEventDescriptor(
            eventClass: eventClass,
            eventID: eventID,
            targetDescriptor: nil,
            returnID: AEReturnID(kAutoGenerateReturnID),
            transactionID: AETransactionID(kAnyTransactionID)
        )
    }

    /// A quit Apple event (Activity Monitor, an installer, osascript) is a
    /// real request to quit: finalize the recording and terminate instead of
    /// treating it as SwiftUI's last-window terminate and cancelling it.
    @MainActor
    func testQuitAppleEventFinalizesTheRecordingAndTerminates() {
        let delegate = AppDelegate()
        let recorder = RecordingState()
        delegate.recorder = recorder
        recorder.isRecording = true

        let reply = delegate.shouldTerminate(currentAppleEvent: appleEvent(kCoreEventClass, kAEQuitApplication))

        XCTAssertEqual(reply, .terminateNow)
        XCTAssertFalse(recorder.isRecording)
    }

    /// Any other Apple event (or none, as for SwiftUI's own terminate call)
    /// keeps the old behavior: stay alive as a menu-bar app.
    @MainActor
    func testOtherAppleEventsDoNotAllowTermination() {
        let delegate = AppDelegate()
        XCTAssertEqual(delegate.shouldTerminate(currentAppleEvent: nil), .terminateCancel)
        XCTAssertEqual(
            delegate.shouldTerminate(currentAppleEvent: appleEvent(kCoreEventClass, kAEOpenApplication)),
            .terminateCancel
        )
        XCTAssertFalse(AppDelegate.isQuitAppleEvent(appleEvent(kCoreEventClass, kAEReopenApplication)))
    }
}

// MARK: - Settings Keys Completeness Tests

nonisolated final class SettingsKeysTests: XCTestCase {
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
        XCTAssertEqual(SettingsKeys.lastSampleRate, "lastSampleRate")
        XCTAssertEqual(SettingsKeys.debugLogging, "debugLogging")
        XCTAssertEqual(SettingsKeys.successfulRecordingSessions, "successfulRecordingSessions")
        XCTAssertEqual(SettingsKeys.hasPromptedForReview, "hasPromptedForReview")
        XCTAssertEqual(SettingsKeys.outputDirBookmark, "outputDirBookmark")
        XCTAssertEqual(SettingsKeys.globalShortcut, "globalShortcut")
    }
}

// MARK: - Plural strings

/// Counted messages pick the singular from the catalog's plural variations
/// instead of "recording(s)" or an English-only branch.
nonisolated final class PluralStringTests: XCTestCase {
    func testDroppedSamplesArePluralAware() {
        let count = 1
        XCTAssertEqual(String(localized: "\(count) samples dropped"), "1 sample dropped")
        XCTAssertEqual(
            String(localized: "Warning: \(count) samples dropped during this recording"),
            "Warning: 1 sample dropped during this recording"
        )
    }

    func testRecoveredRecordingsArePluralAware() {
        for (count, noun) in [(1, "recording that was"), (3, "recordings that were")] {
            XCTAssertEqual(
                String(
                    localized:
                        "BlackBox finished \(count) recordings that were interrupted by a crash or power loss."
                ),
                "BlackBox finished \(count) \(noun) interrupted by a crash or power loss."
            )
        }
    }
}

// MARK: - Rotation Cadence Presets

nonisolated final class CadencePresetsTests: XCTestCase {
    /// Every preset the picker offers reopens as itself, not as "Custom".
    /// The 30 s and 1 min presets used to fall through to "Custom".
    func testEveryPresetRoundTrips() {
        for preset in CadencePresets.all {
            XCTAssertEqual(CadencePresets.selection(for: preset.seconds), preset.seconds)
        }
        XCTAssertEqual(CadencePresets.selection(for: 30), 30)
        XCTAssertEqual(CadencePresets.selection(for: 60), 60)
    }

    /// Whole hours and minutes come from the catalog's plural variations.
    @MainActor
    func testCustomCadenceDescriptionIsPluralAware() {
        XCTAssertEqual(CustomCadenceField.cadenceDescription(3600), "1 hour")
        XCTAssertEqual(CustomCadenceField.cadenceDescription(7200), "2 hours")
        XCTAssertEqual(CustomCadenceField.cadenceDescription(60), "1 minute")
        XCTAssertEqual(CustomCadenceField.cadenceDescription(900), "15 minutes")
    }

    func testOtherValuesAreCustom() {
        XCTAssertEqual(CadencePresets.selection(for: 45), CadencePresets.custom)
        XCTAssertEqual(CadencePresets.selection(for: 86_400), CadencePresets.custom)
    }

    func testPresetTagsAreUniqueAndDistinctFromCustom() {
        XCTAssertFalse(CadencePresets.all.contains { $0.seconds == CadencePresets.custom })
        XCTAssertEqual(Set(CadencePresets.all.map(\.seconds)).count, CadencePresets.all.count)
    }
}
