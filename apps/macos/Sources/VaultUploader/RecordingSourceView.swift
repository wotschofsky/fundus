import SwiftUI

/// The source screen intentionally follows Mic Ingest: sessions, not individual files.
struct RecordingSourceView: View {
    let files: [RecorderFile]
    let isScanning: Bool
    let title: String
    let subtitle: String
    let connected: Bool
    let source: SourceSessions
    let preview: SessionPreview
    let server: String
    let canUpload: Bool
    let isConnected: Bool
    let chooseFolder: () -> Void
    let upload: ([RecordingSession]) -> Void
    let connect: () -> Void
    let openEntry: (String) -> Void
    let retryEntry: (String) -> Void
    let pauseEntry: (String) -> Void
    @AppStorage("showFragments") private var showFragments = false

    private var visible: [RecordingSession] {
        source.visibleSessions.filter { showFragments || $0.duration >= 10 || $0.recordings.count > 1 || source.record(for: $0, server: server) != nil }
    }
    private var fragments: Int { source.visibleSessions.filter { $0.duration < 10 && $0.recordings.count == 1 && source.record(for: $0, server: server) == nil }.count }
    private var days: [(day: Date, sessions: [RecordingSession])] {
        Dictionary(grouping: visible) { Calendar.current.startOfDay(for: $0.start) }
            .map { (day: $0.key, sessions: $0.value) }.sorted { $0.day > $1.day }
    }
    private var pending: [RecordingSession] { source.selectedSessions.filter { source.record(for: $0, server: server) == nil } }

    var body: some View {
        Group {
            if !connected {
                ContentUnavailableView {
                    Label(isScanning ? "Looking for microphones…" : "Connect the DJI Mic", systemImage: "mic")
                } description: {
                    Text("Recordings appear automatically when the microphone is connected. You can also choose a folder.")
                } actions: { Button("Choose Source Folder…", action: chooseFolder) }
            } else {
                List {
                    if source.sessions.isEmpty && source.problems.isEmpty {
                        Text(isScanning ? "Reading recordings…" : "No recordings found here.").foregroundStyle(.secondary)
                    }
                    ForEach(days, id: \.day) { day in
                        Section(day.day.formatted(.dateTime.weekday(.wide).day().month(.wide))) {
                            ForEach(day.sessions) { session in
                                SourceSessionRow(session: session, source: source, preview: preview, server: server,
                                                 canUpload: canUpload, openEntry: openEntry, retryEntry: retryEntry, pauseEntry: pauseEntry)
                            }
                        }
                    }
                    if fragments > 0 {
                        Button(showFragments ? "Hide recordings shorter than 10 seconds" : "Show \(fragments) recordings shorter than 10 seconds") { showFragments.toggle() }
                            .buttonStyle(.link).font(.callout)
                    }
                    if !source.problems.isEmpty {
                        Section("Unreadable Files") {
                            ForEach(source.problems) { problem in
                                VStack(alignment: .leading, spacing: 3) {
                                    Text(problem.filename).font(.callout).lineLimit(1).truncationMode(.middle)
                                    Text(problem.message).font(.caption).foregroundStyle(.secondary)
                                }
                            }
                        }
                    }
                }
                .listStyle(.inset)
                .overlay(alignment: .topTrailing) { if isScanning { ProgressView().controlSize(.small).padding(12) } }
            }
        }
        .frame(maxWidth: .infinity, maxHeight: .infinity)
        .navigationTitle(title)
        .navigationSubtitle(subtitle)
        .toolbar {
            if source.transmitters.count > 1 {
                ToolbarItem(placement: .principal) {
                    Picker("Microphone", selection: Binding(get: { source.visibleTransmitter ?? "" }, set: { source.transmitterTab = $0 })) {
                        ForEach(source.transmitters, id: \.self) { Text($0).tag($0) }
                    }
                    .pickerStyle(.segmented).labelsHidden().help("Each microphone has its own recording sessions")
                }
            }
        }
        .safeAreaInset(edge: .bottom, spacing: 0) {
            if connected {
                VStack(spacing: 0) {
                    if let session = preview.session {
                        PlayerBar(player: preview.player, clock: preview.clock(atSeconds:), title: Format.sessionTitle(session.start, session.end),
                                  marked: source.trim(for: session).map { $0.start...$0.end }, onClose: preview.stop)
                        SourceTrimBar(session: session, source: source, preview: preview)
                        if let error = preview.error { Text(error).font(.caption).foregroundStyle(.red).padding(.bottom, 6) }
                    }
                    HStack(spacing: 12) {
                        Menu {
                            Button("Select All") { visible.filter { source.record(for: $0, server: server) == nil }.forEach { source.setSelected(true, $0) } }
                            Button("Select None") { visible.forEach { source.setSelected(false, $0) } }
                        } label: { Text("\(pending.count) selected") }
                            .menuStyle(.borderlessButton).fixedSize().disabled(!canUpload)
                        if !pending.isEmpty {
                            Text(Format.duration(pending.reduce(0) { $0 + (source.trim(for: $1)?.duration ?? $1.duration) })).foregroundStyle(.secondary)
                        }
                        Spacer()
                        if !isConnected { Button("Connect Vault…", action: connect) }
                        else {
                            Button(pending.isEmpty ? "Import Recordings" : "Import \(pending.count) \(pending.count == 1 ? "Recording" : "Recordings")") {
                                preview.stop()
                                upload(pending)
                            }
                            .buttonStyle(.borderedProminent).disabled(pending.isEmpty || !canUpload)
                        }
                    }
                    .padding(.horizontal, 16).padding(.vertical, 12)
                }
                .background(.bar)
            }
        }
        .onAppear { source.update(files, server: server) }
        .onChange(of: files) { _, files in source.update(files, server: server) }
        .onChange(of: source.sessions) { _, sessions in
            if let session = preview.session, !sessions.contains(where: { $0.id == session.id }) { preview.stop() }
        }
        .onDisappear { preview.stop() }
    }
}

private struct SourceSessionRow: View {
    let session: RecordingSession
    let source: SourceSessions
    let preview: SessionPreview
    let server: String
    let canUpload: Bool
    let openEntry: (String) -> Void
    let retryEntry: (String) -> Void
    let pauseEntry: (String) -> Void
    @State private var expanded = false
    private var record: SessionImportRecord? { source.record(for: session, server: server) }
    private var locked: Bool { record != nil && !["failed", "paused"].contains(record?.stage ?? "") }

    var body: some View {
        HStack(alignment: .firstTextBaseline, spacing: 10) {
            Toggle("Import recording at \(Format.timeRange(session.start, session.end))", isOn: Binding(get: { source.isSelected(session) && record == nil }, set: { source.setSelected($0, session) }))
                .toggleStyle(.checkbox).labelsHidden().disabled(record != nil || !canUpload)
            VStack(alignment: .leading, spacing: 4) {
                HStack(alignment: .firstTextBaseline) {
                    let trim = source.trim(for: session)
                    Text(Format.timeRange(session.start(trimmedBy: trim), session.end(trimmedBy: trim))).fontWeight(.medium)
                    Spacer()
                    Text(Format.duration(trim?.duration ?? session.duration)).monospacedDigit().foregroundStyle(.secondary)
                    Button { preview.play(session, at: preview.session?.id == session.id ? nil : trim?.start) } label: {
                        Image(systemName: preview.isPlaying(session) ? "pause.circle.fill" : "play.circle").font(.title3)
                    }
                    .buttonStyle(.borderless).accessibilityLabel(preview.isPlaying(session) ? "Pause preview" : "Play preview")
                }
                HStack(spacing: 10) {
                    if source.trim(for: session) != nil { Label("Trimmed", systemImage: "scissors").foregroundStyle(.secondary) }
                    Button { expanded.toggle() } label: {
                        HStack(spacing: 4) {
                            Image(systemName: expanded ? "chevron.down" : "chevron.right").font(.caption2)
                            Text(Format.sourceFiles(session.recordings.count))
                        }
                    }
                    .buttonStyle(.plain).foregroundStyle(.secondary)
                    status
                    Spacer(minLength: 0)
                }.font(.callout)
                if expanded { chunks }
            }
        }
        .padding(.vertical, 5)
        .contextMenu {
            Button("Merge with Previous") { source.mergeWithPrevious(session) }.disabled(source.previous(of: session) == nil || locked || !canUpload)
            if session.recordings.count > 1 {
                Menu("Split Before") {
                    ForEach(session.recordings.dropFirst()) { file in
                        Button(file.filename) { source.split(session, before: file) }
                    }
                }.disabled(locked || !canUpload)
            }
        }
    }

    @ViewBuilder private var status: some View {
        if let record {
            if record.stage == "complete" {
                Label("Imported", systemImage: "checkmark.circle").foregroundStyle(.secondary)
                if let id = record.entryID { Button("Open Transcript") { openEntry(id) }.buttonStyle(.link) }
            } else if ["failed", "paused"].contains(record.stage) {
                Text(record.failure ?? (record.stage == "paused" ? "Paused" : "Not finished")).foregroundStyle(record.stage == "failed" ? Color.red : Color.secondary).lineLimit(2)
                if let id = record.entryID { Button("Resume") { retryEntry(id) }.buttonStyle(.link) }
            } else {
                ProgressView().controlSize(.mini)
                Text(record.pauseRequested == true ? "Pausing after current step…" : (record.activeStage ?? record.stage).capitalized).foregroundStyle(.secondary)
                if let id = record.entryID { Button("Pause") { pauseEntry(id) }.buttonStyle(.link).disabled(record.pauseRequested == true) }
            }
        } else if source.partlyImported(session, server: server) {
            Text("Partly imported before").foregroundStyle(.secondary)
        } else if session.looksLikeContinuation, source.previous(of: session) != nil {
            Text("Starts right after the previous recording").foregroundStyle(.secondary)
            Button("Merge") { source.mergeWithPrevious(session) }.buttonStyle(.link).disabled(!canUpload)
        }
    }

    private var chunks: some View {
        VStack(alignment: .leading, spacing: 4) {
            ForEach(Array(session.recordings.enumerated()), id: \.element.id) { index, file in
                HStack(spacing: 8) {
                    Button("Play from here", systemImage: "play.fill") { preview.play(session, at: SessionPreview.offset(of: file, in: session)) }
                        .labelStyle(.iconOnly).buttonStyle(.borderless).font(.caption)
                    Text(file.filename).font(.callout).lineLimit(1).truncationMode(.middle)
                    Text(Format.timeRange(file.start, file.end)).monospacedDigit().foregroundStyle(.secondary)
                    Spacer(minLength: 4)
                    if index == 0, source.previous(of: session) != nil {
                        Button("Merge with Previous") { source.mergeWithPrevious(session) }.disabled(locked || !canUpload)
                    } else if index > 0 {
                        Button("Split Here") { source.split(session, before: file) }.disabled(locked || !canUpload)
                    }
                }.buttonStyle(.link)
            }
            Text(session.recordings[0].format.summary).font(.caption).foregroundStyle(.secondary).padding(.top, 2)
        }.padding(.top, 5).padding(.leading, 12)
    }
}

private struct SourceTrimBar: View {
    let session: RecordingSession
    let source: SourceSessions
    let preview: SessionPreview
    var body: some View {
        let trim = source.trim(for: session) ?? Trim(start: 0, end: session.duration)
        HStack(spacing: 10) {
            Image(systemName: "scissors").foregroundStyle(.secondary)
            Text("Import from").foregroundStyle(.secondary)
            Text(preview.clock(atSeconds: trim.start)).monospacedDigit()
            Button("Set Start") { source.setTrim(start: preview.player.currentSeconds, for: session) }
            Text("to").foregroundStyle(.secondary)
            Text(preview.clock(atSeconds: trim.end)).monospacedDigit()
            Button("Set End") { source.setTrim(end: preview.player.currentSeconds, for: session) }
            if source.trim(for: session) != nil { Button("Reset") { source.resetTrim(for: session) }.buttonStyle(.link) }
            Spacer()
            Text(source.trim(for: session) == nil ? "Whole recording" : "\(Format.duration(trim.duration)) of \(Format.duration(session.duration))").foregroundStyle(.secondary)
        }
        .font(.callout).controlSize(.small)
        .padding(.horizontal, 16).padding(.bottom, 10)
        .disabled(!preview.player.isAvailable)
    }
}
