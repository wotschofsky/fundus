import Foundation

/// The part of a session to keep, in seconds from the session's first sample. Only the ends can be
/// trimmed; nothing in between is cut.
struct Trim: Codable, Hashable {
    var start: Double
    var end: Double

    var duration: Double { end - start }

    /// Sample-exact frame range over the session's concatenated audio.
    func frames(sampleRate: Double, totalFrames: Int64) -> Range<Int64> {
        let first = min(max(0, Int64((start * sampleRate).rounded())), totalFrames)
        let last = min(max(first, Int64((end * sampleRate).rounded())), totalFrames)
        return first..<last
    }
}

extension RecordingSession {
    /// Positions in the session (seconds from its first sample) to clock times, per source file.
    var timeline: SessionTimeline {
        var offset = 0.0
        return SessionTimeline(chunks: recordings.map { recording in
            defer { offset += recording.duration }
            return .init(offsetSeconds: offset, start: recording.start)
        }, timeZone: .current)
    }

    func start(trimmedBy trim: Trim?) -> Date {
        trim.map { timeline.date(atMs: Int(($0.start * 1000).rounded())) } ?? start
    }

    func end(trimmedBy trim: Trim?) -> Date {
        trim.map { timeline.date(atMs: Int(($0.end * 1000).rounded())) } ?? end
    }
}
