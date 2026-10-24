import AppKit
import Combine
import SwiftUI
import UniformTypeIdentifiers
import VaultNative

struct RemoteSegment: Identifiable, Codable, Equatable, Sendable {
    let id: String
    let speaker: String
    let startMs: Int
    let endMs: Int
    let text: String
    @MainActor init?(_ value: [String: Any]) {
        guard let id = value["id"] as? String, let text = value["text"] as? String else { return nil }
        self.id = id; self.text = text
        speaker = value["speaker"] as? String ?? ""
        startMs = RemoteJSON.integer(value["start_ms"]); endMs = RemoteJSON.integer(value["end_ms"])
    }
}
struct RemoteSpeaker: Identifiable, Sendable {
    let label: String
    let name: String?
    let personID: String?
    let state: String
    let explanations: [String]
    var id: String { label }
    init(_ value: [String: Any]) {
        label = value["label"] as? String ?? ""
        name = value["name"] as? String; personID = value["person_id"] as? String
        state = value["state"] as? String ?? "unknown"
        if state == "manual" { explanations = ["Assigned by you"] }
        else {
            explanations = (value["evidence"] as? [[String: Any]] ?? []).compactMap {
                if $0["source"] as? String == "voice" { return "Matched a saved voice profile" }
                if let quote = $0["quote"] as? String, !quote.isEmpty { return "Mentioned in the recording: “\(quote)”" }
                if let name = $0["name"] as? String { return "Possible name: \(name)" }
                return nil
            }
        }
    }
}
private struct RemoteDetail: Sendable {
    let entry: RemoteRecording
    let speakers: [RemoteSpeaker]
    let warnings: [String]
    let timeline: SessionTimeline
    let metadata: Data
    @MainActor init(_ value: [String: Any]) throws {
        guard let entry = RemoteRecording(value) else { throw CocoaError(.coderReadCorrupt) }
        self.entry = entry
        speakers = (value["speakers"] as? [[String: Any]] ?? []).map(RemoteSpeaker.init)
        warnings = value["warnings"] as? [String] ?? []
        timeline = Self.timeline(value, entry: entry)
        metadata = try JSONSerialization.data(withJSONObject: value, options: [.prettyPrinted, .sortedKeys])
    }
    @MainActor static func timeline(_ value: [String: Any], entry: RemoteRecording) -> SessionTimeline {
        let sources = value["sources"] as? [[String: Any]] ?? []
        let format = sources.first?["format"] as? [String: Any] ?? [:]
        let rate = max(1, RemoteJSON.integer(format["sample_rate"]))
        let trimmedSeconds = Double(RemoteJSON.integer(value["trim_start_frame"])) / Double(rate)
        var chunks = sources.compactMap { source -> SessionTimeline.Chunk? in
            guard let start = RemoteJSON.date(source["recorded_at"]) else { return nil }
            return SessionTimeline.Chunk(offsetSeconds: Double(RemoteJSON.integer(source["offset_ms"])) / 1000 - trimmedSeconds, start: start)
        }
        if chunks.isEmpty { chunks = [.init(offsetSeconds: 0, start: entry.recordedAt)] }
        return SessionTimeline(chunks: chunks, timeZone: TimeZone(identifier: entry.timezone) ?? .current)
    }
}

@MainActor final class RemoteTranscriptModel: ObservableObject {
    @Published private(set) var entry: RemoteRecording?
    @Published private(set) var rows: [RemoteSegment] = []
    @Published private(set) var speakers: [RemoteSpeaker] = []
    @Published private(set) var people: [RemotePerson] = []
    @Published private(set) var warnings: [String] = []
    @Published private(set) var isLoading = false
    @Published private(set) var isUpdating = false
    @Published private(set) var isLoadingAudio = false
    @Published var error: String?
    @Published var audioError: String?
    let player = PlayerModel()
    private var timeline = SessionTimeline(chunks: [], timeZone: .current)
    private var metadata = Data()
    private var currentEntryID = ""
    private var currentServer = ""
    private var generation = UUID()

    func open(_ id: String) async {
        let server = NativeVaultSession.shared.server
        if currentEntryID != id || currentServer != server {
            player.unload(); rows = []; speakers = []; entry = nil; error = nil; audioError = nil
        }
        currentEntryID = id; currentServer = server
        let generation = UUID(); self.generation = generation
        isLoading = true
        defer { if self.generation == generation { isLoading = false } }
        do {
            async let detail = Self.fetchDetail(id)
            async let transcript = Self.fetchTranscript(id)
            async let people = Self.fetchPeople()
            let loaded = try await (detail, transcript, people)
            guard !Task.isCancelled, self.generation == generation, server == NativeVaultSession.shared.server else { return }
            entry = loaded.0.entry; speakers = loaded.0.speakers; warnings = loaded.0.warnings
            rows = loaded.1; self.people = loaded.2; timeline = loaded.0.timeline; metadata = loaded.0.metadata
            // If completion raced the paged read, read once after the completion marker.
            if rows.isEmpty, entry?.stage == "complete" {
                let completedRows = try await Self.fetchTranscript(id)
                guard !Task.isCancelled, self.generation == generation, server == NativeVaultSession.shared.server else { return }
                rows = completedRows
            }
            error = nil
            if loaded.0.entry.audioAvailable && !player.isAvailable { await loadAudio() }
        } catch is CancellationError {} catch { if self.generation == generation { self.error = error.localizedDescription } }
    }
    private static func fetchDetail(_ id: String) async throws -> RemoteDetail {
        try await RemoteDetail(NativeVaultSession.shared.json("/api/v1/entries/\(RemoteJSON.component(id))"))
    }
    private static func fetchPeople() async throws -> [RemotePerson] {
        let value = try await NativeVaultSession.shared.json("/api/v1/people")
        return (value["people"] as? [[String: Any]] ?? []).compactMap(RemotePerson.init)
    }
    private static func fetchTranscript(_ id: String) async throws -> [RemoteSegment] {
        var cursor = 0, rows: [RemoteSegment] = []
        while true {
            try Task.checkCancellation()
            let value = try await NativeVaultSession.shared.json("/api/v1/entries/\(RemoteJSON.component(id))/transcript?limit=300&cursor=\(cursor)")
            rows += (value["segments"] as? [[String: Any]] ?? []).compactMap(RemoteSegment.init)
            guard let next = value["next_cursor"] as? Int, next > cursor else { return rows }
            cursor = next
        }
    }
    func loadAudio() async {
        guard let entry, entry.audioAvailable else { return }
        let id = entry.id, server = currentServer
        isLoadingAudio = true
        defer { isLoadingAudio = false }
        do {
            let url = try await RemoteAssets.audio(server: server, entry: id)
            guard !Task.isCancelled, id == currentEntryID, server == currentServer, server == NativeVaultSession.shared.server else { return }
            player.load(url, duration: Double(entry.durationMs) / 1000); audioError = nil
        } catch is CancellationError {} catch { audioError = error.localizedDescription }
    }
    func clock(atSeconds seconds: Double) -> String {
        entry?.hasRecordingDate == true ? timeline.clockTime(atMs: Int(seconds * 1000)) : Format.position(seconds)
    }
    func speaker(_ label: String) -> RemoteSpeaker? { speakers.first { $0.label == label } }
    func name(_ label: String) -> String { speaker(label)?.name ?? "Speaker \((speakers.firstIndex { $0.label == label } ?? 0) + 1)" }
    var currentRowID: String? {
        guard player.isAvailable, player.currentSeconds > 0 || player.isPlaying else { return nil }
        let milliseconds = Int(player.currentSeconds * 1000)
        var low = 0, high = rows.count
        while low < high {
            let mid = (low + high) / 2
            if rows[mid].startMs <= milliseconds { low = mid + 1 } else { high = mid }
        }
        return low > 0 ? rows[low - 1].id : nil
    }
    func play(_ row: RemoteSegment) { player.seek(to: Double(row.startMs) / 1000, play: true) }
    func assign(_ person: RemotePerson?, speaker label: String, keepUnidentified: Bool = false) async {
        guard !isUpdating else { return }
        isUpdating = true; defer { isUpdating = false }
        do {
            var body: [String: Any] = ["person_id": person?.id as Any? ?? NSNull()]
            if person == nil { body["locked"] = keepUnidentified }
            _ = try await NativeVaultSession.shared.json("/api/v1/entries/\(RemoteJSON.component(currentEntryID))/speakers/\(RemoteJSON.component(label))", method: "PATCH", body: body)
            await open(currentEntryID)
        } catch { self.error = error.localizedDescription }
    }
    func createPerson(_ name: String, speaker: String) async {
        guard !isUpdating else { return }
        do {
            let value = try await NativeVaultSession.shared.json("/api/v1/people", method: "POST", body: ["name": name])
            if let person = RemotePerson(value) { await assign(person, speaker: speaker) }
        } catch { self.error = error.localizedDescription }
    }
    func saveMetadata(title: String, labels: String) async throws {
        let labels = labels.split(separator: ",").map { $0.trimmingCharacters(in: .whitespacesAndNewlines) }.filter { !$0.isEmpty }
        _ = try await NativeVaultSession.shared.json("/api/v1/entries/\(RemoteJSON.component(currentEntryID))", method: "PATCH", body: ["title": title, "labels": labels])
        await open(currentEntryID)
    }
    func processing(_ action: String) async {
        guard !isUpdating else { return }
        isUpdating = true; defer { isUpdating = false }
        do {
            _ = try await NativeVaultSession.shared.json("/api/v1/entries/\(RemoteJSON.component(currentEntryID))/\(action)", method: "POST")
            await open(currentEntryID)
        } catch { self.error = error.localizedDescription }
    }
    func markdown() -> String {
        guard let entry else { return "" }
        func escape(_ value: String) -> String { value.replacingOccurrences(of: "\\", with: "\\\\").replacingOccurrences(of: "*", with: "\\*").replacingOccurrences(of: "[", with: "\\[").replacingOccurrences(of: "]", with: "\\]") }
        var lines = ["# \(entry.displayTitle)", "", "- Recorded: \(entry.recordedAt.formatted(.iso8601))", "- Duration: \(Format.duration(Double(entry.durationMs) / 1000))"]
        if !entry.labels.isEmpty { lines.append("- Labels: \(entry.labels.joined(separator: ", "))") }
        if let summary = entry.summary, !summary.isEmpty { lines += ["", "## Generated Summary", "", summary] }
        lines += ["", "## Transcript", ""]
        for row in rows { lines += ["[\(clock(atSeconds: Double(row.startMs) / 1000))] **\(escape(name(row.speaker)))**", row.text, ""] }
        return lines.joined(separator: "\n")
    }
    func exportedFiles() throws -> URL {
        let directory = try RemoteAssets.directory(server: currentServer, entry: currentEntryID)
        let markdownURL = directory.appendingPathComponent("transcript.md")
        try Data(markdown().utf8).write(to: markdownURL, options: .atomic)
        try metadata.write(to: directory.appendingPathComponent("metadata.json"), options: .atomic)
        let encoder = JSONEncoder(); encoder.outputFormatting = [.prettyPrinted, .sortedKeys]; encoder.keyEncodingStrategy = .convertToSnakeCase
        try encoder.encode(rows).write(to: directory.appendingPathComponent("transcript.json"), options: .atomic)
        return markdownURL
    }
}

struct RemoteTranscriptView: View {
    let entryID: String
    @ObservedObject var library: RemoteLibraryModel
    @ObservedObject private var auth = NativeVaultSession.shared
    @StateObject private var model = RemoteTranscriptModel()
    @State private var editing = false
    @State private var title = ""
    @State private var labels = ""
    @State private var editError: String?
    @State private var newPersonSpeaker: String?
    @State private var newPersonName = ""

    var body: some View {
        VStack(spacing: 0) {
            notices
            if let entry = model.entry, entry.stage != "complete" { processingBar(entry) }
            if !model.speakers.isEmpty { speakerBar }
            if model.isLoading && model.rows.isEmpty {
                ProgressView().frame(maxWidth: .infinity, maxHeight: .infinity)
            } else if model.rows.isEmpty {
                ContentUnavailableView(model.entry?.stage == "complete" ? "No speech detected" : "Transcript not ready", systemImage: "text.bubble",
                                       description: Text(model.entry?.failure ?? "The transcript appears here as remote processing finishes."))
            } else {
                transcriptList
            }
            if model.isLoadingAudio {
                HStack { ProgressView().controlSize(.small); Text("Downloading audio for playback…").foregroundStyle(.secondary); Spacer() }.font(.callout).padding(12)
            }
            PlayerBar(player: model.player, clock: model.clock(atSeconds:))
        }
        .navigationTitle(model.entry?.displayTitle ?? "Transcript")
        .navigationSubtitle(model.entry.map { "\(Format.duration(Double($0.durationMs) / 1000)) · \(model.speakers.map { model.name($0.label) }.joined(separator: ", "))" } ?? "")
        .toolbar {
            ToolbarItemGroup {
                Button("Edit Recording", systemImage: "pencil") {
                    title = model.entry?.displayTitle ?? ""; labels = model.entry?.labels.joined(separator: ", ") ?? ""; editError = nil; editing = true
                }.disabled(model.entry == nil)
                Button("Open Markdown", systemImage: "doc.text") { export(open: true) }.disabled(model.rows.isEmpty)
                Button("Show in Finder", systemImage: "folder") { export(open: false) }.disabled(model.rows.isEmpty)
                Menu {
                    Button("Export Markdown…") { saveMarkdown() }.disabled(model.rows.isEmpty)
                    Button("Retry Speaker Analysis") { Task { await model.processing("reanalyze"); await library.reload() } }
                        .disabled(model.entry?.isProcessing != false || model.rows.isEmpty)
                    Button("Reload Transcript") { Task { await model.open(entryID) } }
                } label: { Label("More", systemImage: "ellipsis.circle") }
            }
        }
        .task(id: entryID + "|" + auth.server) {
            await model.open(entryID)
            while !Task.isCancelled && model.entry?.isProcessing == true {
                do { try await Task.sleep(for: .seconds(2)) } catch { break }
                await model.open(entryID); await library.reload()
            }
        }
        .onChange(of: library.peopleRevision) { _, _ in Task { await model.open(entryID) } }
        .onDisappear { model.player.unload() }
        .sheet(isPresented: $editing) {
            VStack(spacing: 18) {
                Form {
                    TextField("Title", text: $title)
                    TextField("Labels", text: $labels, prompt: Text("Comma-separated labels"))
                }
                if let editError { Text(editError).foregroundStyle(.red).font(.callout) }
                HStack { Button("Cancel") { editing = false }.keyboardShortcut(.cancelAction); Spacer(); Button("Save") {
                    Task {
                        do { try await model.saveMetadata(title: title, labels: labels); await library.reload(); editing = false }
                        catch { editError = error.localizedDescription }
                    }
                }.keyboardShortcut(.defaultAction).disabled(title.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty) }
            }.padding(24).frame(width: 440)
        }
        .alert("New Person", isPresented: Binding(get: { newPersonSpeaker != nil }, set: { if !$0 { newPersonSpeaker = nil } })) {
            TextField("Name", text: $newPersonName)
            Button("Cancel", role: .cancel) {}
            Button("Add") { if let speaker = newPersonSpeaker { Task { await model.createPerson(newPersonName, speaker: speaker); library.peopleDidChange() } } }
        } message: { Text("Confirming a speaker builds their voice profile for future recordings.") }
    }
    @ViewBuilder private var notices: some View {
        let messages = model.warnings + [model.error, model.audioError].compactMap { $0 }
        if !messages.isEmpty {
            HStack(alignment: .top, spacing: 8) {
                Image(systemName: "exclamationmark.triangle").foregroundStyle(.orange)
                VStack(alignment: .leading, spacing: 3) { ForEach(Array(messages.enumerated()), id: \.offset) { _, message in Text(message).textSelection(.enabled) } }
                Spacer()
                if model.audioError != nil { Button("Retry Audio") { Task { await model.loadAudio() } } }
            }.font(.callout).padding(12)
            Divider()
        }
    }
    private func processingBar(_ entry: RemoteRecording) -> some View {
        HStack(spacing: 12) {
            if entry.isProcessing { ProgressView().controlSize(.small) }
            Text(entry.status).foregroundStyle(.secondary)
            Spacer()
            if entry.isProcessing {
                Button("Pause After Current Step") { Task { await model.processing("pause"); await library.reload() } }.disabled(entry.pauseRequested || model.isUpdating)
            } else if ["paused", "failed"].contains(entry.stage) {
                Button("Resume Processing") { Task { await model.processing("retry"); await library.reload() } }.disabled(model.isUpdating)
            }
        }.font(.callout).padding(12).background(.bar)
    }
    private var speakerBar: some View {
        HStack(spacing: 14) {
            Text("Speakers").foregroundStyle(.secondary)
            ScrollView(.horizontal, showsIndicators: false) {
                HStack(spacing: 14) { ForEach(model.speakers) { speaker in speakerMenu(speaker.label) } }
            }
            if model.isUpdating { ProgressView().controlSize(.small) }
        }.font(.callout).padding(.horizontal, 16).padding(.vertical, 9)
    }
    private var transcriptList: some View {
        let current = model.currentRowID
        return List(model.rows) { row in
            HStack(alignment: .firstTextBaseline, spacing: 12) {
                Button(model.clock(atSeconds: Double(row.startMs) / 1000)) { model.play(row) }
                    .buttonStyle(.plain).font(.callout).monospacedDigit().foregroundStyle(current == row.id ? Color.accentColor : Color.secondary)
                    .frame(width: 68, alignment: .leading).disabled(!model.player.isAvailable)
                    .help("Play from this passage")
                VStack(alignment: .leading, spacing: 4) {
                    speakerMenu(row.speaker)
                    Text(row.text).textSelection(.enabled).frame(maxWidth: .infinity, alignment: .leading)
                }
            }
            .padding(.vertical, 6).padding(.horizontal, 6)
            .background(current == row.id ? Color.accentColor.opacity(0.08) : .clear, in: RoundedRectangle(cornerRadius: 6))
            .contentShape(Rectangle()).onTapGesture { model.play(row) }
            .listRowSeparator(.hidden)
            .accessibilityAction(named: "Play from here") { model.play(row) }
        }.listStyle(.plain).scrollContentBackground(.hidden)
    }
    private func speakerMenu(_ label: String) -> some View {
        let speaker = model.speaker(label)
        return Menu {
            if let explanations = speaker?.explanations, !explanations.isEmpty {
                Section("Identified from") { ForEach(explanations, id: \.self) { Text($0) } }
            }
            if speaker?.state == "automatic", let personID = speaker?.personID, let person = model.people.first(where: { $0.id == personID }) {
                Button("Confirm Identity") { Task { await model.assign(person, speaker: label) } }
            }
            Section("Assign") {
                ForEach(model.people) { person in
                    Toggle(person.name, isOn: Binding(get: { speaker?.personID == person.id }, set: { if $0 { Task { await model.assign(person, speaker: label) } } }))
                }
                Button("New Person…") { newPersonName = ""; newPersonSpeaker = label }
            }
            Section {
                if speaker?.personID != nil { Button("Remove Identity") { Task { await model.assign(nil, speaker: label) } } }
                Toggle("Keep Unidentified", isOn: Binding(get: { speaker?.state == "manual_unknown" }, set: { keep in Task { await model.assign(nil, speaker: label, keepUnidentified: keep) } }))
            }
        } label: {
            HStack(spacing: 4) { Text(model.name(label)).fontWeight(.semibold); Image(systemName: "chevron.down").font(.caption2).foregroundStyle(.secondary) }
        }
        .menuStyle(.button).buttonStyle(.plain).menuIndicator(.hidden).fixedSize()
        .foregroundStyle(speaker?.personID == nil ? .secondary : .primary)
        .disabled(model.isUpdating)
        .help("Change or confirm this speaker’s identity")
    }
    private func export(open: Bool) {
        do {
            let url = try model.exportedFiles()
            if open { NSWorkspace.shared.open(url) } else { NSWorkspace.shared.activateFileViewerSelecting([url]) }
        } catch { model.error = error.localizedDescription }
    }
    private func saveMarkdown() {
        let panel = NSSavePanel()
        panel.nameFieldStringValue = "transcript.md"
        panel.allowedContentTypes = [.plainText]
        panel.begin { response in
            guard response == .OK, let url = panel.url else { return }
            do { try Data(model.markdown().utf8).write(to: url, options: .atomic) }
            catch { model.error = error.localizedDescription }
        }
    }
}
