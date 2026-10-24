import AppKit
import Combine
import CryptoKit
import SwiftUI
import VaultNative

@MainActor enum RemoteJSON {
    private static let fractional: ISO8601DateFormatter = { let f = ISO8601DateFormatter(); f.formatOptions = [.withInternetDateTime, .withFractionalSeconds]; return f }()
    private static let plain = ISO8601DateFormatter()
    static func date(_ value: Any?) -> Date? { guard let text = value as? String else { return nil }; return fractional.date(from: text) ?? plain.date(from: text) }
    static func integer(_ value: Any?) -> Int { (value as? NSNumber)?.intValue ?? 0 }
    static func component(_ value: String) -> String { value.addingPercentEncoding(withAllowedCharacters: .alphanumerics.union(CharacterSet(charactersIn: "-._~")))! }
}

struct RemoteRecording: Identifiable, Equatable, Sendable {
    let id: String
    let title: String
    let recordedAt: Date
    let hasRecordingDate: Bool
    let durationMs: Int
    let timezone: String
    let stage: String
    let activeStage: String?
    let failure: String?
    let pauseRequested: Bool
    let labels: [String]
    let summary: String?
    let audioAvailable: Bool
    let trashedAt: Date?
    @MainActor init?(_ value: [String: Any]) {
        guard let id = value["id"] as? String else { return nil }
        self.id = id
        title = value["title"] as? String ?? ""
        let date = RemoteJSON.date(value["recorded_at"])
        hasRecordingDate = date != nil
        recordedAt = date ?? RemoteJSON.date(value["created_at"]) ?? .distantPast
        durationMs = RemoteJSON.integer(value["duration_ms"])
        timezone = value["timezone"] as? String ?? TimeZone.current.identifier
        stage = value["stage"] as? String ?? "queued"
        activeStage = value["active_stage"] as? String
        failure = value["failure"] as? String
        pauseRequested = value["pause_requested"] as? Bool ?? false
        labels = value["labels"] as? [String] ?? []
        summary = value["summary"] as? String
        audioAvailable = value["audio_available"] as? Bool ?? false
        trashedAt = RemoteJSON.date(value["trashed_at"])
    }
    var isProcessing: Bool { !["complete", "failed", "paused"].contains(stage) }
    var displayTitle: String {
        title.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty ? "Recording · \(recordedAt.formatted(date: .abbreviated, time: .shortened))" : title
    }
    var status: String { pauseRequested ? "Pausing after the current step…" : (activeStage ?? stage).capitalized }
}

struct RemotePerson: Identifiable, Equatable, Sendable {
    let id: String
    let name: String
    let samples: Int
    @MainActor init?(_ value: [String: Any]) {
        guard let id = value["id"] as? String, let name = value["name"] as? String else { return nil }
        self.id = id; self.name = name; samples = RemoteJSON.integer(value["samples"])
    }
}

@MainActor final class RemoteLibraryModel: ObservableObject {
    @Published private(set) var entries: [RemoteRecording] = []
    @Published private(set) var trashEntries: [RemoteRecording] = []
    @Published private(set) var isLoading = false
    @Published var error: String?
    @Published private(set) var peopleRevision = UUID()
    @Published private(set) var hasMore = false
    @Published private(set) var hasMoreTrash = false
    @Published private(set) var isLoadingMore = false
    private var origin = ""
    private var reloadID = UUID()
    private var trashReloadID = UUID()
    private var moreID = UUID()
    private var nextCursor: Int?
    private var trashNextCursor: Int?
    var trashedEntries: [RemoteRecording] { trashEntries }
    func entry(id: String) -> RemoteRecording? { entries.first { $0.id == id } ?? trashEntries.first { $0.id == id } }
    func peopleDidChange() { peopleRevision = UUID() }

    private func currentServer() -> String {
        let server = NativeVaultSession.shared.server
        if server != origin {
            entries = []; trashEntries = []; origin = server
            nextCursor = nil; trashNextCursor = nil; hasMore = false; hasMoreTrash = false
            reloadID = UUID(); trashReloadID = UUID(); moreID = UUID(); isLoadingMore = false; isLoading = false
            error = nil
        }
        return server
    }
    func reload() async { await reloadPage(trash: false) }
    func reloadTrash() async { await reloadPage(trash: true) }
    private func reloadPage(trash: Bool) async {
        let server = currentServer()
        guard !server.isEmpty else { return }
        let current = UUID()
        if trash { trashReloadID = current } else { reloadID = current; isLoading = true }
        defer { if !trash && reloadID == current { isLoading = false } }
        do {
            let (first, next) = try await fetchPage(trash: trash, cursor: 0)
            guard !Task.isCancelled, server == NativeVaultSession.shared.server,
                  (trash ? trashReloadID : reloadID) == current else { return }
            // Refresh only the first page while retaining the older pages the user opened.
            let previous = trash ? trashEntries : entries
            var tail: [RemoteRecording] = []
            if next != nil, let last = first.last, let boundary = previous.firstIndex(where: { $0.id == last.id }) {
                let freshIDs = Set(first.map(\.id))
                tail = previous.dropFirst(boundary + 1).filter { !freshIDs.contains($0.id) }
            }
            let cursor = next.map { $0 + tail.count }
            if trash {
                trashEntries = first + tail; trashNextCursor = cursor; hasMoreTrash = cursor != nil
            } else {
                entries = first + tail; nextCursor = cursor; hasMore = cursor != nil
            }
            error = nil
        } catch is CancellationError {} catch {
            if server == NativeVaultSession.shared.server, (trash ? trashReloadID : reloadID) == current { self.error = error.localizedDescription }
        }
    }
    func loadMore() async { await loadMorePage(trash: false) }
    func loadMoreTrash() async { await loadMorePage(trash: true) }
    private func loadMorePage(trash: Bool) async {
        let server = currentServer()
        guard !server.isEmpty, !isLoadingMore, let cursor = trash ? trashNextCursor : nextCursor else { return }
        let generation = trash ? trashReloadID : reloadID
        let current = UUID(); moreID = current; isLoadingMore = true
        defer { if moreID == current { isLoadingMore = false } }
        do {
            let (page, next) = try await fetchPage(trash: trash, cursor: cursor)
            guard !Task.isCancelled, server == NativeVaultSession.shared.server,
                  generation == (trash ? trashReloadID : reloadID), moreID == current else { return }
            let existingIDs = Set((trash ? trashEntries : entries).map(\.id))
            let additional = page.filter { !existingIDs.contains($0.id) }
            if trash { trashEntries += additional; trashNextCursor = next; hasMoreTrash = next != nil }
            else { entries += additional; nextCursor = next; hasMore = next != nil }
            error = nil
        } catch is CancellationError {} catch { if server == NativeVaultSession.shared.server { self.error = error.localizedDescription } }
    }
    private func fetchPage(trash: Bool, cursor: Int) async throws -> ([RemoteRecording], Int?) {
        try Task.checkCancellation()
        let value = try await NativeVaultSession.shared.json("/api/v1/entries?limit=100&cursor=\(cursor)" + (trash ? "&trash=true" : ""))
        let rows = (value["entries"] as? [[String: Any]] ?? []).compactMap(RemoteRecording.init)
        let next = value["next_cursor"] as? Int
        return (rows, next.flatMap { $0 > cursor ? $0 : nil })
    }
    func trash(id: String) async { await action(id: id, suffix: "/trash") }
    func restore(id: String) async { await action(id: id, suffix: "/restore") }
    func pause(id: String) async { await action(id: id, suffix: "/pause") }
    func resume(id: String) async { await action(id: id, suffix: "/retry") }
    func permanentlyDelete(id: String) async { await action(id: id, suffix: "", method: "DELETE") }
    private func action(id: String, suffix: String, method: String = "POST") async {
        let server = currentServer()
        do {
            _ = try await NativeVaultSession.shared.json("/api/v1/entries/\(RemoteJSON.component(id))\(suffix)", method: method)
            guard server == NativeVaultSession.shared.server else { return }
            if suffix == "/trash" || method == "DELETE" { entries.removeAll { $0.id == id } }
            if suffix == "/restore" || method == "DELETE" { trashEntries.removeAll { $0.id == id } }
            await reload(); await reloadTrash(); error = nil
        } catch { if server == NativeVaultSession.shared.server { self.error = error.localizedDescription } }
    }
}

struct RemoteLibraryView: View {
    @ObservedObject var model: RemoteLibraryModel
    let openEntry: (String) -> Void
    var showTrash = false
    @State private var search = ""
    @State private var deleting: RemoteRecording?
    @State private var trashing: RemoteRecording?
    private var recordings: [RemoteRecording] {
        (showTrash ? model.trashEntries : model.entries).filter {
            search.isEmpty || $0.displayTitle.localizedCaseInsensitiveContains(search) || $0.labels.contains { $0.localizedCaseInsensitiveContains(search) }
        }
    }
    var body: some View {
        Group {
            if model.isLoading && recordings.isEmpty {
                ProgressView().frame(maxWidth: .infinity, maxHeight: .infinity)
            } else if recordings.isEmpty {
                ContentUnavailableView(search.isEmpty ? (showTrash ? "Trash is empty" : "No transcripts yet") : "No matching recordings",
                                       systemImage: showTrash ? "trash" : "text.bubble",
                                       description: Text(showTrash ? "Recordings moved to Trash can be restored here." : "Imported recordings appear here as the server processes them."))
            } else {
                List {
                    ForEach(recordings) { entry in
                    Button { openEntry(entry.id) } label: {
                        HStack(spacing: 14) {
                            VStack(alignment: .leading, spacing: 4) {
                                Text(entry.displayTitle).fontWeight(.medium).foregroundStyle(.primary)
                                HStack(spacing: 8) {
                                    Text(entry.recordedAt, format: .dateTime.day().month(.abbreviated).hour().minute())
                                    Text(Format.duration(Double(entry.durationMs) / 1000))
                                    if !entry.labels.isEmpty { Text(entry.labels.joined(separator: ", ")).lineLimit(1) }
                                }
                                .font(.callout).foregroundStyle(.secondary)
                            }
                            Spacer()
                            if entry.isProcessing { ProgressView().controlSize(.small) }
                            if entry.stage != "complete" { Text(entry.status).font(.callout).foregroundStyle(entry.stage == "failed" ? .red : .secondary) }
                        }
                        .padding(.vertical, 5)
                        .contentShape(Rectangle())
                    }
                    .buttonStyle(.plain)
                    .contextMenu {
                        Button("Open Transcript") { openEntry(entry.id) }
                        if showTrash {
                            Button("Restore") { Task { await model.restore(id: entry.id) } }
                            Divider()
                            Button("Delete Permanently…", role: .destructive) { deleting = entry }
                        } else {
                            if entry.isProcessing {
                                Button("Pause After Current Step") { Task { await model.pause(id: entry.id) } }.disabled(entry.pauseRequested)
                            } else if ["failed", "paused"].contains(entry.stage) {
                                Button("Resume Processing") { Task { await model.resume(id: entry.id) } }
                            }
                            Divider()
                            Button("Move to Trash…", role: .destructive) { trashing = entry }.disabled(entry.isProcessing)
                        }
                    }
                    }
                    if showTrash ? model.hasMoreTrash : model.hasMore {
                        Button {
                            Task { if showTrash { await model.loadMoreTrash() } else { await model.loadMore() } }
                        } label: {
                            HStack { Spacer(); if model.isLoadingMore { ProgressView().controlSize(.small) }; Text("Load More"); Spacer() }
                        }
                        .disabled(model.isLoadingMore)
                    }
                }
                .listStyle(.inset)
                .scrollContentBackground(.hidden)
            }
        }
        .navigationTitle(showTrash ? "Trash" : "Transcripts")
        .searchable(text: $search, prompt: "Filter recordings")
        .toolbar { Button("Reload", systemImage: "arrow.clockwise") { Task { if showTrash { await model.reloadTrash() } else { await model.reload() } } } }
        .safeAreaInset(edge: .bottom, spacing: 0) {
            if recordings.isEmpty && (showTrash ? model.hasMoreTrash : model.hasMore) {
                Button("Load More Recordings") { Task { if showTrash { await model.loadMoreTrash() } else { await model.loadMore() } } }
                    .disabled(model.isLoadingMore).padding(12)
            }
            if let error = model.error { Label(error, systemImage: "exclamationmark.circle").foregroundStyle(.red).font(.callout).padding(12) }
        }
        .task {
            if showTrash { await model.reloadTrash() } else { await model.reload() }
        }
        .confirmationDialog("Move this recording to Trash?", isPresented: Binding(get: { trashing != nil }, set: { if !$0 { trashing = nil } }), presenting: trashing) { entry in
            Button("Move to Trash", role: .destructive) { Task { await model.trash(id: entry.id) } }
        } message: { _ in Text("It stays on the server and can be restored from Trash. Recorder originals are unchanged.") }
        .confirmationDialog("Permanently delete this recording?", isPresented: Binding(get: { deleting != nil }, set: { if !$0 { deleting = nil } }), presenting: deleting) { entry in
            Button("Delete Permanently", role: .destructive) { Task { await model.permanentlyDelete(id: entry.id) } }
        } message: { _ in Text("Its archived audio, transcript and contributed voice samples will be deleted. This cannot be undone.") }
    }
}

final class RemoteNoRedirects: NSObject, URLSessionTaskDelegate {
    func urlSession(_ session: URLSession, task: URLSessionTask, willPerformHTTPRedirection response: HTTPURLResponse, newRequest request: URLRequest, completionHandler: @escaping (URLRequest?) -> Void) { completionHandler(nil) }
}

@MainActor enum RemoteAssets {
    static func directory(server: String, entry: String) throws -> URL {
        func hash(_ value: String) -> String { SHA256.hash(data: Data(value.utf8)).map { String(format: "%02x", $0) }.joined() }
        let base = FileManager.default.urls(for: .applicationSupportDirectory, in: .userDomainMask)[0]
            .appendingPathComponent("TranscriptVault/Playback/\(hash(server))/\(hash(entry))", isDirectory: true)
        try FileManager.default.createDirectory(at: base, withIntermediateDirectories: true, attributes: [.posixPermissions: 0o700])
        var directory = base
        var flags = URLResourceValues(); flags.isExcludedFromBackup = true
        try? directory.setResourceValues(flags)
        return base
    }
    static func audio(server: String, entry: String) async throws -> URL {
        let destination = try directory(server: server, entry: entry).appendingPathComponent("playback.m4a")
        if FileManager.default.fileExists(atPath: destination.path) { return destination }
        let client = URLSession(configuration: .ephemeral, delegate: RemoteNoRedirects(), delegateQueue: nil)
        defer { client.finishTasksAndInvalidate() }
        for attempt in 0...1 {
            let connection = try await NativeVaultSession.connection(server: server, forceRefresh: attempt > 0)
            let request = try connection.request("/api/v1/entries/\(RemoteJSON.component(entry))/audio")
            let (temporary, response) = try await client.download(for: request)
            if (response as? HTTPURLResponse)?.statusCode == 401 && attempt == 0 { try? FileManager.default.removeItem(at: temporary); continue }
            try VaultConnection.check(response)
            try Task.checkCancellation()
            if FileManager.default.fileExists(atPath: destination.path) { try? FileManager.default.removeItem(at: temporary); return destination }
            try FileManager.default.moveItem(at: temporary, to: destination)
            return destination
        }
        throw URLError(.userAuthenticationRequired)
    }
}
