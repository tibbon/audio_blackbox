import Foundation
import XCTest

/// Base class for tests that read or write `UserDefaults.standard`.
///
/// The test host is the app itself (same bundle identifier, same sandbox
/// container), so its standard defaults are the developer's real
/// preferences. Tests used to `removeObject` their keys in `tearDown`, which
/// deleted the real setting on every run. This snapshots the app's
/// persistent domain before each test and puts it back afterwards, so a test
/// can change whatever it needs and the real preferences survive.
///
/// `nonisolated` like the other test cases: XCTest's lifecycle methods are
/// nonisolated, and an isolated subclass can't override them.
nonisolated class StandardDefaultsTestCase: XCTestCase {
    // swiftlint:disable:next discouraged_optional_collection - nil means the app had no preferences at all, which tearDown restores by removing the domain
    private var savedDomain: [String: Any]?

    private var domainName: String {
        Bundle.main.bundleIdentifier ?? "com.dollhousemediatech.blackbox"
    }

    override func setUp() {
        super.setUp()
        savedDomain = UserDefaults.standard.persistentDomain(forName: domainName)
    }

    override func tearDown() {
        if let savedDomain {
            UserDefaults.standard.setPersistentDomain(savedDomain, forName: domainName)
        } else {
            UserDefaults.standard.removePersistentDomain(forName: domainName)
        }
        savedDomain = nil
        super.tearDown()
    }
}

// MARK: - Isolation check

nonisolated final class StandardDefaultsIsolationTests: XCTestCase {
    private static let probeKey = "com.dollhousemediatech.blackbox.tests.defaultsProbe"
    private static let addedKey = "com.dollhousemediatech.blackbox.tests.defaultsAdded"

    override func setUp() {
        super.setUp()
        UserDefaults.standard.removeObject(forKey: Self.probeKey)
        UserDefaults.standard.removeObject(forKey: Self.addedKey)
    }

    override func tearDown() {
        UserDefaults.standard.removeObject(forKey: Self.probeKey)
        UserDefaults.standard.removeObject(forKey: Self.addedKey)
        super.tearDown()
    }

    /// A value a StandardDefaultsTestCase test deletes comes back after its
    /// tearDown, and a key it adds goes away.
    func testTearDownRestoresThePreferencesATestChanged() {
        UserDefaults.standard.set("real", forKey: Self.probeKey)
        let probe = StandardDefaultsTestCase()

        probe.setUp()
        UserDefaults.standard.removeObject(forKey: Self.probeKey)
        UserDefaults.standard.set("scratch", forKey: Self.addedKey)
        probe.tearDown()

        XCTAssertEqual(UserDefaults.standard.string(forKey: Self.probeKey), "real")
        XCTAssertNil(UserDefaults.standard.object(forKey: Self.addedKey))
    }
}
