import AppKit
import SwiftUI
import VaultNative

struct SettingsView: View {
    @Environment(AppModel.self) private var app
    @State private var processing = ProcessingSettingsModel()

    var body: some View {
        TabView {
            Tab("Vault", systemImage: "books.vertical") { VaultSettings() }
            Tab("Transcription", systemImage: "waveform") { TranscriptionSettings(processing: processing) }
            Tab("Speakers", systemImage: "person.2") { SpeakerSettings() }
            Tab("Advanced", systemImage: "gearshape.2") { AdvancedSettings(processing: processing) }
        }
        .frame(width: 580, height: 460)
        // Each vault has its own processing settings.
        .task(id: "\(app.server)|\(app.isConnected)") {
            processing.reset()
            if app.isConnected { await processing.load() }
        }
    }
}

// MARK: - Vault

private struct VaultSettings: View {
    @Environment(AppModel.self) private var app
    @State private var address = ""
    @State private var uploadToken = ""
    @State private var isWorking = false
    @State private var message: String?
    @State private var error: String?

    private var canChangeVault: Bool { app.canUpload && !isWorking && !app.isSigningIn }

    var body: some View {
        Form {
            Section {
                TextField("Server address", text: $address, prompt: Text("https://vault.example.com"))
                    .onSubmit(connect)
                    .disabled(!canChangeVault)
                LabeledContent("Status") { status }
                HStack {
                    if app.isSigningIn {
                        Button("Cancel") { app.native.cancelSignIn() }
                    } else {
                        Button(app.isConnected && normalized(address) == app.server ? "Reconnect…" : "Connect…", action: connect)
                            .disabled(!canChangeVault || address.trimmingCharacters(in: .whitespaces).isEmpty)
                    }
                    Spacer()
                    if app.isConnected {
                        Button("Disconnect") { run { await app.native.signOut(); message = "Disconnected" } }
                            .disabled(!canChangeVault)
                    }
                }
            } header: {
                Text("Vault")
            } footer: {
                Footnote(app.canUpload ? "Connect opens your browser to sign in and approve this Mac. The connection is stored in the Keychain, separately for each vault."
                         : "Wait for the current upload to finish, or clear it, before changing vaults.")
            }
            if app.profiles.profiles.count > 1 {
                Section("Saved Vaults") {
                    ForEach(app.profiles.profiles) { profile in
                        HStack {
                            Text(profile.name)
                            if profile.server == app.server {
                                Image(systemName: "checkmark").foregroundStyle(.secondary)
                            }
                            Spacer()
                            if profile.server != app.server {
                                Button("Open") { Task { await app.switchVault(to: profile.server) } }
                                    .disabled(!canChangeVault)
                                Button("Forget") { app.profiles.remove(profile.server) }
                            }
                        }
                    }
                }
            }
            if let feedback = error ?? message {
                Section { Feedback(feedback, isError: error != nil) }
            }
            Section {
                DisclosureGroup("Upload with a token") {
                    SecureField("Upload token", text: $uploadToken, prompt: Text("Paste an upload token"))
                    HStack {
                        Spacer()
                        Button("Save Upload Connection", action: saveToken)
                            .disabled(!canChangeVault || uploadToken.trimmingCharacters(in: .whitespaces).isEmpty)
                    }
                }
            } footer: {
                Footnote("For older servers without browser sign-in. A token can upload recordings but cannot open the library.")
            }
        }
        .formStyle(.grouped)
        .onAppear { address = app.server.isEmpty ? app.transfer.server : app.server }
        .onChange(of: app.server) { _, server in if !server.isEmpty { address = server } }
    }

    @ViewBuilder private var status: some View {
        if app.isSigningIn {
            HStack(spacing: 6) {
                ProgressView().controlSize(.small)
                Text("Approve access in your browser")
            }
        } else if isWorking {
            ProgressView().controlSize(.small)
        } else if app.isConnected {
            Label("Connected to \(app.vaultName)", systemImage: "checkmark.circle.fill").foregroundStyle(.green)
        } else {
            Label("Not connected", systemImage: "minus.circle").foregroundStyle(.secondary)
        }
    }

    private func normalized(_ value: String) -> String {
        value.trimmingCharacters(in: .whitespaces).trimmingCharacters(in: CharacterSet(charactersIn: "/"))
    }

    private func connect() {
        guard canChangeVault, !address.trimmingCharacters(in: .whitespaces).isEmpty else { return }
        run {
            try await app.native.signIn(server: address)
            try await app.queue.configureNative(server: app.native.server, timezone: app.transfer.timezone)
            app.profiles.remember(app.native.server)
            message = "Connected"
        }
    }

    private func saveToken() {
        run {
            let server = address.trimmingCharacters(in: .whitespacesAndNewlines)
            try app.queue.configure(server: server, token: uploadToken.trimmingCharacters(in: .whitespacesAndNewlines), timezone: app.transfer.timezone)
            app.profiles.remember(app.transfer.server)
            uploadToken = ""
            message = "Upload connection saved"
        }
    }

    private func run(_ operation: @escaping @MainActor () async throws -> Void) {
        isWorking = true
        error = nil
        message = nil
        Task {
            defer { isWorking = false }
            do { try await operation() } catch is CancellationError {} catch { self.error = error.localizedDescription }
        }
    }
}

// MARK: - Transcription

/// The vault's processing settings, loaded once per Settings window and saved shortly after each edit.
@MainActor
@Observable
final class ProcessingSettingsModel {
    var settings: ProcessingSettings?
    private(set) var error: String?
    private(set) var isSaved = true
    @ObservationIgnored private var saving: Task<Void, Never>?
    /// The vault these settings were read from; edits are only ever saved back to it.
    @ObservationIgnored private var server = ""

    /// Another vault: drop its settings and any edit not yet saved.
    func reset() {
        saving?.cancel()
        settings = nil
        isSaved = true
        error = nil
    }

    func load() async {
        do {
            let server = NativeVaultSession.shared.server
            settings = try await VaultAPI.get("/api/v1/settings", as: ProcessingSettings.self)
            self.server = server
            error = nil
        } catch {
            self.error = error.localizedDescription
        }
    }

    func changed() {
        guard let settings else { return }
        isSaved = false
        saving?.cancel()
        saving = Task {
            try? await Task.sleep(for: .seconds(0.8))
            guard !Task.isCancelled, NativeVaultSession.shared.server == server else { return }
            do {
                try await VaultAPI.send("/api/v1/settings", method: "PATCH", body: [
                    "recording_context": settings.recordingContext,
                    "voice_thresholds": ["strong": settings.voiceThresholds.strong, "margin": settings.voiceThresholds.margin],
                ])
                isSaved = true
                error = nil
            } catch {
                self.error = "Not saved: \(error.localizedDescription)"
            }
        }
    }
}

private struct TranscriptionSettings: View {
    let processing: ProcessingSettingsModel
    @Environment(AppModel.self) private var app
    @State private var clock = "local"
    @State private var zone = TimeZone.current.identifier

    private static let factoryZone = "Etc/GMT-8"
    private var selectedZone: String {
        switch clock {
        case "local": TimeZone.current.identifier
        case "factory": Self.factoryZone
        default: zone
        }
    }

    var body: some View {
        @Bindable var processing = processing
        Form {
            Section {
                Picker("Recorder clock", selection: $clock) {
                    Text("Same as this Mac").tag("local")
                    Text("UTC+8 (DJI factory setting)").tag("factory")
                    Text("Another time zone").tag("other")
                }
                if clock == "other" {
                    Picker("Time zone", selection: $zone) {
                        ForEach(TimeZone.knownTimeZoneIdentifiers, id: \.self) { Text($0.replacingOccurrences(of: "_", with: " ")).tag($0) }
                    }
                }
            } header: {
                Text("Recorder")
            } footer: {
                Footnote("The recorder stamps files with its own clock and stores no time zone. Choose the zone its clock runs in so recording times are right. Applies to new imports.")
            }
            .disabled(!app.canUpload)
            .onChange(of: selectedZone) { _, zone in
                guard zone != app.transfer.timezone else { return }
                Task {
                    do { try await saveRecorderClock(zone) } catch { app.report("Could not change the recorder clock", error) }
                }
            }

            if processing.settings != nil {
                Section {
                    TextEditor(text: Binding(get: { processing.settings?.recordingContext ?? "" },
                                             set: { processing.settings?.recordingContext = $0; processing.changed() }))
                        .font(.body)
                        .frame(minHeight: 90)
                    HStack {
                        SaveState(processing: processing)
                        Spacer()
                        let count = processing.settings?.recordingContext.count ?? 0
                        Text("\(count) / 8,000")
                            .font(.caption)
                            .monospacedDigit()
                            .foregroundStyle(count > 8000 ? .red : .secondary)
                    }
                } header: {
                    Text("Context")
                } footer: {
                    Footnote("Describes what your recordings usually contain: languages, places, names, vocabulary. It helps with names and the summary; the speech itself is never rewritten.")
                }
                if let models = processing.settings?.models, !models.isEmpty {
                    Section("Models on the Vault") {
                        ForEach([("transcription", "Transcription"), ("text", "Titles and summaries"), ("embedding", "Search"), ("voice", "Voice recognition")], id: \.0) { key, title in
                            LabeledContent(title, value: models[key] ?? "Not configured")
                        }
                    }
                }
            } else {
                ConnectionPlaceholder(processing: processing)
            }
        }
        .formStyle(.grouped)
        .onAppear {
            let current = app.transfer.timezone
            if current == TimeZone.current.identifier { clock = "local" }
            else if current == Self.factoryZone { clock = "factory" }
            else { clock = "other"; zone = current }
        }
    }

    private func saveRecorderClock(_ zone: String) async throws {
        if app.isConnected, app.native.server == app.transfer.server {
            try await app.queue.configureNative(server: app.native.server, timezone: zone)
        } else if !app.transfer.server.isEmpty, let token = VaultCredentials.read() {
            try app.queue.configure(server: app.transfer.server, token: token, timezone: zone)
        } else {
            try app.queue.configureTimezone(zone)
        }
    }
}

// MARK: - Speakers

private struct SpeakerSettings: View {
    @Environment(AppModel.self) private var app
    @State private var renaming: Person?
    @State private var deleting: Person?
    @State private var adding = false
    @State private var name = ""
    @State private var error: String?

    var body: some View {
        Form {
            Section {
                if app.library.people.isEmpty {
                    Text(app.isConnected ? "People appear here once you name a speaker in a transcript." : "Connect a vault to see its people.")
                        .foregroundStyle(.secondary)
                }
                ForEach(app.library.people) { person in
                    HStack {
                        VStack(alignment: .leading, spacing: 1) {
                            Text(person.name)
                            Text(person.samples == 0 ? "No voice profile" : "\(person.samples) voice sample\(person.samples == 1 ? "" : "s")")
                                .font(.caption)
                                .foregroundStyle(.secondary)
                        }
                        Spacer()
                        Menu("Edit") {
                            Button("Rename…") { name = person.name; renaming = person }
                            Button("Remove Voice Data") { act { try await app.library.removeVoiceData(person) } }
                                .disabled(person.samples == 0)
                            Divider()
                            Button("Delete Person…", role: .destructive) { deleting = person }
                        }
                        .fixedSize()
                    }
                }
            } header: {
                HStack {
                    Text("People in \(app.vaultName)")
                    Spacer()
                    Button("Add Person…", systemImage: "plus") { name = ""; adding = true }
                        .labelStyle(.iconOnly)
                        .buttonStyle(.borderless)
                        .disabled(!app.isConnected)
                }
            } footer: {
                Footnote("Naming a speaker in a transcript adds their voice to their profile, on the vault. Uncertain voices stay unnamed.")
            }
            if let error {
                Section { Feedback(error, isError: true) }
            }
        }
        .formStyle(.grouped)
        .task { await app.library.refreshPeople() }
        .alert("New Person", isPresented: $adding) {
            TextField("Name", text: $name)
            Button("Cancel", role: .cancel) {}
            Button("Add") { act { _ = try await app.library.addPerson(named: name) } }
        } message: {
            Text("Then choose them for a speaker in a transcript.")
        }
        .alert("Rename Person", isPresented: .init(get: { renaming != nil }, set: { if !$0 { renaming = nil } })) {
            TextField("Name", text: $name)
            Button("Cancel", role: .cancel) {}
            Button("Rename") { if let renaming { act { try await app.library.rename(renaming, to: name) } } }
        } message: {
            Text("Transcripts that mention this person show the new name.")
        }
        .confirmationDialog("Delete \(deleting?.name ?? "this person")?",
                            isPresented: .init(get: { deleting != nil }, set: { if !$0 { deleting = nil } }), presenting: deleting) { person in
            Button("Delete Person and Voice Data", role: .destructive) { act { try await app.library.delete(person) } }
        } message: { _ in
            Text("Their voice profile is removed and their speakers become unidentified again in all transcripts. Transcript text is kept.")
        }
    }

    private func act(_ operation: @escaping @MainActor () async throws -> Void) {
        Task {
            do { try await operation(); error = nil } catch { self.error = error.localizedDescription }
        }
    }
}

// MARK: - Advanced

private struct AdvancedSettings: View {
    let processing: ProcessingSettingsModel
    @Environment(AppModel.self) private var app
    @State private var check: String?

    var body: some View {
        @Bindable var processing = processing
        Form {
            if processing.settings != nil {
                Section {
                    ThresholdRow(title: "Strong match", value: threshold(\.strong))
                    ThresholdRow(title: "Lead over runner-up", value: threshold(\.margin), range: 0...0.4)
                    HStack {
                        SaveState(processing: processing)
                        Spacer()
                        Button("Reset to Defaults") {
                            processing.settings?.voiceThresholds = .default
                            processing.changed()
                        }
                        .disabled(processing.settings?.voiceThresholds == .default)
                    }
                } header: {
                    Text("Voice Matching")
                } footer: {
                    Footnote("Similarity a speaker needs to a saved voice profile, and how far ahead of the next person, to be named automatically. Higher values mean fewer but safer identifications.")
                }
            } else {
                ConnectionPlaceholder(processing: processing)
            }
            Section("Diagnostics") {
                LabeledContent("Vault", value: app.server.isEmpty ? "Not connected" : app.server)
                LabeledContent("Connection") {
                    HStack {
                        if let check { Text(check).foregroundStyle(.secondary) }
                        Button("Check") { checkConnection() }.disabled(!app.isConnected)
                    }
                }
                LabeledContent("Upload cache") {
                    Button("Show in Finder") {
                        // Match the shared queue's legacy location so existing imports stay visible.
                        NSWorkspace.shared.open(FileManager.default.urls(for: .applicationSupportDirectory, in: .userDomainMask)[0]
                            .appendingPathComponent("TranscriptVault/Queue", isDirectory: true))
                    }
                }
                LabeledContent("Logs", value: "Console.app, subsystem Fundus")
            }
        }
        .formStyle(.grouped)
    }

    private func threshold(_ key: WritableKeyPath<ProcessingSettings.Thresholds, Double>) -> Binding<Double> {
        Binding(get: { processing.settings?.voiceThresholds[keyPath: key] ?? 0 },
                set: { processing.settings?.voiceThresholds[keyPath: key] = $0; processing.changed() })
    }

    private func checkConnection() {
        check = "Checking…"
        Task {
            do {
                _ = try await VaultAPI.send("/api/v1/me", method: "GET")
                check = "Connected"
            } catch {
                check = error.localizedDescription
            }
        }
    }
}

// MARK: - Shared

private struct ThresholdRow: View {
    let title: String
    @Binding var value: Double
    var range: ClosedRange<Double> = 0.2...0.95

    var body: some View {
        LabeledContent(title) {
            HStack {
                Slider(value: $value, in: range, step: 0.01)
                    .frame(width: 180)
                Text(value, format: .number.precision(.fractionLength(2)))
                    .monospacedDigit()
                    .frame(width: 36, alignment: .trailing)
            }
        }
    }
}

private struct SaveState: View {
    let processing: ProcessingSettingsModel

    var body: some View {
        if let error = processing.error {
            Text(error).font(.caption).foregroundStyle(.red)
        } else if !processing.isSaved {
            Text("Saving…").font(.caption).foregroundStyle(.secondary)
        }
    }
}

private struct ConnectionPlaceholder: View {
    let processing: ProcessingSettingsModel
    @Environment(AppModel.self) private var app

    var body: some View {
        Section {
            if !app.isConnected {
                Text("Connect a vault to change its processing settings.").foregroundStyle(.secondary)
            } else if let error = processing.error {
                HStack {
                    Feedback(error, isError: true)
                    Spacer()
                    Button("Try Again") { Task { await processing.load() } }
                }
            } else {
                ProgressView().controlSize(.small)
            }
        }
    }
}

/// Explanatory text under a settings section.
private struct Footnote: View {
    let text: String
    init(_ text: String) { self.text = text }

    var body: some View {
        Text(text)
            .font(.callout)
            .foregroundStyle(.secondary)
            .frame(maxWidth: .infinity, alignment: .leading)
    }
}

private struct Feedback: View {
    let message: String
    let isError: Bool
    init(_ message: String, isError: Bool = false) { self.message = message; self.isError = isError }

    var body: some View {
        Label(message, systemImage: isError ? "exclamationmark.circle" : "checkmark.circle")
            .foregroundStyle(isError ? .red : .secondary)
            .textSelection(.enabled)
            .fixedSize(horizontal: false, vertical: true)
    }
}
