import AppKit
import XCTest
@testable import TranscriptVault

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
        defer { source.stop() }
        source.start(timezone: "UTC")
        try await eventually { !source.isScanning }
        XCTAssertEqual(source.devices, [first])

        await inventory.replace([first, second])
        notifications.post(name: NSWorkspace.didMountNotification, object: nil, userInfo: [NSWorkspace.volumeURLUserInfoKey: second.url])
        try await eventually { source.devices.count == 2 }
        XCTAssertEqual(source.devices, [first, second])

        await inventory.replace([second])
        notifications.post(name: NSWorkspace.didUnmountNotification, object: nil, userInfo: [NSWorkspace.volumeURLUserInfoKey: first.url])
        try await eventually { source.devices == [second] && !source.isScanning }
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
