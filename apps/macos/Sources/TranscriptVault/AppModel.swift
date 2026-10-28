import AppKit
import Combine
import Observation
import UniformTypeIdentifiers
import VaultNative

enum SidebarItem: Hashable {
    case importer, uploads, trash
    case entry(String)
}

/// Ties the recorder, the transfer queue and the remote library together for the whole app.
@MainActor
@Observable
final class AppModel {
    let library = Library()
    let sources = RecorderSources()
    let sessions = SourceSessions()
    let preview = SessionPreview()
    let profiles = VaultProfiles.shared

    /// Mirrors of the shared native objects, so views observe one model. `transfer` changes with
    /// every progress tick; the narrower mirrors below change only when their value does, so the
    /// recorder list does not redraw while a file uploads.
    private(set) var transfer: QueueState
    private(set) var transferState: String
    /// The vault uploads go to; can differ from `server` (the library shown) for token-only setups.
    private(set) var uploadServer: String
    private(set) var hasUploads: Bool
    private(set) var isCopyingSources = false
    private(set) var server = ""
    private(set) var isConnected = false
    private(set) var isSigningIn = false
    var problem: Problem?
    var selection: SidebarItem? = .importer

    struct Problem: Identifiable {
        let id = UUID()
        var title: String
        var message: String
    }

    @ObservationIgnored let queue = TransferQueue.shared
    @ObservationIgnored let native = NativeVaultSession.shared
    /// Handed to the queue, until it gives them an import ID.
    private var pendingSessions: [RecordingSession] = []
    /// Asked for from the mic window while another upload runs: they go next, together.
    private(set) var waitingSessions: [RecordingSession] = []
    @ObservationIgnored private var historySignature = ""
    @ObservationIgnored private var lastRefresh = Date.distantPast
    @ObservationIgnored private var subscriptions: Set<AnyCancellable> = []
    @ObservationIgnored private let recorderPrompt = RecorderPrompt()

    init() {
        let snapshot = TransferQueue.shared.snapshot
        transfer = snapshot
        transferState = snapshot.state
        uploadServer = snapshot.server
        hasUploads = !snapshot.files.isEmpty
        queue.$snapshot.sink { [weak self] in self?.queueChanged($0) }.store(in: &subscriptions)
        queue.$activeSourceCopies.sink { [weak self] in self?.isCopyingSources = $0 > 0 }.store(in: &subscriptions)
        // Load whenever a vault becomes usable: at launch, after signing in, after switching vaults.
        native.$isConnected.removeDuplicates().sink { [weak self] connected in
            self?.isConnected = connected
            if connected { Task { await self?.refreshAll() } }
        }.store(in: &subscriptions)
        native.$isSigningIn.sink { [weak self] in self?.isSigningIn = $0 }.store(in: &subscriptions)
        native.$server.removeDuplicates().sink { [weak self] in self?.vaultChanged($0) }.store(in: &subscriptions)
        NotificationCenter.default.publisher(for: NSApplication.didBecomeActiveNotification)
            .sink { [weak self] _ in Task { await self?.refreshIfStale() } }.store(in: &subscriptions)
        sources.onArrival = { [weak self] in self?.recorderConnected() }
    }

    var vaultName: String { URL(string: server)?.host ?? "Choose Vault" }
    var uploadVaultName: String { URL(string: uploadServer)?.host ?? "" }
    var isTransferring: Bool { ["copying", "uploading", "verifying"].contains(transferState) }
    var canUpload: Bool { ["idle", "verified"].contains(transferState) && !sources.isEjecting }
    /// Imports go to `uploadServer`; while another vault is open in the library they would land
    /// somewhere the person is not looking, so they wait until both agree.
    var canImport: Bool { canUpload && !uploadServer.isEmpty && (server.isEmpty || server == uploadServer) }
    var isReadingSource: Bool { isCopyingSources || transferState == "copying" }
    var canEject: Bool { !sources.devices.isEmpty && !sources.isEjecting && !isReadingSource }

    // MARK: Lifecycle

    func start() async {
        sources.start(timezone: transfer.timezone)
        guard !transfer.server.isEmpty else { return }
        profiles.remember(transfer.server)
        // The session restores the last vault by itself; only an upload-token setup needs selecting.
        if !(native.isConnected && native.server == transfer.server) { try? await native.use(server: transfer.server) }
    }

    /// Polls every few seconds while something is in flight, otherwise once a minute (and when the
    /// app becomes active). Checks often, so work that starts while idle is picked up quickly.
    func run() async {
        while !Task.isCancelled {
            try? await Task.sleep(for: .seconds(4))
            guard !Task.isCancelled else { return }
            importWaiting()
            let busy = library.isProcessing || isTransferring || !sessions.pendingImportIDs(server: uploadServer).isEmpty
            if busy || Date.now.timeIntervalSince(lastRefresh) > 60 { await refresh() }
        }
    }

    func refreshAll() async {
        await refresh()
        await library.refreshPeople()
    }

    private func refresh() async {
        lastRefresh = .now
        await refreshReceipts()
        await library.refresh()
        let signature = library.entries.map { "\($0.id)|\($0.stage)|\($0.activeStage ?? "")" }.joined(separator: ",")
        if signature != historySignature {
            historySignature = signature
            await synchronizeHistory()
        }
    }

    private func refreshIfStale() async {
        if Date.now.timeIntervalSince(lastRefresh) > 10 { await refresh() }
    }

    private func vaultChanged(_ server: String) {
        self.server = server
        library.reset()
        historySignature = ""
        // A transcript or Trash of the previous vault must not stay on screen.
        if case .entry = selection { selection = .importer }
        if selection == .trash { selection = .importer }
        // Connecting another vault while connected does not change `isConnected`: load here too.
        Task { if native.isConnected, native.server == server, !server.isEmpty { await refreshAll() } }
    }

    private func queueChanged(_ snapshot: QueueState) {
        let previous = transfer
        transfer = snapshot
        if transferState != snapshot.state { transferState = snapshot.state }
        if uploadServer != snapshot.server { uploadServer = snapshot.server }
        if hasUploads == snapshot.files.isEmpty { hasUploads = !snapshot.files.isEmpty }
        // Cleared before the vault created entries: those sessions are not imported after all.
        if let cleared = previous.importID, snapshot.importID == nil {
            sessions.dropUnfinished(server: previous.server, importID: cleared)
        }
        if let importID = snapshot.importID, !pendingSessions.isEmpty {
            sessions.register(pendingSessions, server: snapshot.server, importID: importID)
            pendingSessions = []
        }
        if snapshot.state == "verified" && previous.state != "verified" {
            Task { await refresh() }
        }
        // The recorder clock changed in Settings: recording times have to be read again.
        if snapshot.timezone != previous.timezone { sources.setTimezone(snapshot.timezone) }
    }

    // MARK: Vaults

    func switchVault(to server: String) async {
        guard canUpload else { return }
        preview.stop()
        let previous = native.server
        do {
            try await native.use(server: server)
            try await queue.configureNative(server: native.server, timezone: transfer.timezone)
            profiles.remember(native.server)
        } catch {
            // Stay on the vault that works rather than showing one that cannot be reached.
            if !previous.isEmpty, previous != server { try? await native.use(server: previous) }
            report("Could not open \(URL(string: server)?.host ?? server)", error)
        }
    }

    func openInBrowser(_ path: String = "") {
        guard let url = URL(string: server + path) else { return }
        NSWorkspace.shared.open(url)
    }

    // MARK: Importing

    func chooseSourceFolder() {
        guard !sources.isEjecting else { return }
        let panel = NSOpenPanel()
        panel.canChooseDirectories = true
        panel.canChooseFiles = false
        panel.prompt = "Choose"
        panel.message = "Choose the folder that contains the recordings. Nothing in it will be changed."
        guard panel.runModal() == .OK, let url = panel.url else { return }
        sources.chooseFolder(url)
        selection = .importer
    }

    func chooseAudioFiles() {
        guard canImport else { return }
        let panel = NSOpenPanel()
        panel.allowsMultipleSelection = true
        panel.allowedContentTypes = [.wav, .mp3, .mpeg4Audio, UTType(filenameExtension: "flac") ?? .audio]
        panel.prompt = "Import"
        panel.message = "Choose recordings to send to \(uploadVaultName). The files themselves are not changed."
        guard panel.runModal() == .OK else { return }
        importFiles(panel.urls)
    }

    /// A mic was plugged in while the app runs: offer its new recordings in a small window.
    private func recorderConnected() {
        sessions.update(sources.shownFiles, server: uploadServer)
        if !sessions.proposed(server: uploadServer).isEmpty { recorderPrompt.show(self) }
    }

    /// One session from the mic window: now, or after the current upload, as the queue takes one
    /// import at a time.
    func importNext(_ session: RecordingSession) {
        waitingSessions.append(session)
        importWaiting()
    }

    /// Asked for, but not yet in the queue's import.
    func isQueued(_ session: RecordingSession) -> Bool {
        waitingSessions.contains { $0.id == session.id } || pendingSessions.contains { $0.id == session.id }
    }

    private func importWaiting() {
        guard !waitingSessions.isEmpty, canImport, pendingSessions.isEmpty else { return }
        // Imported meanwhile, or the mic was unplugged.
        let next = waitingSessions.filter { waiting in
            sessions.record(for: waiting, server: uploadServer) == nil && sessions.sessions.contains { $0.id == waiting.id }
        }
        waitingSessions = []
        if !next.isEmpty { importSessions(next) }
    }

    func importSessions(_ selected: [RecordingSession]) {
        let plan = sessions.uploadPlan(selected)
        pendingSessions = selected
        enqueue(plan.urls, plans: plan.plans, sourceIDs: plan.sourceIDs)
    }

    /// Individual files are not on the Import screen, so their progress is shown under Uploads.
    func importFiles(_ urls: [URL]) {
        guard canImport else { return }
        selection = .uploads
        enqueue(urls)
    }

    private func enqueue(_ urls: [URL], plans: [ImportSessionPlan] = [], sourceIDs: [String: String] = [:]) {
        guard canImport else { return }
        preview.stop()
        Task {
            do { try await queue.enqueue(urls, sessions: plans, sourceIDs: sourceIDs) } catch {
                pendingSessions = []
                report("Could not import the recordings", error)
            }
        }
    }

    /// Also cancels processing: the vault stops the recording after its current step.
    func moveToTrash(_ id: String) {
        if selection == .entry(id) { selection = .importer }
        Task { await setProcessing(.trash, entry: id) }
    }

    func setProcessing(_ action: Library.Action, entry id: String) async {
        do {
            try await library.perform(action, on: id)
            if action == .resume { sessions.setStage(entry: id, stage: "queued") }
            if action == .pause { sessions.setPausing(entry: id) }
        } catch {
            report("Could not change processing", error)
        }
    }

    func ejectRecorders() async {
        guard canEject else { return }
        preview.stop()
        let failures = await sources.ejectAll(isCopying: isReadingSource)
        if !failures.isEmpty {
            problem = Problem(title: "Could not eject all microphones",
                              message: failures.joined(separator: "\n\n") + "\n\nClose other apps using the microphones and try again.")
        }
    }

    func report(_ title: String, _ error: Error) {
        guard !(error is CancellationError) else { return }
        problem = Problem(title: title, message: error.localizedDescription)
    }

    // MARK: Source history

    private func refreshReceipts() async {
        let server = uploadServer
        guard !server.isEmpty, isConnected, server == native.server else { return }
        var ids = sessions.pendingImportIDs(server: server)
        if let current = transfer.importID { ids.insert(current) }
        for id in ids {
            // A failed poll is retried on the next tick; durable transfer errors show under Uploads.
            guard let receipt = try? await VaultAPI.get("/api/v1/imports/\(VaultAPI.component(id))", as: ImportReceipt.self) else { continue }
            sessions.updateReceipt(server: server, importID: id, entries: receipt.entries)
            if receipt.state == "failed" {
                sessions.failImport(server: server, importID: id, message: "The vault could not verify this import. Import it again.")
            }
        }
    }

    /// Marks recorder sessions that were imported before, from this Mac or elsewhere.
    private func synchronizeHistory() async {
        let server = native.server
        guard isConnected, !server.isEmpty else { return }
        var entryIDs: Set<String> = []
        var cursor: Int? = 0
        do {
            while let page = cursor {
                let result = try await VaultAPI.get("/api/v1/source-history?limit=500&cursor=\(page)", as: Page<SourceHistoryEntry>.self)
                guard server == native.server else { return }
                entryIDs.formUnion(result.items.map(\.id))
                sessions.syncHistory(result.items, server: server)
                cursor = result.nextCursor
            }
            sessions.reconcileHistory(server: server, entryIDs: entryIDs)
        } catch {
            // Older servers have no source history; import receipts still mark sessions.
        }
    }
}
