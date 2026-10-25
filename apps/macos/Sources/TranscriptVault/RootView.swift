import SwiftUI

struct RootView: View {
    @Environment(AppModel.self) private var app
    @State private var search = ""

    var body: some View {
        @Bindable var app = app
        NavigationSplitView {
            Sidebar(search: search)
                .searchable(text: $search, placement: .sidebar, prompt: "Search")
                .navigationSplitViewColumnWidth(min: 220, ideal: 270, max: 380)
        } detail: {
            Group {
                switch app.selection {
                case .entry(let id):
                    TranscriptView(entryID: id, library: app.library)
                        .id(id)
                case .uploads:
                    UploadsView()
                case .trash:
                    TrashView()
                default:
                    ImportView()
                }
            }
            .frame(maxWidth: .infinity, maxHeight: .infinity)
        }
        .disabled(app.sources.isEjecting)
        .task { await app.start() }
        .task { await app.run() }
        // A recorder that was just plugged in is what the person wants to look at.
        .onChange(of: app.sources.devices.map(\.id)) { old, new in
            if !Set(new).subtracting(old).isEmpty { app.selection = .importer }
        }
        .onChange(of: app.hasUploads) { _, has in
            if !has, app.selection == .uploads { app.selection = .importer }
        }
        .alert(app.problem?.title ?? "", isPresented: .init(get: { app.problem != nil }, set: { if !$0 { app.problem = nil } })) {
            Button("OK", role: .cancel) {}
        } message: {
            Text(app.problem?.message ?? "")
        }
    }
}

private struct Sidebar: View {
    let search: String
    @Environment(AppModel.self) private var app

    private var entries: [Entry] {
        let query = search.trimmingCharacters(in: .whitespaces)
        guard !query.isEmpty else { return app.library.entries }
        return app.library.entries.filter {
            $0.displayTitle.localizedCaseInsensitiveContains(query)
                || ($0.summary ?? "").localizedCaseInsensitiveContains(query)
                || $0.labels.contains { $0.localizedCaseInsensitiveContains(query) }
        }
    }

    var body: some View {
        @Bindable var app = app
        List(selection: $app.selection) {
            Label("Import", systemImage: "square.and.arrow.down")
                .tag(SidebarItem.importer)
            if app.hasUploads {
                UploadsRow()
                    .tag(SidebarItem.uploads)
            }
            Label("Trash", systemImage: "trash")
                .tag(SidebarItem.trash)

            Section("Transcripts") {
                ForEach(entries) { entry in
                    EntryRow(entry: entry)
                        .tag(SidebarItem.entry(entry.id))
                        .contextMenu { EntryMenu(entry: entry) }
                        .onAppear {
                            if entry.id == app.library.entries.last?.id, search.isEmpty { Task { await app.library.loadMore() } }
                        }
                }
                if entries.isEmpty {
                    Text(placeholder)
                        .foregroundStyle(.secondary)
                        .selectionDisabled()
                }
            }
        }
        .listStyle(.sidebar)
        .onDeleteCommand {
            guard case .entry(let id) = app.selection, app.library.entry(id)?.isProcessing == false else { return }
            app.selection = .importer
            Task { await app.setProcessing(.trash, entry: id) }
        }
        .safeAreaInset(edge: .bottom, spacing: 0) { VaultSwitcher() }
    }

    private var placeholder: String {
        if !search.isEmpty { return "No matches" }
        if !app.isConnected { return "Not connected" }
        if let error = app.library.error { return error }
        return app.library.hasLoaded ? "No transcripts yet" : "Loading…"
    }
}

private struct EntryRow: View {
    let entry: Entry

    var body: some View {
        HStack(spacing: 8) {
            VStack(alignment: .leading, spacing: 1) {
                Text(entry.displayTitle)
                    .lineLimit(1)
                Text(subtitle)
                    .font(.caption)
                    .foregroundStyle(.secondary)
                    .lineLimit(1)
            }
            Spacer(minLength: 0)
            if entry.isProcessing {
                ProgressView().controlSize(.small)
            } else if entry.stage == "failed" {
                Image(systemName: "exclamationmark.circle")
                    .foregroundStyle(.orange)
                    .help(entry.failure ?? "Processing stopped")
            }
        }
        .padding(.vertical, 2)
    }

    private var subtitle: String {
        if entry.stage != "complete" { return entry.status }
        let length = Format.duration(entry.duration)
        return entry.displayTitle == entry.dateLine ? length : "\(entry.dateLine) · \(length)"
    }
}

/// Actions on a recording, shared by the sidebar's context menu and the transcript's toolbar.
struct EntryMenu: View {
    let entry: Entry
    @Environment(AppModel.self) private var app

    var body: some View {
        Button("Open in Browser") { app.openInBrowser("/entries/\(entry.id)") }
        Divider()
        if entry.isProcessing {
            Button("Pause After Current Step") { Task { await app.setProcessing(.pause, entry: entry.id) } }
                .disabled(entry.pauseRequested)
        } else if entry.canResume {
            Button("Resume Processing") { Task { await app.setProcessing(.resume, entry: entry.id) } }
        } else {
            Button("Reanalyze Speakers") { Task { await app.setProcessing(.reanalyzeSpeakers, entry: entry.id) } }
        }
        Divider()
        Button("Move to Trash", role: .destructive) {
            if app.selection == .entry(entry.id) { app.selection = .importer }
            Task { await app.setProcessing(.trash, entry: entry.id) }
        }
        .disabled(entry.isProcessing)
    }
}

private struct UploadsRow: View {
    @Environment(AppModel.self) private var app

    var body: some View {
        HStack {
            Label("Uploads", systemImage: "arrow.up.circle")
            Spacer()
            if app.isTransferring {
                Text(Upload.progress(app.transfer), format: .percent.precision(.fractionLength(0)))
                    .monospacedDigit()
                    .foregroundStyle(.secondary)
            } else if app.transfer.state == "failed" {
                Image(systemName: "exclamationmark.circle").foregroundStyle(.orange)
            } else if app.transfer.state == "verified" {
                Image(systemName: "checkmark.circle").foregroundStyle(.secondary)
            }
        }
    }
}

/// The connected vault's name at the top of the sidebar; a menu to switch or add vaults.
private struct VaultSwitcher: View {
    @Environment(AppModel.self) private var app

    var body: some View {
        Menu {
            ForEach(app.profiles.profiles) { profile in
                Toggle(profile.name, isOn: .init(get: { app.server == profile.server },
                                                 set: { if $0 { Task { await app.switchVault(to: profile.server) } } }))
                    .disabled(!app.canUpload)
            }
            Divider()
            SettingsLink { Text("Connect Another Vault…") }
            if !app.server.isEmpty {
                Button("Open Vault in Browser") { app.openInBrowser() }
                Button("Reload Library") { Task { await app.refreshAll() } }
            }
        } label: {
            Label(app.server.isEmpty ? "Connect a Vault…" : app.vaultName, systemImage: "books.vertical")
                .fontWeight(.medium)
                .frame(maxWidth: .infinity, alignment: .leading)
        }
        .menuStyle(.button)
        .buttonStyle(.bordered)
        .controlSize(.large)
        .flexibleWidth()
        .padding(10)
        .help("Switch vaults, connect another one, or open the vault in your browser")
    }
}

private extension View {
    /// Lets a bordered control span its container, as on macOS 15.
    @ViewBuilder func flexibleWidth() -> some View {
        if #available(macOS 26.0, *) { buttonSizing(.flexible) } else { frame(maxWidth: .infinity) }
    }
}
