import SwiftUI

/// The recorder screen: sessions on the recorder (one tab per microphone), the vault they go to, one Import button.
struct ImportView: View {
    @Environment(AppModel.self) private var app
    @AppStorage("showFragments") private var showFragments = false

    private var source: SourceSessions { app.sessions }
    private var server: String { app.uploadServer }

    var body: some View {
        Group {
            if app.sources.hasSource {
                sessionList
            } else {
                ContentUnavailableView {
                    Label(app.sources.isScanning ? "Looking for Microphones…" : "Connect the DJI Mic", systemImage: "mic")
                } description: {
                    Text("Recordings appear here when the microphone is connected. You can also choose the folder that contains them.")
                } actions: {
                    Button("Choose Source Folder…") { app.chooseSourceFolder() }
                }
            }
        }
        .navigationTitle(app.sources.showsRecorders ? "DJI Mic" : app.sources.folder?.lastPathComponent ?? "Import")
        .navigationSubtitle(app.sources.sourceDescription)
        .toolbar { toolbar }
        .safeAreaInset(edge: .bottom, spacing: 0) {
            if app.sources.hasSource {
                VStack(spacing: 0) {
                    if let session = app.preview.session {
                        PlayerBar(player: app.preview.player, clock: app.preview.clock(atSeconds:),
                                  title: Format.sessionTitle(session.start, session.end),
                                  marked: source.trim(for: session).map { $0.start...$0.end }, onClose: app.preview.stop)
                        TrimBar(session: session)
                        if let error = app.preview.error {
                            Text(error).font(.caption).foregroundStyle(.red).padding(.bottom, 6)
                        }
                    }
                    DestinationBar(pending: pendingSessions, selectable: selectableSessions)
                }
            }
        }
        .onAppear { source.update(app.sources.shownFiles, server: server) }
        .onChange(of: app.sources.shownFiles) { _, files in source.update(files, server: server) }
        // A preview of something no longer listed (recorder unplugged, regrouped) stops.
        .onChange(of: source.sessions) { _, sessions in
            if let current = app.preview.session, !sessions.contains(where: { $0.id == current.id }) { app.preview.stop() }
        }
        .onDisappear { app.preview.stop() }
    }

    @ToolbarContentBuilder
    private var toolbar: some ToolbarContent {
        if source.transmitters.count > 1 {
            ToolbarItem(placement: .principal) {
                Picker("Microphone", selection: Binding(get: { source.visibleTransmitter ?? "" }, set: { source.transmitterTab = $0 })) {
                    ForEach(source.transmitters, id: \.self) { Text($0).tag($0) }
                }
                .pickerStyle(.segmented)
                .labelsHidden()
                .help("Each microphone has its own list")
            }
        }
        if !app.sources.devices.isEmpty {
            ToolbarItem(placement: .principal) {
                Button(app.sources.isEjecting ? "Ejecting…" : "Eject All Mics", systemImage: "eject") {
                    Task { await app.ejectRecorders() }
                }
                .labelStyle(.titleAndIcon)
                .disabled(!app.canEject)
                .help(app.isReadingSource ? "Wait for the recordings to finish copying" : "Safely eject all connected DJI microphones")
            }
        }
        ToolbarItemGroup {
            Button("Rescan", systemImage: "arrow.clockwise") {
                app.sources.refresh()
                if let folder = app.sources.folder { app.sources.chooseFolder(folder) }
            }
            .help("Look for recordings again")
            Button("Choose Source Folder…", systemImage: "folder") { app.chooseSourceFolder() }
                .help("Choose Source Folder…")
        }
    }

    /// Selected sessions that still have something to import.
    private var pendingSessions: [RecordingSession] {
        source.selectedSessions.filter { source.record(for: $0, server: server) == nil }
    }

    /// Button-press fragments of a second or two. Hidden unless asked for, or already imported.
    private func isFragment(_ session: RecordingSession) -> Bool {
        session.duration < 10 && session.recordings.count == 1 && source.record(for: session, server: server) == nil
    }

    /// Visible sessions grouped by the day they started, newest day first, in time order within a day.
    private var days: [(day: Date, sessions: [RecordingSession])] {
        let visible = showFragments ? source.visibleSessions : source.visibleSessions.filter { !isFragment($0) }
        return Dictionary(grouping: visible) { Calendar.current.startOfDay(for: $0.start) }
            .map { (day: $0.key, sessions: $0.value) }
            .sorted { $0.day > $1.day }
    }

    /// Listed sessions that could still be imported.
    private var selectableSessions: [RecordingSession] {
        days.flatMap(\.sessions).filter { source.record(for: $0, server: server) == nil }
    }

    private var sessionList: some View {
        let fragments = source.visibleSessions.filter(isFragment).count
        return List {
            if source.visibleSessions.isEmpty {
                Text(app.sources.isReading ? "Reading recordings…" : "No recordings found here.")
                    .foregroundStyle(.secondary)
            }
            ForEach(days, id: \.day) { day in
                Section(day.day.formatted(.dateTime.weekday(.wide).day().month(.wide))) {
                    ForEach(day.sessions) { session in
                        SessionRow(session: session)
                    }
                }
            }
            if fragments > 0 {
                Button(showFragments ? "Hide recordings shorter than 10 seconds"
                       : "Show \(fragments) \(fragments == 1 ? "recording" : "recordings") shorter than 10 seconds") {
                    showFragments.toggle()
                }
                .buttonStyle(.link)
                .font(.callout)
            }
            if !source.problems.isEmpty {
                Section("Unreadable Files") {
                    ForEach(source.problems) { problem in
                        VStack(alignment: .leading, spacing: 2) {
                            Text(problem.filename).font(.callout.monospaced())
                            Text(problem.message).font(.caption).foregroundStyle(.secondary)
                        }
                    }
                }
            }
        }
        .listStyle(.inset)
        // Sections added after the first render keep a stale row height on macOS 27 (rows get
        // clipped to one line). A new identity per set of days makes the list measure again.
        .id(days.map(\.day))
        .overlay(alignment: .topTrailing) {
            if app.sources.isReading { ProgressView().controlSize(.small).padding(10) }
        }
    }
}

private struct SessionRow: View {
    let session: RecordingSession
    @Environment(AppModel.self) private var app
    @State private var isExpanded = false

    private var source: SourceSessions { app.sessions }
    private var record: SessionImportRecord? { source.record(for: session, server: app.uploadServer) }
    private var trim: Trim? { source.trim(for: session) }
    /// Imported or on its way: grouping and trims are settled.
    private var isLocked: Bool { record.map { !["failed", "paused"].contains($0.stage) } ?? false }

    var body: some View {
        HStack(alignment: .firstTextBaseline, spacing: 10) {
            Toggle("Import", isOn: Binding(get: { source.isSelected(session) && record == nil },
                                           set: { source.setSelected($0, session) }))
                .toggleStyle(.checkbox)
                .labelsHidden()
                .disabled(record != nil || !app.canUpload)

            VStack(alignment: .leading, spacing: 4) {
                HStack(alignment: .firstTextBaseline) {
                    Text(Format.sessionTitle(session.start(trimmedBy: trim), session.end(trimmedBy: trim)))
                        .fontWeight(.medium)
                    Spacer()
                    Text(Format.duration(trim?.duration ?? session.duration))
                        .monospacedDigit()
                        .foregroundStyle(.secondary)
                    Button {
                        app.preview.play(session, at: app.preview.session?.id == session.id ? nil : trim?.start)
                    } label: {
                        Image(systemName: app.preview.isPlaying(session) ? "pause.circle.fill" : "play.circle")
                            .font(.title3)
                    }
                    .buttonStyle(.borderless)
                    .help("Listen before importing: all of this session's files play as one recording")
                    .accessibilityLabel(app.preview.isPlaying(session) ? "Pause preview" : "Play preview")
                }
                HStack(spacing: 10) {
                    if trim != nil, record == nil {
                        Label("Trimmed", systemImage: "scissors")
                            .foregroundStyle(.orange)
                            .help("Imports \(Format.timeRange(session.start(trimmedBy: trim), session.end(trimmedBy: trim))) of \(Format.timeRange(session.start, session.end)). Play the session to change it.")
                    }
                    Button {
                        withAnimation(.easeOut(duration: 0.15)) { isExpanded.toggle() }
                    } label: {
                        HStack(spacing: 4) {
                            Image(systemName: "chevron.right")
                                .font(.caption2.weight(.semibold))
                                .rotationEffect(.degrees(isExpanded ? 90 : 0))
                            Text(Format.sourceFiles(session.recordings.count))
                        }
                        .contentShape(Rectangle())
                    }
                    .buttonStyle(.plain)
                    .foregroundStyle(.secondary)
                    .accessibilityLabel("\(Format.sourceFiles(session.recordings.count)), \(isExpanded ? "collapse" : "expand")")

                    status
                    Spacer(minLength: 0)
                }
                .font(.callout)
                if isExpanded { chunks }
            }
        }
        .padding(.vertical, 4)
        .contextMenu {
            Button("Merge with Previous") { source.mergeWithPrevious(session) }
                .disabled(source.previous(of: session) == nil || isLocked)
            if session.recordings.count > 1 {
                Menu("Split Before") {
                    ForEach(session.recordings.dropFirst()) { recording in
                        Button("\(recording.filename) (\(SessionTimeline.clock(recording.start, in: .current, seconds: false)))") {
                            source.split(session, before: recording)
                        }
                    }
                }
                .disabled(isLocked)
            }
        }
    }

    @ViewBuilder private var status: some View {
        if let record {
            let text = Entry.describe(stage: record.stage, activeStage: record.activeStage, pauseRequested: record.pauseRequested)
            if record.stage == "complete" {
                Label("Imported", systemImage: "checkmark.circle.fill")
                    .foregroundStyle(.green)
                if let id = record.entryID {
                    Button("Open Transcript") { app.preview.stop(); app.selection = .entry(id) }
                        .buttonStyle(.link)
                }
            } else if record.stage == "failed" || record.stage == "paused" {
                Label(record.failure ?? text, systemImage: record.stage == "failed" ? "exclamationmark.circle" : "pause.circle")
                    .foregroundStyle(record.stage == "failed" ? .orange : .secondary)
                    .lineLimit(2)
                    .help(record.failure ?? "")
                if let id = record.entryID {
                    Button("Resume") { Task { await app.setProcessing(.resume, entry: id) } }
                        .buttonStyle(.link)
                }
            } else {
                ProgressView().controlSize(.mini)
                Text(text).foregroundStyle(.secondary)
                if let id = record.entryID, record.pauseRequested != true {
                    Button("Pause") { Task { await app.setProcessing(.pause, entry: id) } }
                        .buttonStyle(.link)
                }
            }
        } else if source.partlyImported(session, server: app.uploadServer) {
            Text("Partly imported before")
                .foregroundStyle(.secondary)
                .help("Some of these files belong to an earlier import that was grouped differently.")
        } else if session.looksLikeContinuation, source.previous(of: session) != nil {
            Text("Starts right after the previous recording")
                .foregroundStyle(.secondary)
            Button("Merge") { source.mergeWithPrevious(session) }
                .buttonStyle(.link)
        }
    }

    private var chunks: some View {
        VStack(alignment: .leading, spacing: 3) {
            ForEach(Array(session.recordings.enumerated()), id: \.element.id) { index, recording in
                HStack(spacing: 8) {
                    Button("Play from here", systemImage: "play.fill") {
                        app.preview.play(session, at: SessionPreview.offset(of: recording, in: session))
                    }
                    .labelStyle(.iconOnly)
                    .buttonStyle(.borderless)
                    .font(.caption)
                    .foregroundStyle(.secondary)
                    .help("Play the session from the start of this file")
                    Text(recording.filename)
                        .font(.callout.monospaced())
                        .lineLimit(1)
                        .truncationMode(.middle)
                    Text(Format.timeRange(recording.start, recording.end))
                        .monospacedDigit()
                        .foregroundStyle(.secondary)
                    Spacer(minLength: 8)
                    if index == 0 {
                        if source.previous(of: session) != nil {
                            Button("Merge with Previous") { source.mergeWithPrevious(session) }
                                .disabled(isLocked)
                        }
                    } else {
                        Button("Split Here") { source.split(session, before: recording) }
                            .disabled(isLocked)
                    }
                }
                .font(.callout)
                .buttonStyle(.link)
            }
            Text(session.recordings[0].format.summary)
                .font(.caption)
                .foregroundStyle(.tertiary)
                .padding(.top, 1)
        }
        .padding(.top, 4)
        .padding(.leading, 14)
    }
}

/// Always visible: where the recordings go, transfer progress, and the Import button.
private struct DestinationBar: View {
    let pending: [RecordingSession]
    let selectable: [RecordingSession]
    @Environment(AppModel.self) private var app

    var body: some View {
        VStack(spacing: 0) {
            Divider()
            HStack(spacing: 12) {
                if app.isTransferring || app.transferState == "failed" || app.transferState == "paused" {
                    TransferSummary()
                } else {
                    VStack(alignment: .leading, spacing: 2) {
                        Text("Import into vault")
                            .font(.caption)
                            .foregroundStyle(.secondary)
                        if app.uploadServer.isEmpty {
                            Text("Not connected").foregroundStyle(.secondary)
                        } else {
                            Label(app.uploadVaultName, systemImage: "books.vertical")
                                .lineLimit(1)
                                .truncationMode(.middle)
                        }
                    }
                }
                Spacer()
                Button(pending.isEmpty ? "Select All" : "Select None") {
                    let select = pending.isEmpty
                    for session in selectable { app.sessions.setSelected(select, session) }
                }
                .buttonStyle(.link)
                .disabled(selectable.isEmpty || !app.canUpload)
                if app.uploadServer.isEmpty {
                    SettingsLink { Text("Connect Vault…") }
                } else {
                    Button(pending.isEmpty ? "Import" : "Import \(pending.count)") {
                        app.importSessions(pending)
                    }
                    .keyboardShortcut(.defaultAction)
                    .disabled(pending.isEmpty || !app.canImport)
                }
            }
            .padding(.horizontal, 16)
            .padding(.vertical, 10)
        }
        .background(.bar)
    }
}

/// Under the preview player: set where the import starts and ends from the playback position.
private struct TrimBar: View {
    let session: RecordingSession
    @Environment(AppModel.self) private var app

    var body: some View {
        let source = app.sessions, preview = app.preview
        let trim = source.trim(for: session)
        let kept = trim ?? Trim(start: 0, end: session.duration)
        ViewThatFits(in: .horizontal) {
            HStack(spacing: 10) {
                Image(systemName: "scissors").foregroundStyle(.secondary)
                bounds(kept)
                Spacer()
                summary(trim, kept)
            }
            VStack(alignment: .leading, spacing: 6) {
                HStack(spacing: 10) { bounds(kept) }
                summary(trim, kept)
            }
        }
        .font(.callout)
        .controlSize(.small)
        .padding(.horizontal, 16)
        .padding(.bottom, 8)
        .background(.bar)
        .disabled(!preview.player.isAvailable)
    }

    @ViewBuilder private func bounds(_ kept: Trim) -> some View {
        let source = app.sessions, preview = app.preview
        Text("Import from").foregroundStyle(.secondary)
        Text(preview.clock(atSeconds: kept.start)).monospacedDigit()
        Button("Set Start") { source.setTrim(start: preview.player.currentSeconds, for: session) }
            .help("Start the import at the playback position")
        Text("to").foregroundStyle(.secondary)
        Text(preview.clock(atSeconds: kept.end)).monospacedDigit()
        Button("Set End") { source.setTrim(end: preview.player.currentSeconds, for: session) }
            .help("End the import at the playback position")
        if source.trim(for: session) != nil {
            Button("Reset") { source.resetTrim(for: session) }
                .buttonStyle(.link)
        }
    }

    private func summary(_ trim: Trim?, _ kept: Trim) -> some View {
        Text(trim == nil ? "Whole recording" : "\(Format.duration(kept.duration)) of \(Format.duration(session.duration))")
            .monospacedDigit()
            .foregroundStyle(.secondary)
    }
}
