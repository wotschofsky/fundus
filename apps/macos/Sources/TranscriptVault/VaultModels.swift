import Foundation

/// A recording in the vault, as listed by `/api/v1/entries`. Detail reads add speakers and sources.
struct Entry: Decodable, Identifiable, Hashable, Sendable {
    let id: String
    let title: String
    let recordedAt: Date?
    let createdAt: Date
    let durationMs: Int
    let timezone: String
    let stage: String
    let activeStage: String?
    let failure: String?
    let pauseRequested: Bool
    let labels: [String]
    let summary: String?
    let warnings: [String]
    let audioAvailable: Bool
    let trashedAt: Date?
    var speakers: [Speaker]?
    var sources: [Source]?
    var trimStartFrame: Int64?

    struct Source: Decodable, Hashable, Sendable {
        let offsetMs: Int
        let recordedAt: Date?
        let filename: String
        let format: Format?
        struct Format: Decodable, Hashable, Sendable { let sampleRate: Int }
    }

    var isProcessing: Bool { !["complete", "failed", "paused"].contains(stage) }
    var isComplete: Bool { stage == "complete" }
    var canResume: Bool { stage == "failed" || stage == "paused" }
    var date: Date { recordedAt ?? createdAt }
    var duration: TimeInterval { Double(durationMs) / 1000 }
    /// Times are shown in this Mac's zone, as in Mic Ingest. `timezone` is only the recorder clock
    /// the recording was imported with (e.g. the DJI factory UTC+8), not where the person is.
    var zone: TimeZone { .current }

    var displayTitle: String {
        let trimmed = title.trimmingCharacters(in: .whitespacesAndNewlines)
        // Until a summary names it, a recording is known by when it was made.
        return trimmed.isEmpty || trimmed.hasPrefix("Recording · ") ? dateLine : trimmed
    }

    /// `Oct 24 · 14:05`, in the recording's own zone; undated recordings say so.
    var dateLine: String {
        recordedAt.map { "\(Format.day($0, in: zone)) · \(SessionTimeline.clock($0, in: zone, seconds: false))" } ?? "Date unknown"
    }

    var status: String { Self.describe(stage: stage, activeStage: activeStage, pauseRequested: pauseRequested) }

    /// What the server is doing with a recording, in words.
    static func describe(stage: String, activeStage: String?, pauseRequested: Bool?) -> String {
        if pauseRequested == true { return "Pausing after this step…" }
        switch activeStage ?? stage {
        case "uploading": return "Uploading"
        case "queued": return "Waiting to process"
        case "preparing": return "Preparing audio"
        case "transcribing": return "Transcribing"
        case "recognizing": return "Recognizing speakers"
        case "enriching": return "Writing summary"
        case "indexing": return "Indexing for search"
        case "paused": return "Paused"
        case "failed": return "Processing stopped"
        case "complete": return "Ready"
        case let other: return other.capitalized
        }
    }
}

struct Speaker: Decodable, Hashable, Sendable {
    let label: String
    let personId: String?
    let name: String?
    let state: String
    let evidence: [Evidence]

    struct Evidence: Decodable, Hashable, Sendable {
        let source: String
        let name: String?
        let quote: String?
        let applied: Bool?
    }

    var isIdentified: Bool { personId != nil }
    var keptUnidentified: Bool { state == "manual_unknown" }

    /// Why the speaker is (or is not) named. Shown on demand in the speaker menu.
    var explanations: [String] {
        if state == "manual" { return ["Assigned by you"] }
        return evidence.compactMap { item in
            // Evidence saved before `applied` existed was always applied.
            let applied = item.applied ?? true
            if item.source == "voice" { return applied ? "Voice matches a saved profile" : "Voice resembles a saved profile" }
            if let quote = item.quote, !quote.isEmpty {
                return applied ? "Introduced as \(item.name ?? "named"): “\(quote)”" : "Possibly \(item.name ?? "named"): “\(quote)”"
            }
            return nil
        }
    }
}

struct Segment: Decodable, Identifiable, Hashable, Sendable {
    let id: String
    let speaker: String
    let startMs: Int
    let endMs: Int
    let text: String
}

struct Person: Decodable, Identifiable, Hashable, Sendable {
    let id: String
    let name: String
    let samples: Int

    private enum CodingKeys: String, CodingKey { case id, name, samples }

    /// Creating a person answers with just its id and name.
    init(from decoder: Decoder) throws {
        let container = try decoder.container(keyedBy: CodingKeys.self)
        id = try container.decode(String.self, forKey: .id)
        name = try container.decode(String.self, forKey: .name)
        samples = try container.decodeIfPresent(Int.self, forKey: .samples) ?? 0
    }
}

struct ProcessingSettings: Codable, Sendable {
    var recordingContext: String
    var voiceThresholds: Thresholds
    var models: [String: String]?
    var voiceConfigured: Bool?

    struct Thresholds: Codable, Equatable, Sendable {
        var strong: Double
        var margin: Double
        static let `default` = Thresholds(strong: 0.78, margin: 0.10)
    }
}

/// Server-side record of what an import became, used to mark sessions on the recorder as imported.
struct SourceHistoryEntry: Decodable, Sendable {
    let id: String
    let stage: String
    let activeStage: String?
    let failure: String?
    let pauseRequested: Bool?
    let trimStartFrame: Int64?
    let trimEndFrame: Int64?
    let sourceIds: [String?]
}

struct ImportReceipt: Decodable, Sendable {
    /// `uploading`, `verifying`, `complete` or `failed`.
    let state: String
    let entries: [Entry]
    struct Entry: Decodable, Sendable {
        let id: String
        let sessionIndex: Int
        let stage: String
        let activeStage: String?
        let failure: String?
        let pauseRequested: Bool?
    }
}

struct Page<Item: Decodable & Sendable>: Decodable, Sendable {
    let items: [Item]
    let nextCursor: Int?

    private struct Key: CodingKey {
        var stringValue: String
        var intValue: Int? { nil }
        init(stringValue: String) { self.stringValue = stringValue }
        init?(intValue: Int) { nil }
    }

    /// Pages name their array after what they contain (`entries`, `segments`, …).
    init(from decoder: Decoder) throws {
        let container = try decoder.container(keyedBy: Key.self)
        nextCursor = try container.decodeIfPresent(Int.self, forKey: Key(stringValue: "nextCursor"))
        let key = container.allKeys.first { ["entries", "segments", "people", "labels"].contains($0.stringValue) }
        items = try key.map { try container.decode([Item].self, forKey: $0) } ?? []
    }
}
