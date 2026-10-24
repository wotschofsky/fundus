import Foundation

struct SessionTimeline: Sendable {
    struct Chunk: Sendable {
        var offsetSeconds: Double
        var start: Date
    }

    var chunks: [Chunk]
    var timeZone: TimeZone

    func date(atMs milliseconds: Int) -> Date {
        let seconds = Double(milliseconds) / 1000
        let chunk = chunks.last { $0.offsetSeconds <= seconds } ?? chunks.first
        guard let chunk else { return Date(timeIntervalSince1970: seconds) }
        return chunk.start.addingTimeInterval(seconds - chunk.offsetSeconds)
    }

    /// `20:44:03`, in the session's time zone.
    func clockTime(atMs milliseconds: Int) -> String {
        Self.clock(date(atMs: milliseconds), in: timeZone)
    }

    static func clock(_ date: Date, in timeZone: TimeZone, seconds: Bool = true) -> String {
        var calendar = Calendar(identifier: .gregorian)
        calendar.timeZone = timeZone
        let parts = calendar.dateComponents([.hour, .minute, .second], from: date)
        return seconds
            ? String(format: "%02d:%02d:%02d", parts.hour ?? 0, parts.minute ?? 0, parts.second ?? 0)
            : String(format: "%02d:%02d", parts.hour ?? 0, parts.minute ?? 0)
    }
}
