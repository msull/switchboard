import Foundation

/// How long ago a host's millisecond timestamp was, in a few
/// characters: `now`, `4m`, `2h`, `3d`.
enum Ago {
    static func text(_ ms: UInt64, now: Date = Date()) -> String {
        guard ms > 0 else { return "" }
        let then = Date(timeIntervalSince1970: TimeInterval(ms) / 1000)
        let seconds = max(0, now.timeIntervalSince(then))
        if seconds < 60 { return "now" }
        if seconds < 3600 { return "\(Int(seconds / 60))m" }
        if seconds < 86400 { return "\(Int(seconds / 3600))h" }
        return "\(Int(seconds / 86400))d"
    }
}
