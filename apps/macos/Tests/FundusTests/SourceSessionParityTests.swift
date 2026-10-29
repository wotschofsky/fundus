import XCTest
import AVFoundation
@testable import Fundus

@MainActor final class SourceSessionParityTests: XCTestCase {
    func testTrimSelectionAndOverridesPersistAndProduceOrderedRemotePlan() throws {
        let defaults = UserDefaults(suiteName: "session-parity-\(UUID())")!
        let model = SourceSessions(defaults: defaults)
        let recordings = Array(briefRecordings()[1...5])
        let files = recordings.map { recording in
            RecorderFile(url: recording.url, size: recording.fileSize, modified: nil, recorded: recording.start, transmitter: recording.transmitter, recording: recording)
        }
        model.update(files, server: "https://vault.example.com")
        let session = try XCTUnwrap(model.sessions.first)
        XCTAssertEqual(model.sessions.count, 1)
        XCTAssertEqual(session.recordings.count, 5)
        XCTAssertTrue(model.isSelected(session))
        model.setTrim(start: 120, end: 7000, for: session)
        let plan = model.uploadPlan([session])
        XCTAssertEqual(plan.urls, recordings.map(\.url))
        XCTAssertEqual(plan.plans[0].sourceIDs, recordings.map(SourceSessions.sourceID))
        XCTAssertEqual(plan.plans[0].trimStartFrame, 120 * 48000)
        XCTAssertEqual(plan.plans[0].trimEndFrame, 7000 * 48000)
        model.split(session, before: recordings[2])
        XCTAssertEqual(model.sessions.count, 2)
        let restored = SourceSessions(defaults: defaults)
        restored.update(files, server: "https://vault.example.com")
        XCTAssertEqual(restored.sessions.count, 2)
        XCTAssertEqual(restored.trim(for: session), Trim(start: 120, end: 7000))
        restored.mergeWithPrevious(restored.sessions[1])
        XCTAssertEqual(restored.sessions.count, 1)
    }

    func testTrimClockFollowsEachOriginalChunkAndHistoryMatchesRemoteSourceIDs() throws {
        let model = SourceSessions(defaults: UserDefaults(suiteName: "session-clock-\(UUID())")!)
        let recordings = Array(briefRecordings()[1...5])
        let files = recordings.map { RecorderFile(url: $0.url, size: $0.fileSize, modified: nil, recorded: $0.start, transmitter: $0.transmitter, recording: $0) }
        model.update(files, server: "https://vault.example.com")
        let session = try XCTUnwrap(model.sessions.first)
        let trim = Trim(start: 1860.6, end: 3630.6)
        model.setTrim(start: trim.start, end: trim.end, for: session)
        XCTAssertEqual(session.start(trimmedBy: trim), date("2026-10-01 21:14:50"))
        XCTAssertEqual(session.end(trimmedBy: trim), date("2026-10-01 21:44:20"))
        let plan = model.uploadPlan([session]).plans[0]
        model.syncHistory(try wire([["id": "remote-entry", "source_ids": plan.sourceIDs, "trim_start_frame": plan.trimStartFrame!, "trim_end_frame": plan.trimEndFrame!, "stage": "complete"]]), server: "https://vault.example.com")
        XCTAssertEqual(model.record(for: session, server: "https://vault.example.com")?.entryID, "remote-entry")
        XCTAssertNil(model.record(for: session, server: "https://other.example.com"))
        model.resetTrim(for: session)
        XCTAssertNil(model.record(for: session, server: "https://vault.example.com"))
    }

    func testPreviewPlaysMergedChunksAsOneContinuousAsset() async throws {
        let folder = try temporaryDirectory()
        defer { try? FileManager.default.removeItem(at: folder) }
        try writeWAV(to: folder.appendingPathComponent("TX01_MIC001_20261001_100000_orig.wav"), seconds: 8, sampleRate: 16000)
        try writeWAV(to: folder.appendingPathComponent("TX01_MIC002_20261001_100008_orig.wav"), seconds: 5, sampleRate: 16000)
        let files = await RecorderScanner.files(in: folder, djiOnly: true, timezone: berlin)
        let session = try XCTUnwrap(SessionGrouper(rolloverDuration: 8, rolloverSlack: 0.5).group(files.compactMap(\.recording)).first)
        XCTAssertEqual(session.recordings.count, 2)
        let (asset, chunks) = try await SessionPreview.composition(for: session)
        let duration = try await asset.load(.duration)
        XCTAssertEqual(duration.seconds, 13, accuracy: 0.01)
        XCTAssertEqual(chunks.map(\.offsetSeconds), [0, 8])
        XCTAssertEqual(SessionTimeline(chunks: chunks, timeZone: berlin).clockTime(atMs: 9000), "10:00:09")
    }

    func testAuthoritativeHistoryRemovesDeletedBadgesAfterAllPagesButKeepsPendingImports() throws {
        let suite = "session-history-\(UUID())"
        let defaults = UserDefaults(suiteName: suite)!
        defer { defaults.removePersistentDomain(forName: suite) }
        let server = "https://vault.example.com"
        let otherServer = "https://other.example.com"
        let files = briefRecordings().map {
            RecorderFile(url: $0.url, size: $0.fileSize, modified: nil, recorded: $0.start, transmitter: $0.transmitter, recording: $0)
        }
        let model = SourceSessions(defaults: defaults)
        model.update(files, server: server)
        XCTAssertGreaterThanOrEqual(model.sessions.count, 3)
        let removed = model.sessions[0], retained = model.sessions[1], pending = model.sessions[2]
        model.register([removed, retained, pending], server: server, importID: "local-batch")
        model.updateReceipt(server: server, importID: "local-batch", entries: try wire([
            ["session_index": 0, "id": "trashed-or-deleted", "stage": "complete"],
            ["session_index": 1, "id": "retained-on-second-page", "stage": "complete"]
        ]))
        model.register([removed], server: otherServer, importID: "another-vault-batch")
        model.updateReceipt(server: otherServer, importID: "another-vault-batch", entries: try wire([
            ["session_index": 0, "id": "other-vault-entry", "stage": "complete"]
        ]))

        // An incomplete or interrupted history fetch must not erase badges for
        // entries absent from the first page, or for locally queued imports.
        let firstPage: [[String: Any]] = [["id": "new-remote-entry", "source_ids": ["remote-source"], "stage": "complete"]]
        model.syncHistory(try wire(firstPage), server: server)
        XCTAssertTrue(model.partlyImported(removed, server: server))
        XCTAssertEqual(model.record(for: retained, server: server)?.entryID, "retained-on-second-page")
        XCTAssertEqual(model.record(for: pending, server: server)?.stage, "uploading")
        let afterInterruptedFetch = SourceSessions(defaults: defaults)
        XCTAssertNotNil(afterInterruptedFetch.record(for: removed, server: server))
        XCTAssertNotNil(afterInterruptedFetch.record(for: retained, server: server))

        let secondPage: [[String: Any]] = [["id": "retained-on-second-page", "source_ids": retained.recordings.map(SourceSessions.sourceID), "stage": "complete"]]
        model.syncHistory(try wire(secondPage), server: server)
        XCTAssertNotNil(model.record(for: removed, server: server))
        let authoritativeIDs = Set((firstPage + secondPage).compactMap { $0["id"] as? String })
        model.reconcileHistory(server: server, entryIDs: authoritativeIDs)

        XCTAssertNil(model.record(for: removed, server: server))
        XCTAssertFalse(model.partlyImported(removed, server: server))
        XCTAssertEqual(model.record(for: retained, server: server)?.entryID, "retained-on-second-page")
        XCTAssertEqual(model.record(for: pending, server: server)?.importID, "local-batch")
        XCTAssertNil(model.record(for: pending, server: server)?.entryID)
        XCTAssertEqual(model.record(for: removed, server: otherServer)?.entryID, "other-vault-entry")

        // A fully fetched empty archive is authoritative too; pending local
        // uploads and another vault's history still survive, including relaunch.
        model.reconcileHistory(server: server, entryIDs: [])
        let restored = SourceSessions(defaults: defaults)
        XCTAssertNil(restored.record(for: removed, server: server))
        XCTAssertNil(restored.record(for: retained, server: server))
        XCTAssertEqual(restored.record(for: pending, server: server)?.stage, "uploading")
        XCTAssertEqual(restored.record(for: removed, server: otherServer)?.entryID, "other-vault-entry")
        XCTAssertEqual(restored.records.count, 2)
    }

    func testScannerInspectsRealHeadersAndReportsUnreadableClips() async throws {
        let folder = try temporaryDirectory()
        defer { try? FileManager.default.removeItem(at: folder) }
        try writeWAV(to: folder.appendingPathComponent("TX01_MIC001_20261001_193402_orig.wav"), seconds: 2.5)
        try Data("not audio".utf8).write(to: folder.appendingPathComponent("TX01_MIC002_20261001_204349_orig.wav"))
        let files = await RecorderScanner.files(in: folder, djiOnly: true, timezone: berlin)
        XCTAssertEqual(files.count, 2)
        let valid = try XCTUnwrap(files.first { $0.recording != nil }?.recording)
        XCTAssertEqual(valid.duration, 2.5, accuracy: 0.0001)
        XCTAssertEqual(valid.frameCount, 20_000)
        XCTAssertEqual(files.filter { $0.inspectionError != nil }.count, 1)
    }

    /// Server JSON, decoded the way the app decodes it.
    private func wire<T: Decodable>(_ objects: [[String: Any]]) throws -> [T] {
        try VaultAPI.decode([T].self, from: JSONSerialization.data(withJSONObject: objects))
    }

    func testCreatedPersonDecodesWithoutSampleCount() throws {
        // POST /people answers {id, name}; GET /people adds samples.
        let created = try VaultAPI.decode(Person.self, from: Data(#"{"id":"p1","name":"Morgan"}"#.utf8))
        XCTAssertEqual(created.samples, 0)
        let listed = try VaultAPI.decode(Page<Person>.self, from: Data(#"{"people":[{"id":"p1","name":"Morgan","samples":3}]}"#.utf8))
        XCTAssertEqual(listed.items.first?.samples, 3)
    }

    func testFailedImportReleasesItsSessions() throws {
        let defaults = UserDefaults(suiteName: "failed-import-\(UUID())")!
        let model = SourceSessions(defaults: defaults)
        let server = "https://vault.example.com"
        let files = briefRecordings().map { RecorderFile(url: $0.url, size: $0.fileSize, modified: nil, recorded: $0.start, transmitter: $0.transmitter, recording: $0) }
        model.update(files, server: server)
        let session = try XCTUnwrap(model.sessions.first)
        model.register([session], server: server, importID: "batch")
        XCTAssertEqual(model.pendingImportIDs(server: server), ["batch"])
        let receipt = try VaultAPI.decode(ImportReceipt.self, from: Data(#"{"id":"batch","state":"failed","entries":[],"files":[]}"#.utf8))
        model.failImport(server: server, importID: "batch", message: "rejected")
        XCTAssertEqual(receipt.state, "failed")
        XCTAssertTrue(model.pendingImportIDs(server: server).isEmpty)
        XCTAssertEqual(model.record(for: session, server: server)?.stage, "failed")
        model.dropUnfinished(server: server, importID: "batch")
        XCTAssertNil(model.record(for: session, server: server))
    }

    func testEntriesDecodeFromServerJSON() throws {
        let json = #"{"entries":[{"id":"e1","title":"Planning","recorded_at":"2026-10-22T07:30:12.000Z","created_at":"2026-10-24T16:29:20.974Z","duration_ms":1500000,"timezone":"Europe/Berlin","stage":"transcribing","active_stage":"transcribing","failure":null,"pause_requested":false,"labels":["work"],"summary":null,"warnings":[],"audio_available":true,"trashed_at":null,"trim_start_frame":null}],"next_cursor":100}"#
        let page = try VaultAPI.decode(Page<Entry>.self, from: Data(json.utf8))
        XCTAssertEqual(page.nextCursor, 100)
        let entry = try XCTUnwrap(page.items.first)
        XCTAssertTrue(entry.isProcessing)
        XCTAssertEqual(entry.status, "Transcribing")
        // Shown in this Mac's zone, not the recorder clock the entry was imported with.
        let recorded = try XCTUnwrap(entry.recordedAt)
        XCTAssertEqual(entry.dateLine, "\(Format.day(recorded)) · \(SessionTimeline.clock(recorded, in: .current, seconds: false))")
        XCTAssertEqual(recorded.timeIntervalSince1970, 1_792_654_212, accuracy: 0.001)
        XCTAssertEqual(entry.duration, 1500)
    }
}
