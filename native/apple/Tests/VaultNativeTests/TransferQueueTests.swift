#if os(macOS)
import Foundation
import XCTest
@testable import VaultNative

private struct TransferMetrics: Sendable {
    var maximumCopies = 0
    var copyOverlappedUpload = false
    var maximumPreparations = 0
    var maximumUploads = 0
    var preparationOverlappedUpload = false
    var preparations = 0
    var patches = 0
    var heads = 0
    var completions = 0
    var originalNames: Set<String> = []
    var sourceIDs: Set<String> = []
    var plannedSources: [[String]] = []
    var trimStart: Int64?
    var maximumChunk = 0
}

private actor TransferServer {
    private var metrics = TransferMetrics()
    private var copying = 0
    private var preparing = 0
    private var uploading = 0
    private var lengths: [String: Int64] = [:]
    private var offsets: [String: Int64] = [:]
    private var delayUpload = false
    init(delayUpload: Bool = false) { self.delayUpload = delayUpload }
    func report() -> TransferMetrics { metrics }
    func releaseUpload() { delayUpload = false }
    func copy(_ source: URL, _ destination: URL) async throws {
        copying += 1; metrics.maximumCopies = max(metrics.maximumCopies, copying)
        defer { copying -= 1 }
        let initial = source.lastPathComponent == "clip-0.wav" || source.lastPathComponent == "clip-1.wav"
        try await Task.sleep(for: .milliseconds(initial ? 10 : 80))
        try FileManager.default.copyItem(at: source, to: destination)
    }
    func prepare(_ source: URL, _ folder: URL) async throws -> URL {
        preparing += 1; metrics.preparations += 1
        metrics.maximumPreparations = max(metrics.maximumPreparations, preparing)
        defer { preparing -= 1 }
        try await Task.sleep(for: .milliseconds(25))
        return source // A deterministic passthrough seam; codec verification has separate real tests.
    }
    func request(_ request: URLRequest, _ file: URL?) async throws -> (Data, URLResponse) {
        let path = request.url!.path
        let method = request.httpMethod ?? "GET"
        func response(_ status: Int, _ value: [String: Any] = [:], headers: [String: String] = [:]) -> (Data, URLResponse) {
            (try! JSONSerialization.data(withJSONObject: value), HTTPURLResponse(url: request.url!, statusCode: status, httpVersion: "HTTP/1.1", headerFields: headers)!)
        }
        if path == "/api/v1/imports", method == "POST" {
            if let data = request.httpBody, let body = try JSONSerialization.jsonObject(with: data) as? [String: Any], let sessions = body["sessions"] as? [[String: Any]] {
                metrics.plannedSources = sessions.compactMap { $0["source_ids"] as? [String] }
                metrics.trimStart = (sessions.first?["trim_start_frame"] as? NSNumber)?.int64Value
            }
            return response(201, ["id": "batch"])
        }
        if path == "/api/v1/uploads", method == "POST" {
            var metadata: [String: String] = [:]
            for part in (request.value(forHTTPHeaderField: "Upload-Metadata") ?? "").split(separator: ",") {
                let pieces = part.split(separator: " ", maxSplits: 1)
                if pieces.count == 2, let data = Data(base64Encoded: String(pieces[1])) { metadata[String(pieces[0])] = String(decoding: data, as: UTF8.self) }
            }
            let location = "/api/v1/uploads/\(lengths.count)"
            lengths[location] = Int64(request.value(forHTTPHeaderField: "Upload-Length")!)!
            offsets[location] = 0
            if let name = metadata["original_name"] { metrics.originalNames.insert(name) }
            if let id = metadata["source_id"] { metrics.sourceIDs.insert(id) }
            return response(201, headers: ["Location": location])
        }
        if method == "HEAD", let offset = offsets[path] {
            metrics.heads += 1
            return response(204, headers: ["Upload-Offset": String(offset)])
        }
        if method == "PATCH", let file, let offset = offsets[path] {
            let supplied = Int64(request.value(forHTTPHeaderField: "Upload-Offset")!)!
            guard supplied == offset else { return response(409) }
            let size = try file.resourceValues(forKeys: [.fileSizeKey]).fileSize!
            metrics.maximumChunk = max(metrics.maximumChunk, size)
            metrics.patches += 1
            uploading += 1
            metrics.maximumUploads = max(metrics.maximumUploads, uploading)
            metrics.preparationOverlappedUpload = metrics.preparationOverlappedUpload || preparing > 0
            metrics.copyOverlappedUpload = metrics.copyOverlappedUpload || copying > 0
            defer { uploading -= 1 }
            // Simulate the server committing bytes even if its response is lost on Pause.
            offsets[path] = offset + Int64(size)
            if delayUpload { try await Task.sleep(for: .seconds(30)) }
            else { try await Task.sleep(for: .milliseconds(130)) }
            return response(204, headers: ["Upload-Offset": String(offset + Int64(size))])
        }
        if path == "/api/v1/imports/batch/complete" {
            guard lengths.allSatisfy({ offsets[$0.key] == $0.value }) else { return response(409, ["error": "Incomplete fixture import"] ) }
            metrics.completions += 1
            return response(200, ["state": "verifying"])
        }
        if path == "/api/v1/imports/batch" {
            return response(200, ["state": "complete", "files": lengths.map { _ in ["state": "verified"] }])
        }
        return response(404, ["error": "Unexpected fixture request"])
    }
}

private actor SourceCopyGate {
    private var continuation: CheckedContinuation<Void, Never>?
    private(set) var entered = false
    func copy(_ source: URL, _ destination: URL) async throws {
        entered = true
        await withCheckedContinuation { continuation = $0 }
        try Task.checkCancellation()
        try FileManager.default.copyItem(at: source, to: destination)
    }
    func release() { continuation?.resume(); continuation = nil }
}

@MainActor
final class TransferQueueTests: XCTestCase {
    private func waitFor(_ condition: () async -> Bool) async throws {
        for _ in 0..<1000 {
            if await condition() { return }
            try await Task.sleep(for: .milliseconds(5))
        }
        XCTFail("Transfer did not reach the expected state")
    }
    private func make(_ server: TransferServer, count: Int, size: Int = 131_072) throws -> (URL, [URL], TransferQueue) {
        let folder = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        let originals = folder.appendingPathComponent("originals", isDirectory: true)
        try FileManager.default.createDirectory(at: originals, withIntermediateDirectories: true)
        var urls: [URL] = []
        for index in 0..<count {
            let url = originals.appendingPathComponent("clip-\(index).wav")
            try Data(repeating: UInt8(index + 1), count: size).write(to: url)
            urls.append(url)
        }
        var state = QueueState(); state.server = "https://fixture.example"
        let queue = TransferQueue(folder: folder.appendingPathComponent("queue"), initialState: state, tokenOverride: "synthetic-only",
                                  copy: { try await server.copy($0, $1) }, preparation: { try await server.prepare($0, $1) }, request: { try await server.request($0, $1) })
        return (folder, urls, queue)
    }

    func testPauseKeepsRecorderBusyUntilSourceCopyActuallyDrains() async throws {
        let server = TransferServer()
        let gate = SourceCopyGate()
        let folder = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        try FileManager.default.createDirectory(at: folder, withIntermediateDirectories: true)
        defer { try? FileManager.default.removeItem(at: folder) }
        let original = folder.appendingPathComponent("original.wav")
        try Data(repeating: 1, count: 64).write(to: original)
        var state = QueueState(); state.server = "https://fixture.example"
        let queue = TransferQueue(folder: folder.appendingPathComponent("queue"), initialState: state, tokenOverride: "synthetic-only",
                                  copy: { try await gate.copy($0, $1) }, preparation: { try await server.prepare($0, $1) }, request: { try await server.request($0, $1) })
        try await queue.enqueue([original])
        try await waitFor { await gate.entered }
        XCTAssertTrue(queue.isReadingSource)
        queue.pause()
        XCTAssertTrue(queue.isReadingSource, "Pause must not enable eject before source handles finish draining")
        XCTAssertEqual(queue.activeSourceCopies, 1)
        await gate.release()
        try await waitFor { queue.snapshot.state == "paused" }
        XCTAssertFalse(queue.isReadingSource)
        XCTAssertEqual(queue.activeSourceCopies, 0)
        XCTAssertNotNil(queue.snapshot.files.first?.originalURL, "The pending source remains resumable without pretending it is still being read")
    }

    func testBoundedPreparationAndUploadOverlapWithPersistedSessionManifest() async throws {
        let server = TransferServer()
        let (folder, urls, queue) = try make(server, count: 6)
        defer { try? FileManager.default.removeItem(at: folder) }
        let sourceIDs = Dictionary(uniqueKeysWithValues: urls.enumerated().map { ($0.element.path, "source-\($0.offset)") })
        let plans = [ImportSessionPlan(sourceIDs: ["source-0", "source-1"], trimStartFrame: 48_000, trimEndFrame: 96_000), ImportSessionPlan(sourceIDs: ["source-2", "source-3", "source-4", "source-5"])]
        try await queue.enqueue(urls, sessions: plans, sourceIDs: sourceIDs)
        try await waitFor { queue.snapshot.state == "verified" || queue.snapshot.state == "failed" }
        XCTAssertEqual(queue.snapshot.state, "verified", queue.snapshot.error ?? "")
        let report = await server.report()
        XCTAssertEqual(report.maximumCopies, 2)
        XCTAssertTrue(report.copyOverlappedUpload)
        XCTAssertEqual(report.maximumPreparations, 2)
        XCTAssertGreaterThan(report.maximumUploads, 1)
        XCTAssertLessThanOrEqual(report.maximumUploads, 3)
        XCTAssertTrue(report.preparationOverlappedUpload)
        XCTAssertEqual(report.heads, urls.count)
        XCTAssertEqual(report.completions, 1)
        XCTAssertEqual(report.originalNames, Set(urls.map(\.lastPathComponent)))
        XCTAssertEqual(report.sourceIDs, Set(sourceIDs.values))
        XCTAssertEqual(report.plannedSources, plans.map(\.sourceIDs))
        XCTAssertEqual(report.trimStart, 48_000)
        XCTAssertLessThanOrEqual(report.maximumChunk, 8 * 1024 * 1024)
        let journal = try JSONDecoder().decode(QueueState.self, from: Data(contentsOf: folder.appendingPathComponent("queue/queue.json")))
        XCTAssertEqual(journal.sessions?.map(\.sourceIDs), plans.map(\.sourceIDs))
        for (index, url) in urls.enumerated() { XCTAssertEqual(try Data(contentsOf: url), Data(repeating: UInt8(index + 1), count: 131_072)) }
    }

    func testPauseReconcilesCommittedUploadOffsetWithoutPreparingOrUploadingAgain() async throws {
        let server = TransferServer(delayUpload: true)
        let (folder, urls, queue) = try make(server, count: 1)
        defer { try? FileManager.default.removeItem(at: folder) }
        try await queue.enqueue(urls)
        try await waitFor { await server.report().patches == 1 }
        queue.pause()
        await server.releaseUpload()
        queue.resume() // Also exercises Resume pressed before cancellation has drained.
        try await waitFor { queue.snapshot.state == "verified" || queue.snapshot.state == "failed" }
        XCTAssertEqual(queue.snapshot.state, "verified", queue.snapshot.error ?? "")
        let report = await server.report()
        XCTAssertEqual(report.preparations, 1)
        XCTAssertEqual(report.patches, 1)
        XCTAssertEqual(report.heads, 2)
        XCTAssertEqual(report.completions, 1)
    }

    func testInterruptedStagingKeepsWholeBatchForResume() async throws {
        let server = TransferServer()
        let (folder, urls, queue) = try make(server, count: 4)
        defer { try? FileManager.default.removeItem(at: folder) }
        try await queue.enqueue(urls)
        queue.pause()
        try await waitFor { queue.snapshot.state == "paused" }
        XCTAssertEqual(queue.snapshot.files.count, urls.count)
        queue.resume()
        try await waitFor { queue.snapshot.state == "verified" || queue.snapshot.state == "failed" }
        XCTAssertEqual(queue.snapshot.state, "verified", queue.snapshot.error ?? "")
        let report = await server.report()
        XCTAssertEqual(report.originalNames, Set(urls.map(\.lastPathComponent)))
        XCTAssertEqual(report.completions, 1)
    }

    func testOlderQueueJournalRemainsDecodable() throws {
        let data = Data(#"{"server":"https://fixture.example","timezone":"UTC","state":"paused","files":[{"id":"old","name":"clip.wav","source":"copy.wav","offset":0,"length":1,"state":"waiting"}]}"#.utf8)
        let journal = try JSONDecoder().decode(QueueState.self, from: data)
        XCTAssertNil(journal.sessions)
        XCTAssertNil(journal.files[0].sourceID)
        XCTAssertNil(journal.files[0].originalURL)
    }
}
#endif
