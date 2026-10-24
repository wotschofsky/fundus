import SwiftUI
import UniformTypeIdentifiers
import VaultNative

@main
struct VaultUploaderApp: App {
    var body: some Scene {
        Window("Vault Uploader", id: "main") {
            UploaderView()
                .preferredColorScheme(.light)
                .frame(minWidth: 760, minHeight: 460)
        }
        .defaultSize(width: 960, height: 640)
        .commands { UploaderCommands() }
        Settings { VaultSettingsView().preferredColorScheme(.light) }
    }
}

private enum UploadDestination: Hashable { case recorder, folder, transfers, library, people, trash, entry(String) }

struct UploaderView: View {
    @StateObject private var queue = TransferQueue.shared
    @StateObject private var sources = RecorderSources()
    @StateObject private var native = NativeVaultSession.shared
    @StateObject private var library = RemoteLibraryModel()
    @State private var sessions = SourceSessions()
    @State private var preview = SessionPreview()
    @State private var profiles = VaultProfiles.shared
    @State private var pendingSessions: [RecordingSession] = []
    @Environment(\.openSettings) private var openSettings
    @State private var destination: UploadDestination? = .recorder
    @State private var pickingFiles = false
    @State private var pickingFolder = false
    @State private var error: String?
    @State private var ejectError: String?

    private var canUpload: Bool { ["idle", "verified"].contains(queue.snapshot.state) && !sources.isEjecting }
    private var canEject: Bool { !sources.devices.isEmpty && !sources.isEjecting && !queue.isReadingSource }

    var body: some View {
        NavigationSplitView {
            List(selection: $destination) {
                Section("Sources") {
                    Label("DJI Mic", systemImage: "mic")
                        .badge(sources.devices.count)
                        .tag(UploadDestination.recorder)
                    if let folder = sources.folder {
                        Label(folder.lastPathComponent, systemImage: "folder")
                            .tag(UploadDestination.folder)
                    }
                }
                Section("Library") {
                    Label("All Recordings", systemImage: "waveform").tag(UploadDestination.library)
                    Label("People", systemImage: "person.2").tag(UploadDestination.people)
                    Label("Trash", systemImage: "trash").tag(UploadDestination.trash)
                }
                if !library.entries.isEmpty {
                    Section("Transcripts") {
                        ForEach(library.entries.prefix(100)) { entry in
                            VStack(alignment: .leading, spacing: 2) {
                                Text(entry.title.isEmpty ? Format.day(entry.recordedAt) : entry.title).lineLimit(1)
                                Text(entry.stage == "complete" ? Format.duration(Double(entry.durationMs) / 1000) : (entry.activeStage ?? entry.stage).capitalized)
                                    .font(.caption).foregroundStyle(.secondary)
                            }.tag(UploadDestination.entry(entry.id))
                        }
                    }
                }
                Section("Transfers") {
                    Label("Uploads", systemImage: "square.and.arrow.up")
                        .badge(queue.snapshot.files.count)
                        .tag(UploadDestination.transfers)
                }
            }
            .navigationSplitViewColumnWidth(min: 190, ideal: 230, max: 320)
            .safeAreaInset(edge: .top, spacing: 0) {
                Menu {
                    ForEach(profiles.profiles) { profile in
                        Button(profile.name) { switchVault(profile.server) }.disabled(!canUpload)
                    }
                    Divider()
                    SettingsLink { Text("Connect Another Vault…") }
                    Button("Reload Library") { Task { await library.reload(); await synchronizeHistory() } }
                } label: {
                    Label(URL(string: queue.snapshot.server)?.host ?? "Choose Vault…", systemImage: "books.vertical")
                        .fontWeight(.medium).frame(maxWidth: .infinity, alignment: .leading)
                }
                .menuStyle(.button).buttonStyle(.bordered).controlSize(.large)
                .padding(.horizontal, 10).padding(.bottom, 6)
            }
            .safeAreaInset(edge: .bottom, spacing: 0) {
                HStack {
                    if let url = URL(string: queue.snapshot.server), !queue.snapshot.server.isEmpty {
                        Link(destination: url) { Label("Open Vault", systemImage: "arrow.up.right.square") }
                            .lineLimit(1)
                    } else {
                        Button("Connect Vault…") { openSettings() }.buttonStyle(.link)
                    }
                    Spacer()
                    SettingsLink { Image(systemName: "gearshape") }
                        .buttonStyle(.borderless)
                        .help("Connection Settings (⌘,)")
                        .accessibilityLabel("Connection Settings")
                }
                .font(.callout)
                .padding(12)
            }
        } detail: {
            Group {
                switch destination {
                case .transfers:
                    UploadQueueView(queue: queue, addFiles: chooseFiles)
                case .library:
                    RemoteLibraryView(model: library, openEntry: openEntry)
                case .people:
                    RemotePeopleSettings(library: library)
                case .trash:
                    RemoteLibraryView(model: library, openEntry: openEntry, showTrash: true)
                case .entry(let id):
                    RemoteTranscriptView(entryID: id, library: library).id(id)
                case .folder:
                    RecordingSourceView(files: sources.folderFiles, isScanning: sources.isReadingFolder,
                                        title: sources.folder?.lastPathComponent ?? "Source Folder",
                                        subtitle: "Audio files", connected: sources.folder != nil,
                                        source: sessions, preview: preview, server: queue.snapshot.server, canUpload: canUpload,
                                        isConnected: !queue.snapshot.server.isEmpty,
                                        chooseFolder: chooseFolder, upload: upload, connect: { openSettings() },
                                        openEntry: openEntry, retryEntry: retryEntry, pauseEntry: pauseEntry)
                default:
                    RecordingSourceView(files: sources.files, isScanning: sources.isScanning,
                                        title: "DJI Mic", subtitle: recorderSubtitle,
                                        connected: !sources.devices.isEmpty,
                                        source: sessions, preview: preview, server: queue.snapshot.server, canUpload: canUpload,
                                        isConnected: !queue.snapshot.server.isEmpty,
                                        chooseFolder: chooseFolder, upload: upload, connect: { openSettings() },
                                        openEntry: openEntry, retryEntry: retryEntry, pauseEntry: pauseEntry)
                }
            }
            .frame(maxWidth: .infinity, maxHeight: .infinity)
            .background(.white)
            .toolbar {
                if !sources.devices.isEmpty {
                    ToolbarItem(placement: .principal) {
                        Button(sources.isEjecting ? "Ejecting…" : "Eject All Mics", systemImage: "eject", action: ejectRecorders)
                            .labelStyle(.titleAndIcon)
                            .disabled(!canEject)
                            .help(queue.isReadingSource ? "Wait for recordings to finish copying" : "Safely eject all connected DJI microphones")
                            .accessibilityIdentifier("eject-all-mics")
                    }
                    if #available(macOS 26.0, *) { ToolbarSpacer(.fixed) }
                }
                ToolbarItemGroup {
                    Button("Rescan", systemImage: "arrow.clockwise", action: rescan)
                        .disabled(sources.isEjecting)
                        .help("Rescan connected microphones (⌘R)")
                    Button("Choose Source Folder…", systemImage: "folder", action: chooseFolder)
                        .disabled(sources.isEjecting)
                        .help("Choose Source Folder… (⇧⌘O)")
                    Button("Add Audio Files…", systemImage: "plus", action: chooseFiles)
                        .disabled(!canUpload)
                        .help("Add Audio Files… (⌘O)")
                }
            }
        }
        .focusedSceneValue(\.uploaderActions, UploaderActions(addFiles: chooseFiles, chooseFolder: chooseFolder,
                           rescan: rescan, eject: ejectRecorders, canUpload: canUpload, canEject: canEject))
        .task {
            sources.start(timezone: queue.snapshot.timezone)
            if !queue.snapshot.server.isEmpty {
                profiles.remember(queue.snapshot.server)
                try? await native.use(server: queue.snapshot.server)
                await library.reload()
                await synchronizeHistory()
            }
        }
        .task(id: queue.snapshot.server) {
            while !Task.isCancelled {
                await refreshReceipts()
                if native.isConnected { await library.reload() }
                let active = !["idle", "verified", "failed", "paused"].contains(queue.snapshot.state)
                    || library.entries.contains { !["complete", "failed", "paused"].contains($0.stage) }
                try? await Task.sleep(for: .seconds(active ? 4 : 15))
            }
        }
        .onChange(of: native.server) { _, server in
            guard !server.isEmpty, native.isConnected else { return }
            Task {
                do { try await queue.configureNative(server: server, timezone: queue.snapshot.timezone); profiles.remember(server); await library.reload(); await synchronizeHistory() }
                catch { self.error = error.localizedDescription }
            }
        }
        .onChange(of: native.isConnected) { _, connected in
            if connected { Task { await library.reload(); await synchronizeHistory() } }
        }
        .onChange(of: library.entries.map { "\($0.id)|\($0.stage)|\($0.activeStage ?? "")" }) { _, _ in
            Task { await synchronizeHistory() }
        }
        .onDisappear { sources.stop() }
        .onChange(of: sources.devices.map(\.id)) { old, new in
            if !Set(new).subtracting(old).isEmpty { destination = .recorder }
        }
        .onChange(of: sources.folder) { old, folder in
            if folder == nil, destination == .folder { destination = .recorder }
            else if old == nil, folder != nil, sources.devices.isEmpty { destination = .folder }
        }
        .onChange(of: queue.snapshot.timezone) { _, timezone in sources.setTimezone(timezone) }
        .onChange(of: queue.snapshot.importID) { _, id in
            if let id, !pendingSessions.isEmpty {
                sessions.register(pendingSessions, server: queue.snapshot.server, importID: id)
                pendingSessions = []
            }
        }
        .onChange(of: queue.snapshot.state) { _, state in
            if state == "verified" { Task { await refreshReceipts(); await library.reload(); await synchronizeHistory() } }
        }
        .fileImporter(isPresented: $pickingFiles, allowedContentTypes: [.wav, .mp3, .mpeg4Audio, UTType(filenameExtension: "flac") ?? .audio], allowsMultipleSelection: true) { result in
            switch result {
            case .success(let urls): enqueue(urls)
            case .failure(let failure): error = failure.localizedDescription
            }
        }
        .fileImporter(isPresented: $pickingFolder, allowedContentTypes: [.folder]) { result in
            switch result {
            case .success(let url):
                sources.chooseFolder(url)
                destination = .folder
            case .failure(let failure): error = failure.localizedDescription
            }
        }
        .alert("Could not add recordings", isPresented: Binding(get: { error != nil }, set: { if !$0 { error = nil } })) {
            Button("OK", role: .cancel) {}
        } message: { Text(error ?? "") }
        .alert("Could not eject all microphones", isPresented: Binding(get: { ejectError != nil }, set: { if !$0 { ejectError = nil } })) {
            Button("OK", role: .cancel) {}
        } message: { Text((ejectError ?? "") + "\n\nClose other apps using the microphones and try again.") }
    }

    private var recorderSubtitle: String {
        if sources.isScanning && sources.devices.isEmpty { return "Looking for recordings…" }
        if sources.devices.isEmpty { return "No microphone connected" }
        return "\(sources.devices.count) \(sources.devices.count == 1 ? "microphone" : "microphones") · \(sources.files.count) recordings"
    }

    private func chooseFiles() {
        guard canUpload else { return }
        if queue.snapshot.server.isEmpty { openSettings() } else { pickingFiles = true }
    }
    private func chooseFolder() { guard !sources.isEjecting else { return }; pickingFolder = true }
    private func rescan() {
        sources.refresh()
        if let folder = sources.folder { sources.chooseFolder(folder) }
    }
    private func upload(_ selected: [RecordingSession]) {
        let plan = sessions.uploadPlan(selected)
        pendingSessions = selected
        enqueue(plan.urls, plans: plan.plans, sourceIDs: plan.sourceIDs)
    }
    private func enqueue(_ urls: [URL], plans: [ImportSessionPlan] = [], sourceIDs: [String: String] = [:]) {
        guard canUpload else { return }
        preview.stop()
        destination = .transfers
        Task {
            guard !sources.isEjecting else { return }
            do { try await queue.enqueue(urls, sessions: plans, sourceIDs: sourceIDs) }
            catch { self.error = error.localizedDescription }
        }
    }
    private func openEntry(_ id: String) { preview.stop(); destination = .entry(id) }
    private func retryEntry(_ id: String) {
        Task {
            do {
                _ = try await native.json("/api/v1/entries/\(id)/retry", method: "POST")
                sessions.setStage(entry: id, stage: "queued")
                await library.reload()
            } catch { self.error = error.localizedDescription }
        }
    }
    private func pauseEntry(_ id: String) {
        Task {
            do { _ = try await native.json("/api/v1/entries/\(id)/pause", method: "POST"); sessions.setPausing(entry: id); await library.reload() }
            catch { self.error = error.localizedDescription }
        }
    }
    private func switchVault(_ server: String) {
        guard canUpload else { return }
        preview.stop()
        Task {
            do {
                try await native.use(server: server)
                try await queue.configureNative(server: server, timezone: queue.snapshot.timezone)
                await library.reload()
                await synchronizeHistory()
                destination = .library
            } catch { self.error = error.localizedDescription; openSettings() }
        }
    }
    private func refreshReceipts() async {
        let server = queue.snapshot.server
        guard !server.isEmpty else { return }
        var ids = Set(sessions.records.filter { $0.server == server && !["complete", "failed", "paused"].contains($0.stage) }.map(\.importID).filter { !$0.isEmpty })
        if let current = queue.snapshot.importID { ids.insert(current) }
        for id in ids {
            do {
                let connection = try await NativeVaultSession.connection(server: server)
                let receipt = try await connection.json("/api/v1/imports/\(id)")
                sessions.updateReceipt(server: server, importID: id, entries: receipt["entries"] as? [[String: Any]] ?? [])
            } catch { /* Durable queue errors remain visible in Uploads; a transient poll is retried. */ }
        }
    }
    private func synchronizeHistory() async {
        guard native.isConnected else { return }
        do {
            let server = native.server
            var entryIDs: Set<String> = []
            var cursor: Int? = 0
            while let page = cursor {
                let result = try await native.json("/api/v1/source-history?limit=500&cursor=\(page)")
                guard server == native.server else { return }
                let entries = result["entries"] as? [[String: Any]] ?? []
                entryIDs.formUnion(entries.compactMap { $0["id"] as? String })
                sessions.syncHistory(entries, server: server)
                cursor = result["next_cursor"] as? Int
            }
            guard server == native.server else { return }
            sessions.reconcileHistory(server: server, entryIDs: entryIDs)
        } catch { /* Older servers have no source history; the import receipt is still retained. */ }
    }
    private func ejectRecorders() {
        guard canEject else { return }
        preview.stop()
        Task {
            let failures = await sources.ejectAll(isCopying: queue.isReadingSource)
            if !failures.isEmpty { ejectError = failures.joined(separator: "\n\n") }
        }
    }
}

private struct UploaderActions {
    let addFiles: () -> Void
    let chooseFolder: () -> Void
    let rescan: () -> Void
    let eject: () -> Void
    let canUpload: Bool
    let canEject: Bool
}
private struct UploaderActionsKey: FocusedValueKey { typealias Value = UploaderActions }
private extension FocusedValues {
    var uploaderActions: UploaderActions? {
        get { self[UploaderActionsKey.self] }
        set { self[UploaderActionsKey.self] = newValue }
    }
}
private struct UploaderCommands: Commands {
    @FocusedValue(\.uploaderActions) private var actions
    var body: some Commands {
        CommandGroup(replacing: .newItem) {
            Button("Add Audio Files…") { actions?.addFiles() }.keyboardShortcut("o").disabled(actions?.canUpload != true)
            Button("Choose Source Folder…") { actions?.chooseFolder() }.keyboardShortcut("o", modifiers: [.command, .shift])
        }
        CommandMenu("Recorder") {
            Button("Rescan") { actions?.rescan() }.keyboardShortcut("r")
            Button("Eject All Mics") { actions?.eject() }.keyboardShortcut("e").disabled(actions?.canEject != true)
        }
    }
}
