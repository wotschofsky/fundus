import AVFoundation
import Observation
import VaultNative

/// One transcript row, precomputed so scrolling stays cheap for multi-hour recordings.
struct TranscriptRow: Identifiable, Equatable, Sendable {
    let id: String
    let speaker: String
    let startMs: Int
    let clock: String
    let text: String
}

/// A recording opened from the vault: transcript rows, speakers, streamed playback, edits.
@MainActor
@Observable
final class TranscriptModel {
    let entryID: String
    private(set) var entry: Entry?
    private(set) var rows: [TranscriptRow] = []
    private(set) var loadError: String?
    private(set) var isLoading = true
    private(set) var isUpdatingSpeaker = false
    /// The row being spoken; updated only when it changes, not on every playback tick.
    private(set) var currentRowID: String?
    var actionError: String?
    let player = PlayerModel()

    @ObservationIgnored private let library: Library
    @ObservationIgnored private var audio: VaultAudioLoader?
    @ObservationIgnored private var timeline = SessionTimeline(chunks: [], timeZone: .current)
    @ObservationIgnored private var rowsStage: String?

    init(entryID: String, library: Library) {
        self.entryID = entryID
        self.library = library
    }

    var speakers: [Speaker] { entry?.speakers ?? [] }
    func speaker(_ label: String) -> Speaker? { speakers.first { $0.label == label } }

    /// The person's name, or `Speaker 2` by order of appearance in the speaker list.
    func name(of label: String) -> String {
        if let name = speaker(label)?.name { return name }
        return "Speaker \((speakers.firstIndex { $0.label == label } ?? speakers.count) + 1)"
    }

    func clock(atSeconds seconds: Double) -> String {
        entry?.recordedAt == nil ? Format.position(seconds) : timeline.clockTime(atMs: Int(seconds * 1000))
    }

    // MARK: Loading

    func load() async {
        isLoading = true
        defer { isLoading = false }
        await refresh()
    }

    /// While the vault is still working on the recording, follow its progress.
    func watch() async {
        while !Task.isCancelled, entry?.isProcessing == true {
            try? await Task.sleep(for: .seconds(3))
            guard !Task.isCancelled else { return }
            await refresh()
        }
    }

    func refresh() async {
        do {
            let detail = try await VaultAPI.get(VaultAPI.entryPath(entryID), as: Entry.self)
            entry = detail
            timeline = Self.timeline(for: detail)
            loadError = nil
            // Segments are written once per processing step; reread them only when the stage moves on
            // (or the recording time changed, which moves every clock time).
            let stage = "\(detail.stage)|\(detail.activeStage ?? "")|\(detail.recordedAt?.timeIntervalSince1970 ?? 0)"
            if stage != rowsStage {
                rows = try await Self.rows(entryID, timeline: timeline, dated: detail.recordedAt != nil)
                rowsStage = stage
            }
            if detail.audioAvailable && audio == nil { loadAudio(detail) }
        } catch is CancellationError {
        } catch {
            if entry == nil { loadError = error.localizedDescription } else { actionError = error.localizedDescription }
        }
    }

    private func loadAudio(_ entry: Entry) {
        let (asset, loader) = VaultAudioLoader.asset(server: NativeVaultSession.shared.server, entryID: entry.id)
        audio = loader
        player.load(AVPlayerItem(asset: asset), duration: entry.duration)
    }

    private static func rows(_ id: String, timeline: SessionTimeline, dated: Bool) async throws -> [TranscriptRow] {
        var segments: [Segment] = []
        var cursor = 0
        while true {
            let page = try await VaultAPI.get(VaultAPI.entryPath(id, "/transcript?limit=2000&cursor=\(cursor)"), as: Page<Segment>.self)
            segments += page.items
            guard let next = page.nextCursor, next > cursor else { break }
            cursor = next
        }
        // Clock strings for thousands of rows: off the main actor.
        return await Task.detached(priority: .userInitiated) {
            segments.map { segment in
                TranscriptRow(id: segment.id, speaker: segment.speaker, startMs: segment.startMs,
                              clock: dated ? timeline.clockTime(atMs: segment.startMs) : Format.position(Double(segment.startMs) / 1000),
                              text: segment.text)
            }
        }.value
    }

    /// Positions in the playback audio to clock times, per source file, after the session's trim.
    private static func timeline(for entry: Entry) -> SessionTimeline {
        let sources = entry.sources ?? []
        let rate = Double(max(1, sources.first?.format?.sampleRate ?? 48_000))
        let trimmed = Double(entry.trimStartFrame ?? 0) / rate
        var chunks = sources.compactMap { source in
            source.recordedAt.map { SessionTimeline.Chunk(offsetSeconds: Double(source.offsetMs) / 1000 - trimmed, start: $0) }
        }
        if chunks.isEmpty, let start = entry.recordedAt { chunks = [.init(offsetSeconds: 0, start: start)] }
        return SessionTimeline(chunks: chunks, timeZone: entry.zone)
    }

    // MARK: Playback

    func play(_ row: TranscriptRow) {
        player.seek(to: Double(row.startMs) / 1000, play: true)
    }

    func playbackMoved() {
        guard player.isAvailable, player.currentSeconds > 0 || player.isPlaying else { return setCurrent(nil) }
        let ms = Int(player.currentSeconds * 1000)
        // Rows are ordered by start time: binary search for the last one that has started.
        var low = 0, high = rows.count
        while low < high {
            let mid = (low + high) / 2
            if rows[mid].startMs <= ms { low = mid + 1 } else { high = mid }
        }
        setCurrent(low > 0 ? rows[low - 1].id : nil)
    }

    private func setCurrent(_ id: String?) {
        if currentRowID != id { currentRowID = id }
    }

    // MARK: Edits

    /// `person` names the speaker; `nil` keeps it unidentified (`locked`) or hands it back to recognition.
    func setIdentity(_ person: Person?, for label: String, locked: Bool = true) async {
        isUpdatingSpeaker = true
        defer { isUpdatingSpeaker = false }
        do {
            try await library.assign(person, to: label, in: entryID, locked: locked)
            await refresh()
            actionError = nil
        } catch {
            actionError = "Could not save the change: \(error.localizedDescription)"
        }
    }

    func addPerson(named name: String, for label: String) async {
        do {
            let person = try await library.addPerson(named: name)
            await setIdentity(person, for: label)
        } catch {
            actionError = error.localizedDescription
        }
    }

    func save(title: String?, labels: String, recordedAt: Date?) async throws {
        let labels = labels.split(separator: ",").map { $0.trimmingCharacters(in: .whitespaces) }.filter { !$0.isEmpty }
        try await library.update(entryID, title: title, labels: labels, recordedAt: recordedAt)
        await refresh()
    }

    func markdown() -> String {
        guard let entry else { return "" }
        func escape(_ value: String) -> String {
            value.replacingOccurrences(of: "\\", with: "\\\\").replacingOccurrences(of: "*", with: "\\*")
                .replacingOccurrences(of: "[", with: "\\[").replacingOccurrences(of: "]", with: "\\]")
        }
        var lines = ["# \(entry.displayTitle)", "", "- Recorded: \(entry.dateLine)", "- Duration: \(Format.duration(entry.duration))"]
        if !entry.labels.isEmpty { lines.append("- Labels: \(entry.labels.joined(separator: ", "))") }
        if let summary = entry.summary, !summary.isEmpty { lines += ["", "## Summary (generated)", "", summary] }
        lines += ["", "## Transcript", ""]
        for row in rows { lines += ["[\(row.clock)] **\(escape(name(of: row.speaker)))**", row.text, ""] }
        return lines.joined(separator: "\n")
    }
}
