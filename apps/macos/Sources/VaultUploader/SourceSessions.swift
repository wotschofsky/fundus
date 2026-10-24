import CryptoKit
import Foundation
import Observation
import VaultNative

struct SessionImportRecord: Codable {
    var server: String
    var fingerprint: String
    var sources: [String]
    var sourceIDs: [String]? = nil
    var trimStartFrame: Int64? = nil
    var trimEndFrame: Int64? = nil
    var importID: String
    var sessionIndex: Int
    var entryID: String?
    var stage: String
    var activeStage: String?
    var failure: String?
    var pauseRequested: Bool? = nil
}

@MainActor @Observable
final class SourceSessions {
    private(set) var sessions: [RecordingSession] = []
    private(set) var problems: [SourceProblem] = []
    private(set) var records: [SessionImportRecord] = []
    private var selected: Set<String> = []
    private var recordings: [SourceRecording] = []
    private var overrides: [String: BoundaryOverride]
    private var trims: [String: Trim]
    private let defaults: UserDefaults
    private let grouper = SessionGrouper()
    var transmitterTab: String?
    var transmitters: [String] { Set(recordings.map(\.transmitter)).sorted() }
    var visibleTransmitter: String? { transmitterTab.flatMap { transmitters.contains($0) ? $0 : nil } ?? transmitters.first }
    var visibleSessions: [RecordingSession] { sessions.filter { $0.transmitter == visibleTransmitter } }
    var selectedSessions: [RecordingSession] { visibleSessions.filter(isSelected) }

    init(defaults: UserDefaults = .standard) {
        self.defaults = defaults
        overrides = defaults.data(forKey: "session-boundaries").flatMap { try? JSONDecoder().decode([String: BoundaryOverride].self, from: $0) } ?? [:]
        trims = defaults.data(forKey: "session-trims").flatMap { try? JSONDecoder().decode([String: Trim].self, from: $0) } ?? [:]
        records = defaults.data(forKey: "session-import-history").flatMap { try? JSONDecoder().decode([SessionImportRecord].self, from: $0) } ?? []
    }

    func update(_ files: [RecorderFile], server: String) {
        let old = Set(recordings.map(\.fingerprint))
        recordings = files.compactMap(\.recording)
        problems = files.compactMap { file in file.inspectionError.map { SourceProblem(filename: file.name, message: $0) } }
        sessions = grouper.group(recordings, overrides: overrides)
        selected.formIntersection(recordings.map(\.fingerprint))
        for session in sessions where session.duration >= 120 && record(for: session, server: server) == nil {
            let new = session.recordings.map(\.fingerprint).filter { !old.contains($0) }
            selected.formUnion(new)
        }
    }

    func isSelected(_ session: RecordingSession) -> Bool { session.recordings.allSatisfy { selected.contains($0.fingerprint) } }
    func setSelected(_ value: Bool, _ session: RecordingSession) {
        if value { selected.formUnion(session.recordings.map(\.fingerprint)) }
        else { selected.subtract(session.recordings.map(\.fingerprint)) }
    }
    func previous(of session: RecordingSession) -> RecordingSession? {
        sessions.last { $0.transmitter == session.transmitter && $0.recordings[0].variant == session.recordings[0].variant && $0.recordings[0].format == session.recordings[0].format && $0.start < session.start }
    }
    func mergeWithPrevious(_ session: RecordingSession) {
        guard let previous = previous(of: session), let last = previous.recordings.last, let first = session.recordings.first else { return }
        overrides[first.fingerprint] = grouper.decision(previous: last, next: first).joinsPrevious ? nil : .merge
        regroup()
    }
    func split(_ session: RecordingSession, before recording: SourceRecording) {
        guard let index = session.recordings.firstIndex(of: recording), index > 0 else { return }
        overrides[recording.fingerprint] = grouper.decision(previous: session.recordings[index-1], next: recording).joinsPrevious ? .split : nil
        regroup()
    }
    private func regroup() {
        sessions = grouper.group(recordings, overrides: overrides)
        defaults.set(try? JSONEncoder().encode(overrides), forKey: "session-boundaries")
    }
    func trim(for session: RecordingSession) -> Trim? { trims[session.fingerprint] }
    func setTrim(start: Double? = nil, end: Double? = nil, for session: RecordingSession) {
        var trim = trims[session.fingerprint] ?? Trim(start: 0, end: session.duration)
        let minimum = min(1.0, session.duration)
        trim.start = min(max(0, start ?? trim.start), max(0, session.duration - minimum))
        trim.end = min(max(end ?? trim.end, trim.start + minimum), session.duration)
        if start != nil, trim.end <= trim.start + minimum { trim.end = session.duration }
        trims[session.fingerprint] = trim.start < 0.001 && trim.end >= session.duration - 0.001 ? nil : trim
        saveTrims()
    }
    func resetTrim(for session: RecordingSession) { trims[session.fingerprint] = nil; saveTrims() }
    private func saveTrims() { defaults.set(try? JSONEncoder().encode(trims), forKey: "session-trims") }
    static func sourceID(_ recording: SourceRecording) -> String { SHA256.hash(data: Data(recording.fingerprint.utf8)).map { String(format: "%02x", $0) }.joined() }

    func uploadPlan(_ sessions: [RecordingSession]) -> (urls: [URL], plans: [ImportSessionPlan], sourceIDs: [String: String]) {
        let all = sessions.flatMap(\.recordings)
        let plans = sessions.map { session in
            let total = session.recordings.reduce(Int64(0)) { $0 + $1.frameCount }
            let frames = trim(for: session)?.frames(sampleRate: session.recordings[0].format.sampleRate, totalFrames: total)
            return ImportSessionPlan(sourceIDs: session.recordings.map(Self.sourceID), trimStartFrame: frames?.lowerBound, trimEndFrame: frames?.upperBound)
        }
        return (all.map(\.url), plans, Dictionary(all.map { ($0.url.path, Self.sourceID($0)) }, uniquingKeysWith: { first, _ in first }))
    }
    func record(for session: RecordingSession, server: String) -> SessionImportRecord? {
        let ids = session.recordings.map(Self.sourceID)
        let total = session.recordings.reduce(Int64(0)) { $0 + $1.frameCount }
        let bounds = trim(for: session)?.frames(sampleRate: session.recordings[0].format.sampleRate, totalFrames: total)
        return records.last {
            $0.server == server && ($0.fingerprint == session.fingerprint || $0.sourceIDs == ids)
            && ($0.trimStartFrame ?? 0) == (bounds?.lowerBound ?? 0)
            && ($0.trimEndFrame ?? total) == (bounds?.upperBound ?? total)
        }
    }
    func partlyImported(_ session: RecordingSession, server: String) -> Bool {
        let imported = Set(records.filter { $0.server == server && $0.stage == "complete" }.flatMap(\.sources))
        let sourceIDs = Set(records.filter { $0.server == server && $0.stage == "complete" }.flatMap { $0.sourceIDs ?? [] })
        return session.recordings.contains { imported.contains($0.fingerprint) || sourceIDs.contains(Self.sourceID($0)) }
    }
    func register(_ selectedSessions: [RecordingSession], server: String, importID: String) {
        for (index, session) in selectedSessions.enumerated() {
            records.removeAll { $0.server == server && $0.fingerprint == session.fingerprint }
            let total = session.recordings.reduce(Int64(0)) { $0 + $1.frameCount }
            let bounds = trim(for: session)?.frames(sampleRate: session.recordings[0].format.sampleRate, totalFrames: total)
            records.append(SessionImportRecord(server: server, fingerprint: session.fingerprint, sources: session.recordings.map(\.fingerprint), sourceIDs: session.recordings.map(Self.sourceID), trimStartFrame: bounds?.lowerBound, trimEndFrame: bounds?.upperBound, importID: importID, sessionIndex: index, stage: "uploading"))
        }
        saveHistory()
    }
    func updateReceipt(server: String, importID: String, entries: [[String: Any]]) {
        for entry in entries {
            guard let index = entry["session_index"] as? Int,
                  let position = records.firstIndex(where: { $0.server == server && $0.importID == importID && $0.sessionIndex == index }) else { continue }
            records[position].entryID = entry["id"] as? String
            records[position].stage = entry["stage"] as? String ?? "queued"
            records[position].activeStage = entry["active_stage"] as? String
            records[position].failure = entry["failure"] as? String
            records[position].pauseRequested = entry["pause_requested"] as? Bool
        }
        saveHistory()
    }
    func syncHistory(_ entries: [[String: Any]], server: String) {
        for entry in entries {
            guard let id = entry["id"] as? String, let ids = entry["source_ids"] as? [String], !ids.isEmpty else { continue }
            let existing = records.firstIndex { $0.server == server && $0.entryID == id }
            var record = existing.map { records[$0] } ?? SessionImportRecord(server: server, fingerprint: "remote:" + id, sources: [], sourceIDs: ids, importID: "", sessionIndex: 0, stage: "queued")
            record.entryID = id
            record.sourceIDs = ids
            record.trimStartFrame = (entry["trim_start_frame"] as? NSNumber)?.int64Value
            record.trimEndFrame = (entry["trim_end_frame"] as? NSNumber)?.int64Value
            record.stage = entry["stage"] as? String ?? "queued"
            record.activeStage = entry["active_stage"] as? String
            record.failure = entry["failure"] as? String
            record.pauseRequested = entry["pause_requested"] as? Bool
            if let existing { records[existing] = record } else { records.append(record) }
        }
        saveHistory()
    }
    func reconcileHistory(server: String, entryIDs: Set<String>) {
        records.removeAll { $0.server == server && $0.entryID != nil && !entryIDs.contains($0.entryID!) }
        saveHistory()
    }
    func setStage(entry: String, stage: String) {
        for index in records.indices where records[index].entryID == entry { records[index].stage = stage; records[index].pauseRequested = false }
        saveHistory()
    }
    func setPausing(entry: String) {
        for index in records.indices where records[index].entryID == entry { records[index].pauseRequested = true }
        saveHistory()
    }
    private func saveHistory() { defaults.set(try? JSONEncoder().encode(records), forKey: "session-import-history") }
}
