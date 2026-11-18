import AppKit
import XCTest
@testable import Fundus

private actor Inventory {
    var devices: [RecorderDevice]
    init(_ devices: [RecorderDevice]) { self.devices = devices }
    func replace(_ devices: [RecorderDevice]) { self.devices = devices }
    func read() -> [RecorderDevice] { devices }
}

@MainActor
final class RecorderSourcesTests: XCTestCase {
    private let first = RecorderDevice(url: URL(fileURLWithPath: "/Volumes/DJI"), files: [])
    private let second = RecorderDevice(url: URL(fileURLWithPath: "/Volumes/DJI 1"), files: [])

    private func eventually(_ condition: () -> Bool) async throws {
        for _ in 0..<200 {
            if condition() { return }
            try await Task.sleep(for: .milliseconds(5))
        }
        XCTFail("Recorder state did not update")
    }

    func testDetectsAlreadyConnectedAndRespondsToMountAndUnmount() async throws {
        let inventory = Inventory([first])
        let notifications = NotificationCenter()
        let source = RecorderSources(notifications: notifications, discover: { _ in await inventory.read() }, eject: { _ in })
        var arrivals = 0
        source.onArrival = { arrivals += 1 }
        defer { source.stop() }
        source.start(timezone: "UTC")
        try await eventually { !source.isScanning }
        XCTAssertEqual(source.devices, [first])
        XCTAssertEqual(arrivals, 0, "a recorder connected at launch is not announced")

        await inventory.replace([first, second])
        notifications.post(name: NSWorkspace.didMountNotification, object: nil, userInfo: [NSWorkspace.volumeURLUserInfoKey: second.url])
        try await eventually { source.devices.count == 2 }
        XCTAssertEqual(source.devices, [first, second])
        XCTAssertEqual(arrivals, 1)

        await inventory.replace([second])
        notifications.post(name: NSWorkspace.didUnmountNotification, object: nil, userInfo: [NSWorkspace.volumeURLUserInfoKey: first.url])
        try await eventually { source.devices == [second] && !source.isScanning }
        source.refresh()
        try await eventually { !source.isScanning }
        XCTAssertEqual(arrivals, 1, "unplugging and rescanning announce nothing")
    }

    func testProposesSessionsTheVaultDoesNotHaveYet() {
        let sessions = SourceSessions(defaults: UserDefaults(suiteName: UUID().uuidString)!)
        let files = [recording("TX01_MIC001_20261020_100000_orig.wav", seconds: 600),
                     recording("TX01_MIC002_20261020_120000_orig.wav", seconds: 5),
                     recording("TX01_MIC003_20261020_140000_orig.wav", seconds: 900),
                     recording("TX01_MIC004_20261020_160000_orig.wav", seconds: 30)]
            .map { RecorderFile(url: $0.url, size: $0.fileSize, modified: nil, recorded: $0.start, transmitter: $0.transmitter, recording: $0) }
        sessions.update(files, server: "https://vault")
        XCTAssertEqual(sessions.proposed(server: "https://vault").map(\.recordings[0].filename),
                       ["TX01_MIC001_20261020_100000_orig.wav", "TX01_MIC002_20261020_120000_orig.wav",
                        "TX01_MIC003_20261020_140000_orig.wav", "TX01_MIC004_20261020_160000_orig.wav"])

        sessions.register([sessions.sessions[0]], server: "https://vault", importID: "import")
        XCTAssertEqual(sessions.proposed(server: "https://vault").map(\.recordings[0].filename),
                       ["TX01_MIC002_20261020_120000_orig.wav", "TX01_MIC003_20261020_140000_orig.wav", "TX01_MIC004_20261020_160000_orig.wav"])
        XCTAssertEqual(sessions.proposed(server: "https://other").count, 4, "imports are per vault")
    }

    private func files(_ recordings: [SourceRecording]) -> [RecorderFile] {
        recordings.map { RecorderFile(url: $0.url, size: $0.fileSize, modified: nil, recorded: $0.start, transmitter: $0.transmitter, recording: $0) }
    }

    func testSplittingDependsOnlyOnTheSessionItself() {
        let sessions = SourceSessions(defaults: UserDefaults(suiteName: UUID().uuidString)!)
        // One file, then a 30-minute rollover pair: two sessions on one mic.
        sessions.update(files([recording("TX01_MIC001_20261020_100000_orig.wav", seconds: 600),
                               recording("TX01_MIC002_20261020_120000_orig.wav", seconds: 1800),
                               recording("TX01_MIC003_20261020_123000_orig.wav", seconds: 300)]), server: "https://vault")
        let older = sessions.sessions[0], newer = sessions.sessions[1]
        XCTAssertEqual(newer.recordings.count, 2)
        var queued: Set<String> = []
        func locks() -> (merge: Bool, split: Bool) { sessions.boundaryLocks(newer, server: "https://vault", isQueued: { queued.contains($0.id) }) }
        XCTAssert(locks() == (false, false))

        // The older one waits its turn, uploads, is imported: nothing may merge into it, the newer one still splits.
        queued = [older.id]
        XCTAssert(locks() == (true, false), "queued before")
        queued = []
        sessions.register([older], server: "https://vault", importID: "import")
        XCTAssert(locks() == (true, false), "uploading before")
        sessions.updateReceipt(server: "https://vault", importID: "import",
                               entries: [.init(id: "entry", sessionIndex: 0, stage: "complete", activeStage: nil, failure: nil, pauseRequested: nil)])
        XCTAssert(locks() == (true, false), "imported before")
        sessions.updateReceipt(server: "https://vault", importID: "import",
                               entries: [.init(id: "entry", sessionIndex: 0, stage: "failed", activeStage: nil, failure: "x", pauseRequested: nil)])
        XCTAssert(locks() == (false, false), "a failed import regroups")

        // The newer one itself waiting its turn, or on its way: neither.
        queued = [newer.id]
        XCTAssert(locks() == (true, true), "queued itself")
        queued = []
        sessions.register([newer], server: "https://vault", importID: "newer")
        XCTAssert(locks() == (true, true), "uploading itself")
    }

    func testRegroupingCannotDropATrim() {
        let sessions = SourceSessions(defaults: UserDefaults(suiteName: UUID().uuidString)!)
        sessions.update(files([recording("TX01_MIC001_20261020_100000_orig.wav", seconds: 600),
                               recording("TX01_MIC002_20261020_120000_orig.wav", seconds: 1800),
                               recording("TX01_MIC003_20261020_123000_orig.wav", seconds: 300)]), server: "https://vault")
        let older = sessions.sessions[0], newer = sessions.sessions[1]
        func locks() -> (merge: Bool, split: Bool) { sessions.boundaryLocks(newer, server: "https://vault", isQueued: { _ in false }) }
        // A trim belongs to its grouping: regrouped, the audio cut off would be imported after all.
        sessions.setTrim(start: 60, for: newer)
        XCTAssert(locks() == (true, true), "trimmed itself")
        sessions.resetTrim(for: newer)
        XCTAssert(locks() == (false, false))
        sessions.setTrim(end: 300, for: older)
        XCTAssert(locks() == (true, false), "the one before it trimmed")
    }

    /// The session a merge locks must be the one the grouper then joins, however the recordings tie.
    private func assertMergeChecksItsPartner(_ recordings: [SourceRecording], file: StaticString = #filePath, line: UInt = #line) {
        let sessions = SourceSessions(defaults: UserDefaults(suiteName: UUID().uuidString)!)
        sessions.update(files(recordings), server: "https://vault")
        XCTAssertEqual(sessions.sessions.count, 3, file: file, line: line)
        for session in sessions.sessions {
            guard let partner = sessions.previous(of: session) else { continue }
            XCTAssertTrue(sessions.boundaryLocks(session, server: "https://vault", isQueued: { $0.id == partner.id }).merge,
                          "a merge must not take the queued partner with it", file: file, line: line)
            let probe = SourceSessions(defaults: UserDefaults(suiteName: UUID().uuidString)!)
            probe.update(files(recordings), server: "https://vault")
            probe.mergeWithPrevious(session)
            XCTAssertTrue(probe.sessions.contains { $0.recordings == partner.recordings + session.recordings }, file: file, line: line)
        }
        let unmatched = sessions.sessions.filter { sessions.previous(of: $0) == nil }
        XCTAssertEqual(unmatched.count, 1, "only the first one has nothing before it", file: file, line: line)
    }

    func testMergeChecksTheSessionItJoinsWhenRecordingsStartInTheSameSecond() {
        // A mic whose clock was never set can start two recordings in the same second.
        assertMergeChecksItsPartner([recording("TX01_MIC001_20261020_100000_orig.wav", seconds: 600),
                                     recording("TX01_MIC002_20261020_120000_orig.wav", seconds: 60),
                                     recording("TX01_MIC003_20261020_120000_orig.wav", seconds: 60)])
    }

    func testMergeChecksTheSessionItJoinsWhenTwoDrivesHoldTheSameName() {
        // Two kits whose first mics are both TX01: same name, same second, different drives.
        var copy = recording("TX01_MIC001_20261020_100000_orig.wav", seconds: 120)
        copy.url = URL(fileURLWithPath: "/Volumes/DJI 1/\(copy.filename)")
        assertMergeChecksItsPartner([recording("TX01_MIC001_20261020_100000_orig.wav", seconds: 600), copy,
                                     recording("TX01_MIC002_20261020_120000_orig.wav", seconds: 60)])
    }

    func testMergesNoAudioAnImportIntoThisVaultHolds() {
        let sessions = SourceSessions(defaults: UserDefaults(suiteName: UUID().uuidString)!)
        let recordings = [recording("TX01_MIC001_20261020_100000_orig.wav", seconds: 600),
                          recording("TX01_MIC002_20261020_120000_orig.wav", seconds: 600),
                          recording("TX01_MIC003_20261020_140000_orig.wav", seconds: 600)]
        sessions.update(files(recordings), server: "https://vault")
        let second = sessions.sessions[1], third = sessions.sessions[2]
        func merge(_ session: RecordingSession, server: String = "https://vault") -> Bool {
            sessions.boundaryLocks(session, server: server, isQueued: { _ in false }).merge
        }
        XCTAssertFalse(merge(third))
        XCTAssertFalse(sessions.partlyImported(second, server: "https://vault"))

        // The first two went into the vault as one recording, from another Mac.
        func history(_ stage: String) -> SourceHistoryEntry {
            SourceHistoryEntry(id: "entry", stage: stage, activeStage: nil, failure: nil, pauseRequested: nil, trimStartFrame: nil, trimEndFrame: nil,
                               sourceIds: recordings.prefix(2).map(SourceSessions.sourceID))
        }
        sessions.syncHistory([history("transcribing")], server: "https://vault")
        XCTAssertTrue(sessions.partlyImported(second, server: "https://vault"), "on its way counts")
        XCTAssertTrue(merge(third), "in progress elsewhere")
        XCTAssertTrue(merge(second))
        sessions.syncHistory([history("complete")], server: "https://vault")
        XCTAssertTrue(merge(third), "imported elsewhere")
        XCTAssertFalse(merge(third, server: "https://other"), "imports are per vault")
    }

    func testEjectsAllRecordersAndPreservesFailuresForRetry() async throws {
        let devices = [first, second]
        var attempts: [URL] = []
        var fail = true
        let firstURL = first.url
        let source = RecorderSources(discover: { _ in devices }, eject: { url in
            attempts.append(url)
            if url == firstURL && fail { throw NSError(domain: "test", code: 1, userInfo: [NSLocalizedDescriptionKey: "Device is busy"]) }
        })
        defer { source.stop() }
        source.refresh()
        try await eventually { !source.isScanning }
        let failures = await source.ejectAll(isCopying: false)
        XCTAssertEqual(failures, ["DJI: Device is busy"])
        XCTAssertEqual(attempts, [first.url, second.url])
        XCTAssertEqual(source.devices, [first])
        fail = false
        let retry = await source.ejectAll(isCopying: false)
        XCTAssertTrue(retry.isEmpty)
        XCTAssertTrue(source.devices.isEmpty)
    }

    func testCopyingAndRepeatedClicksCannotEject() async throws {
        let devices = [first]
        var calls = 0
        var release: CheckedContinuation<Void, Never>?
        let source = RecorderSources(discover: { _ in devices }, eject: { _ in
            calls += 1
            await withCheckedContinuation { release = $0 }
        })
        defer { source.stop() }
        source.refresh()
        try await eventually { !source.isScanning }
        let copying = await source.ejectAll(isCopying: true)
        XCTAssertFalse(copying.isEmpty)
        XCTAssertEqual(calls, 0)
        let ejecting = Task { await source.ejectAll(isCopying: false) }
        try await eventually { release != nil }
        _ = await source.ejectAll(isCopying: false)
        XCTAssertEqual(calls, 1)
        release?.resume()
        _ = await ejecting.value
        XCTAssertFalse(source.isEjecting)
    }

    func testMountDuringEjectIsDiscoveredAfterEjectCompletes() async throws {
        let inventory = Inventory([first])
        let notifications = NotificationCenter()
        var release: CheckedContinuation<Void, Never>?
        let source = RecorderSources(notifications: notifications, discover: { _ in await inventory.read() }, eject: { _ in
            await withCheckedContinuation { release = $0 }
        })
        defer { source.stop() }
        source.start(timezone: "UTC")
        try await eventually { !source.isScanning }
        let ejecting = Task { await source.ejectAll(isCopying: false) }
        try await eventually { release != nil }
        await inventory.replace([second])
        notifications.post(name: NSWorkspace.didMountNotification, object: nil, userInfo: [NSWorkspace.volumeURLUserInfoKey: second.url])
        for _ in 0..<4 { await Task.yield() }
        release?.resume()
        _ = await ejecting.value
        try await eventually { source.devices == [second] && !source.isScanning }
    }

    func testManuallyChosenFolderIsNeverAnEjectTarget() async throws {
        let folder = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        try FileManager.default.createDirectory(at: folder, withIntermediateDirectories: true)
        defer { try? FileManager.default.removeItem(at: folder) }
        let devices = [first]
        var ejected: [URL] = []
        let source = RecorderSources(discover: { _ in devices }, eject: { ejected.append($0) })
        defer { source.stop() }
        source.chooseFolder(folder)
        source.refresh()
        try await eventually { !source.isScanning }
        _ = await source.ejectAll(isCopying: false)
        XCTAssertEqual(ejected, [first.url])
        XCTAssertEqual(source.folder, folder)
    }

    func testScanMatchesDJIFilenamesAndIgnoresUnrelatedOrDeepFiles() async throws {
        let folder = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        try FileManager.default.createDirectory(at: folder, withIntermediateDirectories: true)
        defer { try? FileManager.default.removeItem(at: folder) }
        let name = "TX01_MIC002_20261001_204349_orig.wav"
        try Data([0]).write(to: folder.appendingPathComponent(name))
        try Data([0]).write(to: folder.appendingPathComponent("unrelated.wav"))
        try Data([0]).write(to: folder.appendingPathComponent("TX01_MIC003_20260230_204349.wav"))
        let deep = folder.appendingPathComponent("one/two/three/four", isDirectory: true)
        try FileManager.default.createDirectory(at: deep, withIntermediateDirectories: true)
        try Data([0]).write(to: deep.appendingPathComponent(name))
        try FileManager.default.createSymbolicLink(at: folder.appendingPathComponent("TX02_MIC001_20261001_204349.wav"), withDestinationURL: folder.appendingPathComponent(name))
        let files = await RecorderScanner.files(in: folder, djiOnly: true, timezone: TimeZone(secondsFromGMT: 0)!)
        XCTAssertEqual(files.map(\.name), [name])
        XCTAssertEqual(files.first?.transmitter, "TX01")
        XCTAssertEqual(files.first?.size, 1)
        let manual = await RecorderScanner.files(in: folder, djiOnly: false, timezone: .current)
        XCTAssertTrue(manual.contains { $0.name == "unrelated.wav" })
    }
}
