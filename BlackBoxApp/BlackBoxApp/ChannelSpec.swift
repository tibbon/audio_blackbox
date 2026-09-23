import Foundation

// The channel-spec helpers below are `nonisolated`: pure string parsing with no
// shared state, called from main-actor code and from nonisolated unit tests alike.

/// Highest 1-based channel number the engine accepts (Rust `MAX_CHANNELS` is
/// 255, and its 0-based channels run 0...254).
nonisolated let maxChannelNumber = 255

/// Parse a 1-based channel spec ("1-8, 16, 24-32") the way the engine will:
/// the sorted, de-duplicated channel numbers, or an empty array (a valid spec
/// always names at least one channel) when the spec would
/// make `blackbox_start_recording` fail — an empty spec, an empty or
/// malformed token ("1-", "a", "1,,2"), a reversed range, or a channel
/// outside `1...maxChannelNumber`.
nonisolated func parseChannelSpec(_ spec: String) -> [Int] {
    var channels = Set<Int>()
    for part in spec.split(separator: ",", omittingEmptySubsequences: false) {
        let token = part.trimmingCharacters(in: .whitespaces)
        let bounds = token.split(separator: "-", omittingEmptySubsequences: false)
            .map { Int($0.trimmingCharacters(in: .whitespaces)) }
        switch bounds.count {
        case 1:
            guard let channel = bounds[0], (1...maxChannelNumber).contains(channel) else { return [] }
            channels.insert(channel)

        case 2:
            guard let start = bounds[0], let end = bounds[1],
                start >= 1, start <= end, end <= maxChannelNumber
            else { return [] }
            channels.formUnion(start...end)

        default:
            return []
        }
    }
    return channels.sorted()
}

/// The 1-based device channels a session records, in the order the engine
/// publishes their peak levels. Mirrors `recording_channels` in
/// src/cpal_processor.rs: the requested channels the device has, ascending
/// (the engine's parser sorts and de-duplicates), or every device channel
/// when it has none of them. An empty or invalid spec is the engine's
/// default, channel 1.
nonisolated func recordedChannelNumbers(spec: String, deviceChannelCount: Int) -> [Int] {
    guard deviceChannelCount > 0 else { return [] }
    let requested = parseChannelSpec(spec)
    let kept = (requested.isEmpty ? [1] : requested).filter { $0 <= deviceChannelCount }
    return kept.isEmpty ? Array(1...deviceChannelCount) : kept
}

/// Count the number of unique channels in a 1-based spec string (e.g. "1,3-5,8" → 5).
nonisolated func countChannels(_ spec: String) -> Int {
    var channels = Set<Int>()
    for part in spec.split(separator: ",") {
        let token = part.trimmingCharacters(in: .whitespaces)
        if token.contains("-") {
            let bounds = token.split(separator: "-")
            if bounds.count == 2,
                let start = Int(bounds[0].trimmingCharacters(in: .whitespaces)),
                let end = Int(bounds[1].trimmingCharacters(in: .whitespaces)),
                start >= 1, end >= start
            {
                for ch in start...end { channels.insert(ch) }
            }
        } else if let num = Int(token), num >= 1 {
            channels.insert(num)
        }
    }
    return channels.count
}

/// "1 channel" / "8 channels", localized. The String Catalog carries plural
/// variations for this key, so languages with other plural rules get theirs.
nonisolated func channelCountLabel(_ count: Int) -> String {
    String(localized: "\(count) channels", comment: "Number of recorded channels, e.g. in the menu-bar menu")
}

/// Convert a 1-based channel spec string to 0-based for the Rust engine.
/// e.g. "1,3-5,8" → "0,2-4,7"
nonisolated func channelSpecToZeroBased(_ spec: String) -> String {
    spec.split(separator: ",")
        .map { part in
            let token = part.trimmingCharacters(in: .whitespaces)
            if token.contains("-") {
                let bounds = token.split(separator: "-")
                if bounds.count == 2,
                    let start = Int(bounds[0].trimmingCharacters(in: .whitespaces)),
                    let end = Int(bounds[1].trimmingCharacters(in: .whitespaces))
                {
                    return "\(start - 1)-\(end - 1)"
                }
                return token
            }
            if let num = Int(token) {
                return "\(num - 1)"
            }
            return token
        }
        .joined(separator: ",")
}

/// Convert a 0-based channel spec string to 1-based for the UI.
/// e.g. "0,2-4,7" → "1,3-5,8"
nonisolated func channelSpecToOneBased(_ spec: String) -> String {
    spec.split(separator: ",")
        .map { part in
            let token = part.trimmingCharacters(in: .whitespaces)
            if token.contains("-") {
                let bounds = token.split(separator: "-")
                if bounds.count == 2,
                    let start = Int(bounds[0].trimmingCharacters(in: .whitespaces)),
                    let end = Int(bounds[1].trimmingCharacters(in: .whitespaces))
                {
                    return "\(start + 1)-\(end + 1)"
                }
                return token
            }
            if let num = Int(token) {
                return "\(num + 1)"
            }
            return token
        }
        .joined(separator: ",")
}

/// Check if a channel spec uses legacy 0-based numbering (contains a "0" channel).
nonisolated func isLegacyZeroBasedSpec(_ spec: String) -> Bool {
    for part in spec.split(separator: ",") {
        let token = part.trimmingCharacters(in: .whitespaces)
        if token.contains("-") {
            let bounds = token.split(separator: "-")
            if let start = Int(bounds.first?.trimmingCharacters(in: .whitespaces) ?? ""),
                start == 0
            {
                return true
            }
        } else if let num = Int(token), num == 0 {
            return true
        }
    }
    return false
}
