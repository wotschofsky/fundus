import SwiftUI
import UniformTypeIdentifiers

/// Transcript viewer: clock time, speaker, text. Click a passage to hear it.
struct TranscriptView: View {
    @Environment(AppModel.self) private var app
    @State private var model: TranscriptModel
    @State private var newPersonSpeaker: String?
    @State private var newPersonName = ""
    @State private var isEditing = false

    init(entryID: String, library: Library) {
        _model = State(initialValue: TranscriptModel(entryID: entryID, library: library))
    }

    var body: some View {
        VStack(spacing: 0) {
            PlaybackFollower(model: model)
            if let error = model.loadError {
                ContentUnavailableView("Transcript Unavailable", systemImage: "doc.questionmark", description: Text(error))
            } else if model.isLoading && model.entry == nil {
                ProgressView().frame(maxWidth: .infinity, maxHeight: .infinity)
            } else if let entry = model.entry {
                notices(entry)
                speakerBar
                if model.rows.isEmpty {
                    placeholder(entry)
                } else {
                    TranscriptList(model: model, newPerson: askForPerson)
                }
                PlayerBar(player: model.player, clock: model.clock(atSeconds:))
            }
        }
        .navigationTitle(model.entry?.displayTitle ?? "Transcript")
        .navigationSubtitle(subtitle)
        .toolbar { toolbar }
        .task { await model.load() }
        // Restarts whenever processing starts again (Resume, Reanalyze), not only on open.
        .task(id: model.entry?.isProcessing) { await model.watch() }
        .onChange(of: model.entry?.stage) { _, stage in
            if stage != nil { Task { await app.library.refresh() } }
        }
        .onDisappear { model.player.unload() }
        .sheet(isPresented: $isEditing) {
            if let entry = model.entry { EditEntrySheet(entry: entry, save: model.save) }
        }
        .alert("New Person", isPresented: .init(get: { newPersonSpeaker != nil }, set: { if !$0 { newPersonSpeaker = nil } })) {
            TextField("Name", text: $newPersonName)
            Button("Cancel", role: .cancel) {}
            Button("Add") {
                if let label = newPersonSpeaker { Task { await model.addPerson(named: newPersonName, for: label) } }
            }
            .disabled(newPersonName.trimmingCharacters(in: .whitespaces).isEmpty)
        } message: {
            Text("This speaker's voice in this recording is used to recognize them in future recordings.")
        }
    }

    private var subtitle: String {
        guard let entry = model.entry else { return "" }
        let names = model.speakers.compactMap(\.name).joined(separator: ", ")
        return [entry.dateLine, Format.duration(entry.duration), names].filter { !$0.isEmpty }.joined(separator: " · ")
    }

    private func askForPerson(_ label: String) {
        newPersonName = ""
        newPersonSpeaker = label
    }

    @ToolbarContentBuilder
    private var toolbar: some ToolbarContent {
        ToolbarItemGroup {
            Button("Edit Details", systemImage: "pencil") { isEditing = true }
                .disabled(model.entry == nil)
                .help("Edit the title, labels and recording time")
            Button("Export Transcript…", systemImage: "square.and.arrow.up") { export() }
                .disabled(model.rows.isEmpty)
                .help("Save the transcript as a Markdown file")
            if let entry = model.entry {
                Menu("More", systemImage: "ellipsis.circle") { EntryMenu(entry: entry) }
                    .help("More actions for this recording")
            }
        }
    }

    @ViewBuilder private func notices(_ entry: Entry) -> some View {
        let messages = entry.warnings + [model.actionError, model.player.error, entry.stage == "failed" ? entry.failure : nil].compactMap { $0 }
        if !messages.isEmpty {
            HStack(alignment: .firstTextBaseline, spacing: 8) {
                Image(systemName: "exclamationmark.triangle").foregroundStyle(.orange)
                VStack(alignment: .leading, spacing: 2) {
                    ForEach(messages, id: \.self) { Text($0).textSelection(.enabled) }
                }
                Spacer()
                if entry.canResume {
                    Button("Resume Processing") { Task { await app.setProcessing(.resume, entry: entry.id); await model.refresh() } }
                } else if !entry.warnings.isEmpty, entry.isComplete {
                    Button("Retry Speaker Analysis") { Task { await app.setProcessing(.reanalyzeSpeakers, entry: entry.id); await model.refresh() } }
                }
            }
            .font(.callout)
            .padding(.horizontal, 16)
            .padding(.vertical, 8)
            .background(.orange.opacity(0.08))
            Divider()
        }
        if entry.isProcessing || entry.stage == "paused" {
            HStack(spacing: 10) {
                if entry.isProcessing { ProgressView().controlSize(.small) }
                Text(entry.status).foregroundStyle(.secondary)
                Spacer()
                if entry.isProcessing {
                    Button("Pause After Current Step") { Task { await app.setProcessing(.pause, entry: entry.id); await model.refresh() } }
                        .disabled(entry.pauseRequested)
                } else {
                    Button("Resume Processing") { Task { await app.setProcessing(.resume, entry: entry.id); await model.refresh() } }
                }
            }
            .font(.callout)
            .padding(.horizontal, 16)
            .padding(.vertical, 8)
            Divider()
        }
    }

    /// Everyone in the recording at a glance, each name a menu: the obvious place to say who is who.
    @ViewBuilder private var speakerBar: some View {
        if !model.speakers.isEmpty {
            HStack(spacing: 14) {
                Text("Speakers")
                    .foregroundStyle(.secondary)
                ScrollView(.horizontal, showsIndicators: false) {
                    HStack(spacing: 14) {
                        ForEach(model.speakers, id: \.label) { speaker in
                            SpeakerMenu(label: speaker.label, model: model, newPerson: askForPerson)
                        }
                    }
                }
                Spacer(minLength: 8)
                if model.isUpdatingSpeaker {
                    ProgressView().controlSize(.small)
                } else {
                    Text("Click a name to say who it is")
                        .font(.caption)
                        .foregroundStyle(.tertiary)
                }
            }
            .font(.callout)
            .padding(.horizontal, 16)
            .padding(.vertical, 7)
            Divider()
        }
    }

    @ViewBuilder private func placeholder(_ entry: Entry) -> some View {
        if entry.isComplete {
            ContentUnavailableView("No Speech Detected", systemImage: "waveform.slash",
                                   description: Text("The recording was transcribed, but it contains no speech."))
        } else {
            ContentUnavailableView {
                Label(entry.isProcessing ? entry.status : "Not Transcribed Yet", systemImage: "text.bubble")
            } description: {
                Text(entry.audioAvailable ? "You can already listen while the vault works on the transcript."
                     : "The transcript appears here when the vault has processed the recording.")
            }
        }
    }

    private func export() {
        let panel = NSSavePanel()
        panel.nameFieldStringValue = (model.entry?.displayTitle ?? "Transcript").replacingOccurrences(of: "/", with: "-") + ".md"
        panel.allowedContentTypes = [UTType(filenameExtension: "md") ?? .plainText]
        guard panel.runModal() == .OK, let url = panel.url else { return }
        do { try Data(model.markdown().utf8).write(to: url, options: .atomic) } catch { model.actionError = error.localizedDescription }
    }
}

/// Watches the playback position so that only this empty view, not the transcript, updates on every tick.
private struct PlaybackFollower: View {
    let model: TranscriptModel

    var body: some View {
        Color.clear
            .frame(height: 0)
            .onChange(of: model.player.currentSeconds) { model.playbackMoved() }
            .onChange(of: model.player.isPlaying) { model.playbackMoved() }
    }
}

private struct TranscriptList: View {
    let model: TranscriptModel
    let newPerson: (String) -> Void

    var body: some View {
        // List is table-backed and lazy: rows for multi-hour recordings are only built when scrolled to.
        let current = model.currentRowID
        List {
            if let summary = model.entry?.summary, !summary.isEmpty {
                VStack(alignment: .leading, spacing: 4) {
                    Text("Summary")
                        .font(.caption.weight(.semibold))
                        .foregroundStyle(.secondary)
                    Text(summary)
                        .foregroundStyle(.secondary)
                        .textSelection(.enabled)
                }
                .padding(.horizontal, 6)
                .padding(.vertical, 8)
                .listRowSeparator(.hidden)
            }
            ForEach(model.rows) { row in
                UtteranceRow(row: row, isCurrent: current == row.id, model: model, newPerson: newPerson)
                    .listRowSeparator(.hidden)
            }
        }
        .listStyle(.plain)
    }
}

private struct UtteranceRow: View {
    let row: TranscriptRow
    let isCurrent: Bool
    let model: TranscriptModel
    let newPerson: (String) -> Void

    var body: some View {
        HStack(alignment: .firstTextBaseline, spacing: 12) {
            Text(row.clock)
                .font(.callout)
                .monospacedDigit()
                .foregroundStyle(isCurrent ? AnyShapeStyle(.tint) : AnyShapeStyle(.secondary))
                .frame(width: 64, alignment: .leading)
            VStack(alignment: .leading, spacing: 3) {
                SpeakerMenu(label: row.speaker, model: model, newPerson: newPerson)
                Text(row.text)
                    .textSelection(.enabled)
                    .frame(maxWidth: .infinity, alignment: .leading)
            }
        }
        .padding(.vertical, 5)
        .padding(.horizontal, 6)
        .background(isCurrent ? Color.accentColor.opacity(0.08) : .clear, in: RoundedRectangle(cornerRadius: 6))
        .contentShape(Rectangle())
        .onTapGesture { model.play(row) }
        .accessibilityAction(named: "Play from here") { model.play(row) }
    }
}

/// The speaker label is the entry point for identity: a quiet menu, never a wizard.
private struct SpeakerMenu: View {
    let label: String
    let model: TranscriptModel
    let newPerson: (String) -> Void
    @Environment(AppModel.self) private var app

    var body: some View {
        let speaker = model.speaker(label)
        let identified = speaker?.isIdentified == true
        Menu {
            if let reasons = speaker?.explanations, !reasons.isEmpty {
                Section(identified ? "Identified from" : "Not enough to identify") {
                    ForEach(reasons, id: \.self) { Text($0) }
                }
            }
            if speaker?.state == "automatic", let person = app.library.people.first(where: { $0.id == speaker?.personId }) {
                Button("Confirm \(person.name)") { Task { await model.setIdentity(person, for: label) } }
            }
            Section(identified ? "Change to" : "Assign") {
                ForEach(app.library.people) { person in
                    Toggle(person.name, isOn: .init(get: { speaker?.personId == person.id },
                                                    set: { if $0 { Task { await model.setIdentity(person, for: label) } } }))
                }
                Button("New Person…") { newPerson(label) }
            }
            Section {
                if identified {
                    Button("Remove Identity") { Task { await model.setIdentity(nil, for: label, locked: false) } }
                }
                Toggle("Keep Unidentified", isOn: .init(get: { speaker?.keptUnidentified == true },
                                                       set: { keep in Task { await model.setIdentity(nil, for: label, locked: keep) } }))
            }
        } label: {
            HStack(spacing: 3) {
                Text(model.name(of: label))
                    .fontWeight(.semibold)
                // A visible "this is a menu" cue: the name alone does not look clickable.
                Image(systemName: "chevron.down")
                    .font(.caption2.weight(.semibold))
                    .foregroundStyle(.tertiary)
            }
            .foregroundStyle(identified ? .primary : .secondary)
            .contentShape(Rectangle())
        }
        .menuStyle(.button)
        .buttonStyle(.plain)
        .menuIndicator(.hidden)
        .fixedSize()
        .disabled(model.isUpdatingSpeaker)
        .help(identified ? "Change or remove this speaker's identity" : "Say who this speaker is")
    }
}

private struct EditEntrySheet: View {
    let entry: Entry
    let save: (String?, String, Date?) async throws -> Void
    @Environment(\.dismiss) private var dismiss
    @State private var title = ""
    @State private var labels = ""
    @State private var recordedAt = Date()
    @State private var setsDate = false
    @State private var error: String?
    @State private var isSaving = false

    /// Generated or still-default titles are left alone unless the person types a new one.
    private var changedTitle: String? {
        let trimmed = title.trimmingCharacters(in: .whitespacesAndNewlines)
        return trimmed.isEmpty || trimmed == entry.title ? nil : trimmed
    }
    private var changedDate: Date? {
        guard entry.recordedAt != nil || setsDate else { return nil }
        return abs(recordedAt.timeIntervalSince(entry.recordedAt ?? .distantPast)) < 1 ? nil : recordedAt
    }

    var body: some View {
        Form {
            TextField("Title", text: $title, prompt: Text(entry.displayTitle))
            TextField("Labels", text: $labels, prompt: Text("work, ideas"))
            if entry.recordedAt == nil {
                Toggle("Set the recording time", isOn: $setsDate)
            }
            if entry.recordedAt != nil || setsDate {
                DatePicker("Recorded", selection: $recordedAt)
            }
            if let error {
                Text(error).foregroundStyle(.red)
            }
        }
        .formStyle(.grouped)
        .frame(width: 440)
        .fixedSize(horizontal: false, vertical: true)
        .toolbar {
            ToolbarItem(placement: .cancellationAction) { Button("Cancel") { dismiss() } }
            ToolbarItem(placement: .confirmationAction) {
                Button("Save") {
                    isSaving = true
                    Task {
                        defer { isSaving = false }
                        do {
                            try await save(changedTitle, labels, changedDate)
                            dismiss()
                        } catch {
                            self.error = error.localizedDescription
                        }
                    }
                }
                .disabled(isSaving)
            }
        }
        .onAppear {
            title = entry.displayTitle == entry.title ? entry.title : ""
            labels = entry.labels.joined(separator: ", ")
            recordedAt = entry.recordedAt ?? entry.createdAt
        }
    }
}
