import Foundation

// The channel-spec helpers below are `nonisolated`: pure string parsing with no
// shared state, called from main-actor code and from nonisolated unit tests alike.

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
