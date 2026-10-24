import AppKit
import Combine
import Foundation

struct RecorderFile: Identifiable, Equatable, Sendable {
    let url: URL
    let size: Int64
    let modified: Date?
    let recorded: Date?
    let transmitter: String?
    var recording: SourceRecording? = nil
    var inspectionError: String? = nil

    var id: String { "\(url.path)|\(size)|\(modified?.timeIntervalSince1970 ?? 0)" }
    var name: String { url.lastPathComponent }
}

struct RecorderDevice: Identifiable, Equatable, Sendable {
    let url: URL
    let files: [RecorderFile]
    var id: String { url.path }
    var name: String {
        let microphones = Set(files.compactMap(\.transmitter)).sorted()
        return microphones.isEmpty ? url.lastPathComponent : microphones.joined(separator: ", ")
    }
}

enum RecorderScanner {
    static let supportedExtensions: Set<String> = ["wav", "flac", "mp3", "m4a"]
    private static let filename = try! NSRegularExpression(
        pattern: #"^([A-Za-z]+[0-9]+)_MIC[0-9]+_([0-9]{8})_([0-9]{6})(?:_[A-Za-z0-9]+)*\.wav$"#,
        options: .caseInsensitive)

    static func djiName(_ name: String, timezone: TimeZone) -> (String, Date)? {
        guard let match = filename.firstMatch(in: name, range: NSRange(name.startIndex..., in: name)) else { return nil }
        func group(_ index: Int) -> String { String(name[Range(match.range(at: index), in: name)!]) }
        let stamp = group(2) + group(3)
        func number(_ offset: Int, _ length: Int) -> Int {
            let start = stamp.index(stamp.startIndex, offsetBy: offset)
            return Int(stamp[start..<stamp.index(start, offsetBy: length)]) ?? -1
        }
        let components = DateComponents(year: number(0, 4), month: number(4, 2), day: number(6, 2), hour: number(8, 2), minute: number(10, 2), second: number(12, 2))
        var utc = Calendar(identifier: .gregorian)
        utc.timeZone = TimeZone(secondsFromGMT: 0)!
        guard components.isValidDate(in: utc) else { return nil }
        var calendar = Calendar(identifier: .gregorian)
        calendar.timeZone = timezone
        guard let date = calendar.date(from: components) else { return nil }
        return (group(1).uppercased(), date)
    }

    /// Bounded, read-only scan, matching the reference app's recorder layout.
    static func files(in folder: URL, djiOnly: Bool, timezone: TimeZone) async -> [RecorderFile] {
        let task = Task.detached(priority: .userInitiated) {
            scanFiles(in: folder, djiOnly: djiOnly, timezone: timezone)
        }
        let files = await withTaskCancellationHandler { await task.value } onCancel: { task.cancel() }
        guard !Task.isCancelled else { return [] }
        // Header inspection is independent, bounded work; no audio conversion runs here.
        return await withTaskGroup(of: (Int, RecorderFile).self) { group in
            var result = files
            var next = 0
            func add(_ index: Int) {
                let file = files[index]
                group.addTask {
                    let inspection = Task.detached(priority: .userInitiated) { inspect(file, timezone: timezone) }
                    return await (index, inspection.value)
                }
            }
            while next < min(4, files.count) { add(next); next += 1 }
            while let (index, file) = await group.next() {
                result[index] = file
                if Task.isCancelled { group.cancelAll() }
                else if next < files.count { add(next); next += 1 }
            }
            return result
        }
    }

    private static func inspect(_ file: RecorderFile, timezone: TimeZone) -> RecorderFile {
        var result = file
        do {
            let inspected = try AudioInspector.inspect(file.url)
            let name = DJIFilename(file.name)
            result.recording = SourceRecording(url: file.url, filename: file.name,
                transmitter: name?.transmitter ?? "Audio", variant: name?.variant ?? file.url.pathExtension.lowercased(),
                sequence: name?.sequence ?? 0, start: name?.startDate(in: timezone) ?? file.recorded ?? .distantPast,
                frameCount: inspected.frameCount, fileSize: file.size, format: inspected.format)
        } catch { result.inspectionError = error.localizedDescription }
        return result
    }

    private static func scanFiles(in folder: URL, djiOnly: Bool, timezone: TimeZone) -> [RecorderFile] {
        let scoped = folder.startAccessingSecurityScopedResource()
        defer { if scoped { folder.stopAccessingSecurityScopedResource() } }
        let keys: Set<URLResourceKey> = [.isRegularFileKey, .isSymbolicLinkKey, .fileSizeKey, .contentModificationDateKey, .creationDateKey]
        guard let enumerator = FileManager.default.enumerator(at: folder, includingPropertiesForKeys: Array(keys), options: [.skipsHiddenFiles, .skipsPackageDescendants]) else { return [RecorderFile]() }
        var found: [RecorderFile] = []
        for case let url as URL in enumerator {
            if Task.isCancelled { break }
            if enumerator.level > 3 { enumerator.skipDescendants(); continue }
            guard supportedExtensions.contains(url.pathExtension.lowercased()),
                  let values = try? url.resourceValues(forKeys: keys),
                  values.isRegularFile == true, values.isSymbolicLink != true else { continue }
            let parsed = djiName(url.lastPathComponent, timezone: timezone)
            if djiOnly && parsed == nil { continue }
            found.append(RecorderFile(url: url, size: Int64(values.fileSize ?? 0), modified: values.contentModificationDate,
                                      recorded: parsed?.1 ?? values.creationDate, transmitter: parsed?.0))
        }
        return found.sorted {
            if $0.recorded != $1.recorded { return ($0.recorded ?? .distantPast) > ($1.recorded ?? .distantPast) }
            return $0.url.path < $1.url.path
        }
    }

    static func devices(timezone: TimeZone) async -> [RecorderDevice] {
        let keys: Set<URLResourceKey> = [.volumeIsInternalKey, .volumeIsLocalKey]
        let volumes = FileManager.default.mountedVolumeURLs(includingResourceValuesForKeys: Array(keys), options: .skipHiddenVolumes) ?? []
        var devices: [RecorderDevice] = []
        for volume in volumes {
            if Task.isCancelled { break }
            guard let values = try? volume.resourceValues(forKeys: keys),
                  values.volumeIsInternal != true, values.volumeIsLocal != false else { continue }
            let recordings = await files(in: volume, djiOnly: true, timezone: timezone)
            if !recordings.isEmpty { devices.append(RecorderDevice(url: volume, files: recordings)) }
        }
        return devices.sorted { $0.id < $1.id }
    }
}

@MainActor
final class RecorderSources: ObservableObject {
    @Published private(set) var devices: [RecorderDevice] = []
    @Published private(set) var folder: URL?
    @Published private(set) var folderFiles: [RecorderFile] = []
    @Published private(set) var isScanning = false
    @Published private(set) var isReadingFolder = false
    @Published private(set) var isEjecting = false
    private var timezone = TimeZone.current
    private let notifications: NotificationCenter
    private let discover: @Sendable (TimeZone) async -> [RecorderDevice]
    private let eject: (URL) async throws -> Void
    private var observers: [NSObjectProtocol] = []
    private var scanTask: Task<Void, Never>?
    private var folderTask: Task<Void, Never>?
    private var generation = UUID()
    private var refreshAfterEject = false

    init(notifications: NotificationCenter = NSWorkspace.shared.notificationCenter,
         discover: @escaping @Sendable (TimeZone) async -> [RecorderDevice] = { await RecorderScanner.devices(timezone: $0) },
         eject: @escaping (URL) async throws -> Void = { url in
             try await Task.detached { try NSWorkspace.shared.unmountAndEjectDevice(at: url) }.value
         }) {
        self.notifications = notifications
        self.discover = discover
        self.eject = eject
    }

    var files: [RecorderFile] { devices.flatMap(\.files).sorted { ($0.recorded ?? .distantPast) > ($1.recorded ?? .distantPast) } }

    func start(timezone: String) {
        self.timezone = TimeZone(identifier: timezone) ?? .current
        guard observers.isEmpty else { return }
        observers.append(notifications.addObserver(forName: NSWorkspace.didMountNotification, object: nil, queue: .main) { [weak self] _ in
            Task { @MainActor in self?.refresh() }
        })
        observers.append(notifications.addObserver(forName: NSWorkspace.didUnmountNotification, object: nil, queue: .main) { [weak self] notification in
            guard let url = notification.userInfo?[NSWorkspace.volumeURLUserInfoKey] as? URL else { return }
            Task { @MainActor in self?.unmounted(url) }
        })
        observers.append(notifications.addObserver(forName: NSWorkspace.didActivateApplicationNotification, object: nil, queue: .main) { [weak self] notification in
            guard let app = notification.userInfo?[NSWorkspace.applicationUserInfoKey] as? NSRunningApplication,
                  app.processIdentifier == ProcessInfo.processInfo.processIdentifier else { return }
            Task { @MainActor in self?.refresh() }
        })
        if let path = UserDefaults.standard.string(forKey: "recorder-source-folder"), FileManager.default.fileExists(atPath: path) {
            chooseFolder(URL(fileURLWithPath: path))
        }
        refresh()
    }

    func stop() {
        observers.forEach { notifications.removeObserver($0) }
        observers = []
        generation = UUID()
        scanTask?.cancel()
        folderTask?.cancel()
        isScanning = false
        isReadingFolder = false
    }

    func setTimezone(_ identifier: String) {
        timezone = TimeZone(identifier: identifier) ?? .current
        refresh()
        if let folder { chooseFolder(folder) }
    }

    func refresh() {
        if isEjecting { refreshAfterEject = true; return }
        scanTask?.cancel()
        generation = UUID()
        let generation = generation, timezone = timezone, discover = discover
        isScanning = true
        scanTask = Task { [weak self] in
            let devices = await discover(timezone)
            guard let self, !Task.isCancelled, self.generation == generation else { return }
            self.devices = devices
            self.isScanning = false
        }
    }

    func chooseFolder(_ url: URL) {
        guard !isEjecting else { return }
        if folder != url { folderFiles = [] }
        folder = url
        UserDefaults.standard.set(url.path, forKey: "recorder-source-folder")
        isReadingFolder = true
        folderTask?.cancel()
        let timezone = timezone
        folderTask = Task { [weak self] in
            let files = await RecorderScanner.files(in: url, djiOnly: false, timezone: timezone)
            guard let self, !Task.isCancelled, self.folder == url else { return }
            self.folderFiles = files
            self.isReadingFolder = false
        }
    }

    private func unmounted(_ volume: URL) {
        // Invalidate an in-flight discovery so it cannot restore a volume that just disappeared.
        generation = UUID()
        scanTask?.cancel()
        isScanning = false
        devices.removeAll { $0.url == volume }
        if let folder, folder.path == volume.path || folder.path.hasPrefix(volume.path + "/") {
            folderTask?.cancel()
            self.folder = nil
            folderFiles = []
            isReadingFolder = false
        }
        if !isEjecting { refresh() }
    }

    /// Only discovered recorder volume roots are targets. Never force an unmount.
    func ejectAll(isCopying: Bool) async -> [String] {
        guard !isCopying else { return ["Wait for the recordings to finish copying before ejecting the microphones."] }
        guard !isEjecting, !devices.isEmpty else { return [] }
        isEjecting = true
        generation = UUID()
        scanTask?.cancel()
        folderTask?.cancel()
        await scanTask?.value
        await folderTask?.value
        isScanning = false
        isReadingFolder = false
        defer {
            isEjecting = false
            if refreshAfterEject { refreshAfterEject = false; refresh() }
        }
        let volumes = devices.map(\.url)
        var failures: [String] = []
        for volume in volumes {
            guard devices.contains(where: { $0.url == volume }) else { continue }
            do {
                try await eject(volume)
                devices.removeAll { $0.url == volume }
                if let folder, folder.path == volume.path || folder.path.hasPrefix(volume.path + "/") {
                    self.folder = nil
                    folderFiles = []
                }
            } catch {
                failures.append("\(volume.lastPathComponent): \(error.localizedDescription)")
            }
        }
        return failures
    }
}
