import Foundation
import Combine

public struct ImportSessionPlan: Codable, Sendable {
    public let sourceIDs: [String]
    public let trimStartFrame: Int64?
    public let trimEndFrame: Int64?
    enum CodingKeys: String, CodingKey {
        case sourceIDs = "source_ids", trimStartFrame = "trim_start_frame", trimEndFrame = "trim_end_frame"
    }
    public init(sourceIDs: [String], trimStartFrame: Int64? = nil, trimEndFrame: Int64? = nil) {
        self.sourceIDs = sourceIDs; self.trimStartFrame = trimStartFrame; self.trimEndFrame = trimEndFrame
    }
}

typealias TransferCopy = @Sendable (URL, URL) async throws -> Void
typealias TransferPreparation = @Sendable (URL, URL) async throws -> URL
typealias TransferRequest = @Sendable (URLRequest, URL?) async throws -> (Data, URLResponse)

public struct UploadItem: Codable, Identifiable, Sendable {
    public var id = UUID().uuidString
    public var name: String
    public var source: String
    public var sourceID: String? = nil
    public var originalURL: URL? = nil
    public var preparationProgress: Double? = nil
    public var estimatedSecondsRemaining: Double? = nil
    public var prepared: String?
    public var location: String?
    public var offset: Int64 = 0
    public var length: Int64 = 0
    public var state = "waiting"
    public var error: String?
}
public struct QueueState: Codable, Sendable {
    public var server = ""
    public var timezone = TimeZone.current.identifier
    public var importID: String?
    public var sessions: [ImportSessionPlan]? = nil
    public var state = "idle"
    public var error: String?
    public var files: [UploadItem] = []
}

/// The JSON journal and audio spool live in Application Support. Only upload chunks
/// enter URLSession, so a recording never needs to be loaded into JS or memory.
@MainActor public final class TransferQueue: NSObject, ObservableObject, URLSessionTaskDelegate, URLSessionDataDelegate {
    public static let shared = TransferQueue()
    // Keep the original background-session identifier so pending uploads reconnect after upgrades.
    public static let sessionIdentifier = "dev.wsky.transcript-vault.uploads"
    @Published public private(set) var snapshot = QueueState()
    @Published public private(set) var activeSourceCopies = 0
    public var isReadingSource: Bool { activeSourceCopies > 0 || snapshot.state == "copying" }
    public var backgroundCompletion: (() -> Void)?
    private let folder: URL
    private var runner: Task<Void, Never>?
    private var runnerID = UUID()
    private let tokenOverride: String?
    private let copyOverride: TransferCopy?
    private let preparationOverride: TransferPreparation?
    private let requestOverride: TransferRequest?
    private var resumeAfterStopping = false
    #if os(macOS)
    private lazy var macUploadSession: URLSession = {
        let configuration = URLSessionConfiguration.default
        configuration.timeoutIntervalForResource = 7 * 24 * 3600
        configuration.httpMaximumConnectionsPerHost = 3
        return URLSession(configuration: configuration)
    }()
    #endif
    private var responseData: [Int: Data] = [:]
    private lazy var session: URLSession = {
        #if os(iOS)
        let configuration = URLSessionConfiguration.background(withIdentifier: Self.sessionIdentifier)
        configuration.sessionSendsLaunchEvents = true
        configuration.isDiscretionary = false
        #else
        let configuration = URLSessionConfiguration.default
        #endif
        configuration.timeoutIntervalForResource = 7 * 24 * 3600
        return URLSession(configuration: configuration, delegate: self, delegateQueue: .main)
    }()
    public override convenience init() {
        // Fundus keeps the original spool location so queued recordings survive the rename.
        let folder = FileManager.default.urls(for: .applicationSupportDirectory, in: .userDomainMask)[0].appendingPathComponent("TranscriptVault/Queue", isDirectory: true)
        self.init(folder: folder)
    }
    init(folder: URL, initialState: QueueState? = nil, tokenOverride: String? = nil,
         copy: TransferCopy? = nil, preparation: TransferPreparation? = nil, request: TransferRequest? = nil) {
        self.folder = folder
        self.tokenOverride = tokenOverride
        self.copyOverride = copy
        self.preparationOverride = preparation
        self.requestOverride = request
        super.init()
        try? FileManager.default.createDirectory(at: folder, withIntermediateDirectories: true)
        try? FileManager.default.setAttributes([.posixPermissions: 0o700], ofItemAtPath: folder.path)
        var privateFolder = folder
        var values = URLResourceValues(); values.isExcludedFromBackup = true
        try? privateFolder.setResourceValues(values)
        if let initialState { snapshot = initialState }
        else if let data = try? Data(contentsOf: journal), let saved = try? JSONDecoder().decode(QueueState.self, from: data) { snapshot = saved }
        #if os(macOS)
        if ["copying", "uploading", "verifying"].contains(snapshot.state) {
            Task { @MainActor in self.start() }
        }
        #else
        _ = session
        session.getAllTasks { tasks in
            Task { @MainActor in
                if tasks.isEmpty && self.snapshot.state == "uploading" { self.start() }
            }
        }
        #endif
    }
    private var journal: URL { folder.appendingPathComponent("queue.json") }
    private func save() { do { try JSONEncoder().encode(snapshot).write(to: journal, options: [.atomic, .completeFileProtectionUntilFirstUserAuthentication]); let handle = try FileHandle(forWritingTo: journal); try handle.synchronize(); try handle.close() } catch { snapshot.error = "Could not save the transfer queue: \(error.localizedDescription)"; snapshot.state = "failed" } }
    public func configure(server: String, token: String, timezone: String) throws {
        guard ["idle", "verified"].contains(snapshot.state) || server == snapshot.server else { throw failure("Finish or clear the current queue before changing servers.") }
        _ = try VaultConnection(server: server, token: token)
        guard TimeZone(identifier: timezone) != nil else { throw failure("Choose a valid recorder timezone.") }
        try VaultCredentials.save(token)
        snapshot.server = server; snapshot.timezone = timezone; save()
    }
    public func configureTimezone(_ timezone: String) throws {
        guard ["idle", "verified"].contains(snapshot.state) else { throw failure("Finish or clear the current queue before changing the recorder clock.") }
        guard TimeZone(identifier: timezone) != nil else { throw failure("Choose a valid recorder timezone.") }
        snapshot.timezone = timezone; save()
    }
    #if os(macOS)
    public func configureNative(server: String, timezone: String) async throws {
        guard ["idle", "verified"].contains(snapshot.state) || server == snapshot.server else { throw failure("Finish or clear the current queue before changing servers.") }
        _ = try await NativeVaultSession.connection(server: server)
        guard TimeZone(identifier: timezone) != nil else { throw failure("Choose a valid recorder timezone.") }
        snapshot.server = server; snapshot.timezone = timezone; save()
    }
    #endif
    public func enqueue(_ urls: [URL], sessions: [ImportSessionPlan] = [], sourceIDs: [String: String] = [:]) async throws {
        guard ["idle", "verified"].contains(snapshot.state), !urls.isEmpty else { throw failure("Finish or clear the current queue first.") }
        guard !snapshot.server.isEmpty else { throw failure("Connect to your vault first.") }
        #if os(macOS)
        _ = try await macConnection()
        #else
        guard VaultCredentials.read() != nil else { throw failure("Connect to your vault first.") }
        #endif
        guard Set(urls.map(\.path)).count == urls.count else { throw failure("Select each source recording only once.") }
        for url in urls {
            guard ["wav", "flac", "mp3", "m4a"].contains(url.pathExtension.lowercased()) else { throw failure("Select WAV, FLAC, MP3, or M4A recordings.") }
        }
        if !sessions.isEmpty {
            let selectedIDs = urls.compactMap { sourceIDs[$0.path] }
            let plannedIDs = sessions.flatMap(\.sourceIDs)
            guard selectedIDs.count == urls.count, Set(selectedIDs).count == selectedIDs.count,
                  Set(plannedIDs) == Set(selectedIDs), plannedIDs.count == selectedIDs.count,
                  sessions.allSatisfy({ !$0.sourceIDs.isEmpty && ($0.trimStartFrame ?? 0) >= 0 && ($0.trimEndFrame ?? 0) >= 0 && ($0.trimEndFrame == nil || $0.trimEndFrame! > ($0.trimStartFrame ?? 0)) }) else {
                throw failure("The recording selection changed. Select the sessions again before uploading.")
            }
        }
        let server = snapshot.server, timezone = snapshot.timezone
        try clear()
        snapshot.server = server; snapshot.timezone = timezone; snapshot.sessions = sessions.isEmpty ? nil : sessions
        snapshot.state = "copying"
        #if os(macOS)
        // Register the complete batch before work starts, including originals still to copy.
        // That makes a paused or interrupted copy resumable without silently losing sources.
        snapshot.files = urls.map { url in
            var item = UploadItem(name: url.lastPathComponent, source: UUID().uuidString + "." + url.pathExtension.lowercased())
            item.sourceID = sourceIDs[url.path]; item.originalURL = url; item.state = "copying"
            return item
        }
        save(); start()
        #else
        save()
        do {
            for url in urls {
                #if os(iOS) || os(macOS)
                let scoped = url.startAccessingSecurityScopedResource()
                defer { if scoped { url.stopAccessingSecurityScopedResource() } }
                #endif
                let ext = url.pathExtension.lowercased()
                guard ["wav", "flac", "mp3", "m4a"].contains(ext) else { throw failure("Select WAV, FLAC, MP3, or M4A recordings.") }
                let name = UUID().uuidString + "." + ext
                let destination = folder.appendingPathComponent(name)
                // Copy while the document picker grant is still valid; originals stay untouched.
                try await Task.detached { try FileManager.default.copyItem(at: url, to: destination) }.value
                var item = UploadItem(name: url.lastPathComponent, source: name)
                item.sourceID = sourceIDs[url.path]
                snapshot.files.append(item); save()
            }
            snapshot.state = "uploading"; save(); start()
        } catch { snapshot.state = "failed"; snapshot.error = error.localizedDescription; save(); throw error }
        #endif
    }
    public func resume() {
        guard !snapshot.files.isEmpty else { return }
        #if os(macOS)
        if runner != nil { resumeAfterStopping = true; return }
        snapshot.error = nil
        snapshot.state = snapshot.files.contains(where: { $0.originalURL != nil }) ? "copying" : "uploading"
        #else
        snapshot.error = nil; snapshot.state = "uploading"
        #endif
        save(); start()
    }
    public func pause() {
        #if os(macOS)
        resumeAfterStopping = false
        runner?.cancel()
        // Keep the copying state until its file handles have closed; eject stays disabled.
        if snapshot.state != "copying" { snapshot.state = "paused"; save() }
        #else
        snapshot.state = "paused"; runner?.cancel(); runner = nil; save()
        session.getAllTasks { tasks in tasks.forEach { $0.cancel() } }
        #endif
    }
    public func clear() throws {
        #if os(macOS)
        guard runner == nil else { throw failure("Wait for preparation and transfers to pause before clearing the queue.") }
        resumeAfterStopping = false
        #endif
        guard ["idle", "verified", "failed", "paused"].contains(snapshot.state) else { throw failure("Pause the queue before clearing it.") }
        runner?.cancel(); runner = nil
        #if os(iOS)
        session.getAllTasks { tasks in tasks.forEach { $0.cancel() } }
        #endif
        for url in (try? FileManager.default.contentsOfDirectory(at: folder, includingPropertiesForKeys: nil)) ?? [] { try? FileManager.default.removeItem(at: url) }
        let server = snapshot.server, timezone = snapshot.timezone
        snapshot = QueueState(); snapshot.server = server; snapshot.timezone = timezone; save()
    }
    public func json() throws -> String { String(decoding: try JSONEncoder().encode(snapshot), as: UTF8.self) }
    private func connection() throws -> VaultConnection {
        guard let token = tokenOverride ?? VaultCredentials.read() else { throw failure("Reconnect with an upload token.") }
        return try VaultConnection(server: snapshot.server, token: token)
    }
    private func start() {
        #if os(macOS)
        guard runner == nil, ["copying", "uploading", "verifying"].contains(snapshot.state) else { return }
        #else
        guard runner == nil, snapshot.state == "uploading" else { return }
        #endif
        let currentID = UUID(); runnerID = currentID
        runner = Task {
            defer {
                if runnerID == currentID {
                    runner = nil
                    #if os(macOS)
                    if resumeAfterStopping { resumeAfterStopping = false; resume() }
                    #endif
                }
            }
            do {
                #if os(macOS)
                try await advanceMac()
                #else
                try await advance()
                #endif
            } catch {
                if runnerID == currentID {
                    #if os(macOS)
                    if Task.isCancelled { snapshot.state = "paused"; snapshot.error = nil }
                    else { snapshot.state = "failed"; snapshot.error = error.localizedDescription }
                    save()
                    #else
                    if snapshot.state != "paused" { snapshot.state = "failed"; snapshot.error = error.localizedDescription; save() }
                    #endif
                }
            }
        }
    }
    private nonisolated static func importBody(timezone: String, sessions: [ImportSessionPlan]?) -> sending [String: Any] {
        var body: [String: Any] = ["recorder_timezone": timezone]
        if let sessions {
            body["sessions"] = sessions.map { plan -> [String: Any] in
                var value: [String: Any] = ["source_ids": plan.sourceIDs]
                if let start = plan.trimStartFrame { value["trim_start_frame"] = start }
                if let end = plan.trimEndFrame { value["trim_end_frame"] = end }
                return value
            }
        }
        return body
    }
    private func advance() async throws {
        let connection = try connection()
        if snapshot.importID == nil {
            let response = try await connection.json("/api/v1/imports", method: "POST", body: Self.importBody(timezone: snapshot.timezone, sessions: snapshot.sessions))
            try Task.checkCancellation()
            guard let id = response["id"] as? String else { throw failure("Invalid import response.") }
            snapshot.importID = id; save()
        }
        for index in snapshot.files.indices {
            try Task.checkCancellation()
            if snapshot.files[index].state == "uploaded" { continue }
            if snapshot.files[index].prepared == nil {
                snapshot.files[index].state = "preparing"; save()
                let source = folder.appendingPathComponent(snapshot.files[index].source), destination = folder
                let preparation = Task.detached { try AudioPreparer.prepare(source, in: destination) { _ in } }
                let output = try await withTaskCancellationHandler { try await preparation.value } onCancel: { preparation.cancel() }
                try Task.checkCancellation()
                snapshot.files[index].prepared = output.lastPathComponent
                snapshot.files[index].length = Int64(try output.resourceValues(forKeys: [.fileSizeKey]).fileSize ?? 0)
                snapshot.files[index].state = "ready"; save()
            }
            if snapshot.files[index].location == nil {
                var request = try connection.request("/api/v1/uploads", method: "POST")
                request.setValue("1.0.0", forHTTPHeaderField: "Tus-Resumable")
                request.setValue(String(snapshot.files[index].length), forHTTPHeaderField: "Upload-Length")
                var metadata = ["import_id": snapshot.importID!, "filename": snapshot.files[index].prepared!, "original_name": snapshot.files[index].name]
                if let sourceID = snapshot.files[index].sourceID { metadata["source_id"] = sourceID }
                request.setValue(metadata.sorted { $0.key < $1.key }.map { "\($0.key) \(Data($0.value.utf8).base64EncodedString())" }.joined(separator: ","), forHTTPHeaderField: "Upload-Metadata")
                let (data, response) = try await URLSession.shared.data(for: request)
                try Task.checkCancellation()
                try VaultConnection.check(response, data)
                guard let location = (response as? HTTPURLResponse)?.value(forHTTPHeaderField: "Location") else { throw failure("Missing upload location.") }
                _ = try connection.request(location)
                snapshot.files[index].location = location; save()
            }
            var head = try connection.request(snapshot.files[index].location!, method: "HEAD")
            head.setValue("1.0.0", forHTTPHeaderField: "Tus-Resumable")
            let (_, response) = try await URLSession.shared.data(for: head); try Task.checkCancellation(); try VaultConnection.check(response)
            guard let offset = Int64((response as? HTTPURLResponse)?.value(forHTTPHeaderField: "Upload-Offset") ?? ""), offset <= snapshot.files[index].length else { throw failure("Invalid upload offset.") }
            snapshot.files[index].offset = offset
            if offset == snapshot.files[index].length { snapshot.files[index].state = "uploaded"; save(); continue }
            let file = try FileHandle(forReadingFrom: folder.appendingPathComponent(snapshot.files[index].prepared!))
            defer { try? file.close() }
            try file.seek(toOffset: UInt64(offset))
            guard let chunk = try file.read(upToCount: 8 * 1024 * 1024), !chunk.isEmpty else { throw failure("Prepared recording is missing. Select the original again.") }
            let chunkURL = folder.appendingPathComponent("chunk-\(snapshot.files[index].id)")
            try chunk.write(to: chunkURL, options: [.atomic, .completeFileProtectionUntilFirstUserAuthentication])
            var request = try connection.request(snapshot.files[index].location!, method: "PATCH")
            request.setValue("1.0.0", forHTTPHeaderField: "Tus-Resumable")
            request.setValue(String(offset), forHTTPHeaderField: "Upload-Offset")
            request.setValue("application/offset+octet-stream", forHTTPHeaderField: "Content-Type")
            snapshot.files[index].state = "uploading"; save()
            let task = session.uploadTask(with: request, fromFile: chunkURL)
            task.taskDescription = snapshot.files[index].id; task.resume()
            return // The native session delegate schedules the next bounded chunk.
        }
        _ = try await connection.json("/api/v1/imports/\(snapshot.importID!)/complete", method: "POST")
        snapshot.state = "verifying"; save()
        try await pollReceipt()
    }
    #if os(macOS)
    private enum MacOperation: Sendable { case copied(String), prepared(String), uploaded(String) }

    private func advanceMac() async throws {
        if snapshot.state == "verifying" { try await pollReceipt(); return }
        snapshot.state = snapshot.files.contains(where: { $0.originalURL != nil }) ? "copying" : "uploading"
        save()
        let connection = try await macConnection()
        if snapshot.importID == nil {
            let response = try await macJSON(connection, "/api/v1/imports", method: "POST", body: Self.importBody(timezone: snapshot.timezone, sessions: snapshot.sessions))
            guard let id = response["id"] as? String else { throw failure("Invalid import response.") }
            snapshot.importID = id; save()
            try Task.checkCancellation()
        }
        // The complete selection and manifest were journaled before starting.
        // Copy, encode and transfer overlap per file, each with independent bounds.
        // Keep the global copying state until every recorder file handle is closed.
        try await withThrowingTaskGroup(of: MacOperation.self) { group in
            var copying = Set<String>(), preparing = Set<String>(), uploading = Set<String>()
            while true {
                try Task.checkCancellation()
                guard ["copying", "uploading"].contains(snapshot.state) else { throw CancellationError() }
                let phase = snapshot.files.contains(where: { $0.originalURL != nil }) ? "copying" : "uploading"
                if snapshot.state != phase { snapshot.state = phase; save() }
                for item in snapshot.files where item.originalURL != nil && !copying.contains(item.id) {
                    if copying.count >= 2 { break }
                    copying.insert(item.id)
                    group.addTask { [id = item.id] in
                        try await self.copyMacFile(id)
                        return .copied(id)
                    }
                }
                for item in snapshot.files where item.prepared != nil && item.state != "uploaded" && !uploading.contains(item.id) {
                    if uploading.count >= 3 { break }
                    uploading.insert(item.id)
                    group.addTask { [id = item.id] in
                        try await self.uploadMacFile(id, connection: connection)
                        return .uploaded(id)
                    }
                }
                for item in snapshot.files where item.originalURL == nil && item.prepared == nil && !preparing.contains(item.id) {
                    if preparing.count >= 2 { break }
                    preparing.insert(item.id)
                    group.addTask { [id = item.id] in
                        try await self.prepareMacFile(id)
                        return .prepared(id)
                    }
                }
                guard !copying.isEmpty || !preparing.isEmpty || !uploading.isEmpty else { break }
                guard let finished = try await group.next() else { break }
                switch finished {
                case .copied(let id): copying.remove(id)
                case .prepared(let id): preparing.remove(id)
                case .uploaded(let id): uploading.remove(id)
                }
            }
        }
        try Task.checkCancellation()
        guard snapshot.files.allSatisfy({ $0.state == "uploaded" }), let importID = snapshot.importID else {
            throw failure("Not every recording finished uploading. Resume to continue.")
        }
        _ = try await macJSON(connection, "/api/v1/imports/\(importID)/complete", method: "POST")
        snapshot.state = "verifying"; save()
        try await pollReceipt()
    }

    private func copyMacFile(_ id: String) async throws {
        guard let index = snapshot.files.firstIndex(where: { $0.id == id }), let original = snapshot.files[index].originalURL else { throw CancellationError() }
        let destination = folder.appendingPathComponent(snapshot.files[index].source)
        activeSourceCopies += 1
        defer { activeSourceCopies -= 1 }
        if let copyOverride { try await copyOverride(original, destination) }
        else {
            let copy = Task.detached(priority: .userInitiated) { try Self.copySource(original, to: destination) }
            try await withTaskCancellationHandler { try await copy.value } onCancel: { copy.cancel() }
        }
        snapshot.files[index].originalURL = nil
        snapshot.files[index].state = "waiting"
        save()
    }

    private nonisolated static func copySource(_ source: URL, to destination: URL) throws {
        let scoped = source.startAccessingSecurityScopedResource()
        defer { if scoped { source.stopAccessingSecurityScopedResource() } }
        let partial = destination.appendingPathExtension("copying")
        try? FileManager.default.removeItem(at: partial)
        do {
            let input = try FileHandle(forReadingFrom: source)
            defer { try? input.close() }
            let size = try source.resourceValues(forKeys: [.fileSizeKey]).fileSize ?? 0
            guard size > 0 else { throw PreparationError.invalidAudio }
            guard FileManager.default.createFile(atPath: partial.path, contents: nil) else { throw CocoaError(.fileWriteUnknown) }
            let output = try FileHandle(forWritingTo: partial)
            defer { try? output.close() }
            var copied = 0
            while let data = try input.read(upToCount: 1024 * 1024), !data.isEmpty {
                try Task.checkCancellation()
                try output.write(contentsOf: data)
                copied += data.count
            }
            try Task.checkCancellation()
            guard copied == size else { throw PreparationError.verificationFailed }
            try output.synchronize()
            try output.close()
            try? FileManager.default.removeItem(at: destination)
            try FileManager.default.moveItem(at: partial, to: destination)
        } catch {
            try? FileManager.default.removeItem(at: partial)
            throw error
        }
    }

    private func prepareMacFile(_ id: String) async throws {
        guard let index = snapshot.files.firstIndex(where: { $0.id == id }) else { throw CancellationError() }
        snapshot.files[index].state = "preparing"
        snapshot.files[index].preparationProgress = 0
        snapshot.files[index].estimatedSecondsRemaining = nil
        save()
        let source = folder.appendingPathComponent(snapshot.files[index].source)
        let output: URL
        if let preparationOverride { output = try await preparationOverride(source, folder) }
        else {
            let destination = folder
            let reporter = PreparationProgressReporter { [weak self] value in
                Task { @MainActor in
                    guard let self, let current = self.snapshot.files.firstIndex(where: { $0.id == id }), self.snapshot.files[current].state == "preparing" else { return }
                    self.snapshot.files[current].preparationProgress = value
                }
            }
            let task = Task.detached(priority: .userInitiated) { try AudioPreparer.prepare(source, in: destination) { reporter.report($0) } }
            output = try await withTaskCancellationHandler { try await task.value } onCancel: { task.cancel() }
        }
        try Task.checkCancellation()
        snapshot.files[index].preparationProgress = 1
        snapshot.files[index].prepared = output.lastPathComponent
        snapshot.files[index].length = Int64(try output.resourceValues(forKeys: [.fileSizeKey]).fileSize ?? 0)
        snapshot.files[index].state = "ready"
        snapshot.files[index].error = nil
        save()
    }

    private func uploadMacFile(_ id: String, connection: VaultConnection) async throws {
        guard let index = snapshot.files.firstIndex(where: { $0.id == id }), let prepared = snapshot.files[index].prepared, let importID = snapshot.importID else { throw CancellationError() }
        if snapshot.files[index].location == nil {
            var request = try connection.request("/api/v1/uploads", method: "POST")
            request.setValue("1.0.0", forHTTPHeaderField: "Tus-Resumable")
            request.setValue(String(snapshot.files[index].length), forHTTPHeaderField: "Upload-Length")
            var metadata = ["import_id": importID, "filename": prepared, "original_name": snapshot.files[index].name]
            if let sourceID = snapshot.files[index].sourceID { metadata["source_id"] = sourceID }
            request.setValue(metadata.sorted { $0.key < $1.key }.map { "\($0.key) \(Data($0.value.utf8).base64EncodedString())" }.joined(separator: ","), forHTTPHeaderField: "Upload-Metadata")
            let (data, response) = try await macRequest(request)
            try VaultConnection.check(response, data)
            guard let location = (response as? HTTPURLResponse)?.value(forHTTPHeaderField: "Location") else { throw failure("Missing upload location.") }
            _ = try connection.request(location)
            // Keep a received server handle even if Pause arrived with its response.
            snapshot.files[index].location = location; save()
        }
        try Task.checkCancellation()
        let location = snapshot.files[index].location!
        var head = try connection.request(location, method: "HEAD")
        head.setValue("1.0.0", forHTTPHeaderField: "Tus-Resumable")
        let (data, response) = try await macRequest(head)
        try VaultConnection.check(response, data)
        guard let remoteOffset = Int64((response as? HTTPURLResponse)?.value(forHTTPHeaderField: "Upload-Offset") ?? ""), remoteOffset >= 0, remoteOffset <= snapshot.files[index].length else { throw failure("Invalid upload offset.") }
        snapshot.files[index].offset = remoteOffset
        snapshot.files[index].estimatedSecondsRemaining = nil
        let started = ProcessInfo.processInfo.systemUptime
        defer { if let current = snapshot.files.firstIndex(where: { $0.id == id }) { snapshot.files[current].estimatedSecondsRemaining = nil } }
        snapshot.files[index].state = "uploading"; save()
        while snapshot.files[index].offset < snapshot.files[index].length {
            try Task.checkCancellation()
            let offset = snapshot.files[index].offset
            let audio = folder.appendingPathComponent(prepared)
            let chunkURL = folder.appendingPathComponent("chunk-\(runnerID)-\(id)")
            defer { try? FileManager.default.removeItem(at: chunkURL) }
            let copy = Task.detached {
                let file = try FileHandle(forReadingFrom: audio)
                defer { try? file.close() }
                try file.seek(toOffset: UInt64(offset))
                guard let bytes = try file.read(upToCount: 8 * 1024 * 1024), !bytes.isEmpty else { throw PreparationError.invalidAudio }
                try bytes.write(to: chunkURL, options: [.atomic, .completeFileProtectionUntilFirstUserAuthentication])
                return bytes.count
            }
            let count = try await withTaskCancellationHandler { try await copy.value } onCancel: { copy.cancel() }
            try Task.checkCancellation()
            var request = try connection.request(location, method: "PATCH")
            request.setValue("1.0.0", forHTTPHeaderField: "Tus-Resumable")
            request.setValue(String(offset), forHTTPHeaderField: "Upload-Offset")
            request.setValue(String(count), forHTTPHeaderField: "Content-Length")
            request.setValue("application/offset+octet-stream", forHTTPHeaderField: "Content-Type")
            let (data, response) = try await macRequest(request, file: chunkURL)
            try VaultConnection.check(response, data)
            guard let confirmed = Int64((response as? HTTPURLResponse)?.value(forHTTPHeaderField: "Upload-Offset") ?? ""), confirmed == offset + Int64(count) else { throw failure("The server returned an unexpected upload offset. Resume to reconcile it.") }
            snapshot.files[index].offset = confirmed
            let elapsed = ProcessInfo.processInfo.systemUptime - started
            let transferred = confirmed - remoteOffset
            if transferred > 0 && elapsed > 0 {
                snapshot.files[index].estimatedSecondsRemaining = Double(snapshot.files[index].length - confirmed) * elapsed / Double(transferred)
            }
            save()
        }
        snapshot.files[index].state = "uploaded"
        snapshot.files[index].error = nil
        save()
    }

    private func macConnection(forceRefresh: Bool = false) async throws -> VaultConnection {
        if let tokenOverride { return try VaultConnection(server: snapshot.server, token: tokenOverride) }
        return try await NativeVaultSession.connection(server: snapshot.server, forceRefresh: forceRefresh)
    }
    private func macRequest(_ original: URLRequest, file: URL? = nil) async throws -> (Data, URLResponse) {
        if let requestOverride { return try await requestOverride(original, file) }
        var request = original
        for attempt in 0...1 {
            let connection = try await macConnection(forceRefresh: attempt > 0)
            guard let url = request.url else { throw failure("Invalid transfer address.") }
            let fresh = try connection.request(url.absoluteString, method: request.httpMethod ?? "GET")
            request.url = fresh.url
            request.setValue("Bearer \(connection.token)", forHTTPHeaderField: "Authorization")
            let result: (Data, URLResponse)
            if let file { result = try await macUploadSession.upload(for: request, fromFile: file) }
            else { result = try await macUploadSession.data(for: request) }
            if (result.1 as? HTTPURLResponse)?.statusCode != 401 || attempt == 1 { return result }
        }
        throw failure("Reconnect to your vault to continue the upload.")
    }
    private func macJSON(_ connection: VaultConnection, _ path: String, method: String = "GET", body: [String: Any]? = nil) async throws -> [String: Any] {
        let request = try connection.request(path, method: method, json: body)
        let (data, response) = try await macRequest(request)
        try VaultConnection.check(response, data)
        guard let result = try JSONSerialization.jsonObject(with: data) as? [String: Any] else { throw failure("Invalid server response.") }
        return result
    }
    #endif

    private func pollReceipt() async throws {
        #if os(macOS)
        let connection = try await macConnection()
        #else
        let connection = try connection()
        #endif
        guard let id = snapshot.importID else { return }
        for _ in 0..<120 {
            try Task.checkCancellation()
            #if os(macOS)
            let receipt = try await macJSON(connection, "/api/v1/imports/\(id)")
            #else
            let receipt = try await connection.json("/api/v1/imports/\(id)")
            #endif
            try Task.checkCancellation()
            if receipt["state"] as? String == "failed" {
                let files = receipt["files"] as? [[String: Any]] ?? []
                throw failure(files.compactMap { $0["failure"] as? String }.first ?? "The server could not verify this import. Your local copies were kept.")
            }
            if receipt["state"] as? String == "complete" {
                let files = receipt["files"] as? [[String: Any]] ?? []
                let rejected = files.filter { !["verified", "duplicate"].contains($0["state"] as? String ?? "") }
                guard !files.isEmpty, rejected.isEmpty else { throw failure(rejected.first?["failure"] as? String ?? "The server could not verify every recording. Your local copies were kept.") }
                snapshot.state = "verified"; snapshot.files.indices.forEach { snapshot.files[$0].state = "verified" }; save()
                // Prepared files are kept until the user clears the receipt.
                return
            }
            try await Task.sleep(for: .seconds(2))
        }
        throw failure("Server verification is still running. Resume to check the receipt.")
    }
    nonisolated public func urlSession(_ session: URLSession, dataTask: URLSessionDataTask, didReceive data: Data) {
        Task { @MainActor in
            var buffer = self.responseData[dataTask.taskIdentifier] ?? Data()
            if buffer.count < 32_768 { buffer.append(data.prefix(32_768 - buffer.count)) }
            self.responseData[dataTask.taskIdentifier] = buffer
        }
    }
    nonisolated public func urlSession(_ session: URLSession, task: URLSessionTask, didCompleteWithError error: (any Error)?) {
        let taskID = task.taskIdentifier, itemID = task.taskDescription, response = task.response
        Task { @MainActor in
            let data = self.responseData.removeValue(forKey: taskID) ?? Data()
            guard self.snapshot.state == "uploading", let itemID, self.snapshot.files.contains(where: { $0.id == itemID }) else { return }
            do {
                if let error { throw error }
                if let response { try VaultConnection.check(response, data) } else { throw self.failure("No upload response.") }
                try? FileManager.default.removeItem(at: self.folder.appendingPathComponent("chunk-\(itemID)"))
                self.start()
            } catch { self.snapshot.state = "failed"; self.snapshot.error = error.localizedDescription; self.save() }
        }
    }
    nonisolated public func urlSessionDidFinishEvents(forBackgroundURLSession session: URLSession) {
        Task { @MainActor in self.backgroundCompletion?(); self.backgroundCompletion = nil }
    }
    private func failure(_ message: String) -> NSError { NSError(domain: "VaultUpload", code: 1, userInfo: [NSLocalizedDescriptionKey: message]) }
}

/// Audio callbacks run on a preparation worker; emit at most twice per second.
private final class PreparationProgressReporter: @unchecked Sendable {
    private let lock = NSLock()
    private var last = -Double.infinity
    private let update: @Sendable (Double) -> Void
    init(_ update: @escaping @Sendable (Double) -> Void) { self.update = update }
    func report(_ value: Double) {
        let now = ProcessInfo.processInfo.systemUptime
        let send = lock.withLock {
            guard now - last >= 0.5 || value >= 1 else { return false }
            last = now
            return true
        }
        if send { update(min(max(value, 0), 1)) }
    }
}
