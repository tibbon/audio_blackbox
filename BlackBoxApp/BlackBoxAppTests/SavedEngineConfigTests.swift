import XCTest

@testable import BlackBox_Audio_Recorder

/// Launch-time settings restore, run against a throwaway defaults suite so
/// the app's real preferences are never read or written.
nonisolated final class SavedEngineConfigTests: XCTestCase {
    private var suiteName = ""
    private var defaults = UserDefaults()

    override func setUp() {
        super.setUp()
        suiteName = "SavedEngineConfigTests.\(UUID().uuidString)"
        defaults = UserDefaults(suiteName: suiteName) ?? UserDefaults()
    }

    override func tearDown() {
        defaults.removePersistentDomain(forName: suiteName)
        super.tearDown()
    }

    /// "Disabled" (0) is a saved choice and must reach the engine; the old
    /// `> 0` check dropped it, so the engine used its 500 MB default.
    func testDisabledMinFreeSpaceIsRestored() {
        defaults.set(0, forKey: SettingsKeys.minDiskSpaceMB)
        let config = SavedEngineConfig.restore(from: defaults)
        XCTAssertEqual(config["min_disk_space_mb"] as? Int, 0)
    }

    func testSavedMinFreeSpaceIsRestored() {
        defaults.set(2000, forKey: SettingsKeys.minDiskSpaceMB)
        let config = SavedEngineConfig.restore(from: defaults)
        XCTAssertEqual(config["min_disk_space_mb"] as? Int, 2000)
    }

    func testUnsetMinFreeSpaceLeavesTheEngineDefault() {
        let config = SavedEngineConfig.restore(from: defaults)
        XCTAssertNil(config["min_disk_space_mb"])
    }

    func testChannelSpecIsSentZeroBased() {
        defaults.set("1,3-4", forKey: SettingsKeys.audioChannels)
        let config = SavedEngineConfig.restore(from: defaults)
        XCTAssertEqual(config["audio_channels"] as? String, "0,2-3")
    }

    func testLegacyZeroBasedSpecIsMigrated() {
        defaults.set("0,2", forKey: SettingsKeys.audioChannels)
        let config = SavedEngineConfig.restore(from: defaults)
        XCTAssertEqual(config["audio_channels"] as? String, "0,2")
        XCTAssertEqual(defaults.string(forKey: SettingsKeys.audioChannels), "1,3")
    }
}
