import XCTest
import AVFoundation
@testable import VaultUploader

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
        model.syncHistory([["id":"remote-entry","source_ids":plan.sourceIDs,"trim_start_frame":plan.trimStartFrame!,"trim_end_frame":plan.trimEndFrame!,"stage":"complete"]], server: "https://vault.example.com")
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
}
