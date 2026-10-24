import AppKit
import SwiftUI
import VaultNative

struct VaultSettingsView: View {
    @StateObject private var native = NativeVaultSession.shared
    @StateObject private var library = RemoteLibraryModel()
    @State private var tab = SettingsTab.connection
    private enum SettingsTab: Hashable { case connection, recorder, people, processing, diagnostics }

    var body: some View {
        TabView(selection: $tab) {
            Tab("Connection", systemImage: "network", value: SettingsTab.connection) { VaultConnectionPreferences() }
            Tab("Recorder", systemImage: "mic", value: SettingsTab.recorder) { VaultRecorderPreferences() }
            Tab("People", systemImage: "person.2", value: SettingsTab.people) {
                if native.isConnected {
                    RemotePeopleSettings(library: library).id(native.server)
                } else { connectFirst }
            }
            Tab("Processing", systemImage: "waveform", value: SettingsTab.processing) {
                if native.isConnected { VaultProcessingPreferences().id(native.server) } else { connectFirst }
            }
            Tab("Diagnostics", systemImage: "info.circle", value: SettingsTab.diagnostics) { VaultDiagnosticsPreferences() }
        }
        .frame(minWidth: 620, idealWidth: 660, minHeight: 450, idealHeight: 520)
    }
    private var connectFirst: some View {
        ContentUnavailableView {
            Label("Connect your vault", systemImage: "network")
        } description: {
            Text("Connect this app to manage your vault’s recordings, people, and processing settings.")
        } actions: {
            Button("Connection Settings") { tab = .connection }
        }
    }
}

private struct VaultConnectionPreferences: View {
    @StateObject private var native = NativeVaultSession.shared
    @StateObject private var queue = TransferQueue.shared
    @State private var server = ""
    @State private var uploadToken = ""
    @State private var busy = false
    @State private var message: String?
    @State private var error: String?
    private var activeTransfer: Bool { ["copying", "uploading", "verifying"].contains(queue.snapshot.state) }
    private var canSwitch: Bool { ["idle", "verified"].contains(queue.snapshot.state) }
    private var canConnect: Bool {
        !activeTransfer && !busy && !native.isSigningIn && !server.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
            && (canSwitch || server.trimmingCharacters(in: CharacterSet(charactersIn: "/")) == queue.snapshot.server.trimmingCharacters(in: CharacterSet(charactersIn: "/")))
    }
    var body: some View {
        Form {
            Section {
                TextField("Server address", text: $server, prompt: Text("https://vault.example.com"))
                    .disabled(!canSwitch || busy || native.isSigningIn)
                    .onSubmit { if canConnect { connect() } }
                LabeledContent("Status") {
                    if native.isSigningIn {
                        HStack(spacing: 8) { ProgressView().controlSize(.small); Text("Approve access in your browser") }
                    } else if busy {
                        ProgressView().controlSize(.small)
                    } else {
                        Label(native.isConnected ? "Connected" : "Not connected", systemImage: native.isConnected ? "checkmark.circle" : "minus.circle")
                            .foregroundStyle(.secondary)
                    }
                }
                HStack {
                    if native.isSigningIn {
                        Button("Cancel Connection") { native.cancelSignIn() }
                    } else {
                        Button("Connect…", action: connect).disabled(!canConnect)
                        Button("Use Saved Connection", action: restore).disabled(!canConnect)
                    }
                    Spacer()
                    Button("Test Connection", action: test)
                        .disabled(!native.isConnected || busy || native.isSigningIn)
                    if native.isConnected {
                        Button("Disconnect") {
                            run { await native.signOut(); message = "Disconnected" }
                        }
                        .disabled(!canSwitch || busy || native.isSigningIn)
                    }
                }
            } header: {
                Text("Vault")
            } footer: {
                Text("Connect opens your browser to sign in and approve this app. The connection is stored separately for each vault in macOS Keychain.")
            }
            if !canSwitch {
                Section {
                    Text(activeTransfer ? "Wait for the transfer to finish, or pause it, before reconnecting. Clear the queue before switching vaults." : "Resume or clear the current transfer before switching vaults.")
                        .font(.callout).foregroundStyle(.secondary)
                }
            }
            if let error { Section { SettingsFeedback(error, isError: true) } }
            if let message { Section { SettingsFeedback(message) } }
            Section {
                DisclosureGroup("Manual upload token") {
                    SecureField("Upload token", text: $uploadToken, prompt: Text("Paste an upload token"))
                    Button("Save Upload Connection") {
                        run {
                            try queue.configure(server: server.trimmingCharacters(in: .whitespacesAndNewlines), token: uploadToken.trimmingCharacters(in: .whitespacesAndNewlines), timezone: queue.snapshot.timezone)
                            try? await native.use(server: queue.snapshot.server)
                            VaultProfiles.shared.remember(queue.snapshot.server)
                            uploadToken = ""
                            message = "Upload connection saved"
                        }
                    }
                    .disabled(!canConnect || uploadToken.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty)
                }
            } footer: {
                Text("For older servers. An upload token transfers recordings; connect through the browser to browse and edit your library.")
            }
        }
        .formStyle(.grouped)
        .onAppear { server = native.server.isEmpty ? queue.snapshot.server : native.server }
        .onChange(of: native.server) { _, value in if !value.isEmpty { server = value } }
    }
    private func connect() {
        run {
            try await native.signIn(server: server)
            try await queue.configureNative(server: native.server, timezone: queue.snapshot.timezone)
            VaultProfiles.shared.remember(native.server)
            server = native.server
            message = "Connected"
        }
    }
    private func restore() {
        run {
            try await native.use(server: server)
            try await queue.configureNative(server: native.server, timezone: queue.snapshot.timezone)
            VaultProfiles.shared.remember(native.server)
            server = native.server
            message = "Connection restored"
        }
    }
    private func test() {
        run {
            _ = try await native.json("/api/v1/me")
            message = "Connection verified"
        }
    }
    private func run(_ operation: @escaping @MainActor () async throws -> Void) {
        guard !busy else { return }
        busy = true; error = nil; message = nil
        Task {
            defer { busy = false }
            do { try await operation() }
            catch is CancellationError { }
            catch { self.error = error.localizedDescription }
        }
    }
}

private struct VaultRecorderPreferences: View {
    @StateObject private var native = NativeVaultSession.shared
    @StateObject private var queue = TransferQueue.shared
    @State private var clock = "local"
    @State private var customZone = TimeZone.current.identifier
    @State private var busy = false
    @State private var message: String?
    @State private var error: String?
    private var timezone: String {
        switch clock { case "local": TimeZone.current.identifier; case "factory": "Etc/GMT-8"; default: customZone }
    }
    private var canSave: Bool { ["idle", "verified"].contains(queue.snapshot.state) && !busy }
    var body: some View {
        Form {
            Section {
                Picker("Recorder clock", selection: $clock) {
                    Text("Same as this Mac").tag("local")
                    Text("UTC+8 (DJI factory clock)").tag("factory")
                    Text("Another time zone").tag("custom")
                }
                .disabled(!canSave)
                if clock == "custom" {
                    Picker("Time zone", selection: $customZone) {
                        ForEach(TimeZone.knownTimeZoneIdentifiers, id: \.self) { Text($0.replacingOccurrences(of: "_", with: " ")).tag($0) }
                    }
                    .disabled(!canSave)
                }
                LabeledContent("Selected time zone", value: timezone)
                HStack {
                    Spacer()
                    if busy { ProgressView().controlSize(.small) }
                    Button("Save Recorder Clock") { save() }.disabled(!canSave)
                }
            } header: {
                Text("Recording times")
            } footer: {
                Text("DJI filenames contain the recorder’s clock time without a time zone. Choose the clock set on the recorder so sessions and transcripts show the correct times. This applies to new imports; original files stay unchanged.")
            }
            if !canSave && !busy {
                Section { Text("Finish or clear the current transfer before changing the recorder clock.").foregroundStyle(.secondary) }
            }
            if let error { Section { SettingsFeedback(error, isError: true) } }
            if let message { Section { SettingsFeedback(message) } }
        }
        .formStyle(.grouped)
        .onAppear {
            let zone = queue.snapshot.timezone
            if zone == TimeZone.current.identifier { clock = "local" }
            else if zone == "Etc/GMT-8" { clock = "factory" }
            else { clock = "custom"; customZone = zone }
        }
    }
    private func save() {
        busy = true; error = nil; message = nil
        Task {
            defer { busy = false }
            do {
                if native.isConnected, native.server == queue.snapshot.server {
                    try await queue.configureNative(server: native.server, timezone: timezone)
                } else if !queue.snapshot.server.isEmpty, let token = VaultCredentials.read() {
                    try queue.configure(server: queue.snapshot.server, token: token, timezone: timezone)
                } else {
                    try queue.configureTimezone(timezone)
                }
                message = "Recorder clock saved"
            } catch { self.error = error.localizedDescription }
        }
    }
}

private struct VaultProcessingPreferences: View {
    @StateObject private var native = NativeVaultSession.shared
    @State private var context = ""
    @State private var strong = 0.82
    @State private var margin = 0.10
    @State private var models: [String: String] = [:]
    @State private var loading = true
    @State private var loaded = false
    @State private var saving = false
    @State private var message: String?
    @State private var error: String?
    var body: some View {
        Group {
            if loading {
                ProgressView().frame(maxWidth: .infinity, maxHeight: .infinity)
            } else if !loaded {
                ContentUnavailableView {
                    Label("Couldn’t load settings", systemImage: "network.slash")
                } description: {
                    Text(error ?? "Check your connection and try again.")
                } actions: {
                    Button("Try Again") { Task { await load() } }
                }
            } else {
                Form {
                    if let error { Section { SettingsFeedback(error, isError: true) } }
                    Section {
                        ForEach([("transcription", "Transcription"), ("text", "Text processing"), ("embedding", "Search embeddings"), ("voice", "Voice recognition")], id: \.0) { key, label in
                            LabeledContent(label, value: models[key] ?? "Not configured")
                        }
                    } header: {
                        Text("Server models")
                    } footer: {
                        Text("Models and provider credentials are configured on the server.")
                    }
                    Section {
                        TextEditor(text: $context).font(.body).frame(minHeight: 100)
                        HStack {
                            Button("Clear Context") { context = "" }.disabled(context.isEmpty || saving)
                            Spacer()
                            Text("\(context.count) / 8,000").font(.caption).foregroundStyle(context.count > 8000 ? .red : .secondary)
                        }
                    } header: {
                        Text("Recording context")
                    } footer: {
                        Text("Helps interpret names and generate summaries after transcription. It does not change diarization or rewrite the spoken transcript.")
                    }
                    Section {
                        SettingsThreshold("Strong voice match", value: $strong)
                        SettingsThreshold("Lead over runner-up", value: $margin)
                        HStack {
                            Button("Reset to Defaults") { strong = 0.82; margin = 0.10 }.disabled(saving)
                            Spacer()
                            if saving { ProgressView().controlSize(.small) }
                            Button("Save Processing Settings") { save() }.disabled(saving || context.count > 8000)
                        }
                    } header: {
                        Text("Voice matching")
                    } footer: {
                        Text("A higher match threshold accepts fewer automatic identifications. The lead requires the best match to be clearly ahead of the next person. Changes apply to future processing.")
                    }
                    if let message { Section { SettingsFeedback(message) } }
                }
                .formStyle(.grouped)
                .disabled(saving)
            }
        }
        .task { await load() }
    }
    private func load() async {
        loading = true; error = nil
        defer { loading = false }
        do {
            let value = try await native.json("/api/v1/settings")
            context = value["recording_context"] as? String ?? ""
            let thresholds = value["voice_thresholds"] as? [String: Any] ?? [:]
            strong = (thresholds["strong"] as? NSNumber)?.doubleValue ?? 0.82
            margin = (thresholds["margin"] as? NSNumber)?.doubleValue ?? 0.10
            models = (value["models"] as? [String: Any] ?? [:]).mapValues { String(describing: $0) }
            loaded = true
        } catch { self.error = error.localizedDescription }
    }
    private func save() {
        saving = true; error = nil; message = nil
        Task {
            defer { saving = false }
            do {
                _ = try await native.json("/api/v1/settings", method: "PATCH", body: ["recording_context": context, "voice_thresholds": ["strong": strong, "margin": margin]])
                message = "Processing settings saved"
            } catch { self.error = error.localizedDescription }
        }
    }
}

private struct SettingsThreshold: View {
    let title: String
    @Binding var value: Double
    init(_ title: String, value: Binding<Double>) { self.title = title; self._value = value }
    var body: some View {
        LabeledContent(title) {
            HStack {
                Slider(value: $value, in: 0...1, step: 0.01).frame(minWidth: 140, idealWidth: 180, maxWidth: 240)
                Text(value, format: .number.precision(.fractionLength(2))).monospacedDigit().frame(width: 36, alignment: .trailing)
            }
        }
    }
}

private struct VaultDiagnosticsPreferences: View {
    @StateObject private var native = NativeVaultSession.shared
    @StateObject private var queue = TransferQueue.shared
    @State private var checking = false
    @State private var status = "Not checked"
    @State private var voiceConfigured: Bool?
    @State private var error: String?
    var body: some View {
        Form {
            Section {
                LabeledContent("Server", value: native.server.isEmpty ? (queue.snapshot.server.isEmpty ? "Not connected" : queue.snapshot.server) : native.server)
                LabeledContent("Connection", value: status)
                if let voiceConfigured { LabeledContent("Server voice model", value: voiceConfigured ? "Configured" : "Not configured") }
                HStack {
                    if checking { ProgressView().controlSize(.small) }
                    Button("Check Connection") { check() }.disabled(!native.isConnected || checking)
                    Spacer()
                    if let url = URL(string: native.server), !native.server.isEmpty {
                        Link("Open Vault", destination: url)
                    }
                }
            } header: { Text("Server") } footer: {
                Text("Checks the vault connection without making a provider request or sending recordings.")
            }
            Section {
                LabeledContent("Transfer state", value: queue.snapshot.state.capitalized)
                LabeledContent("Files in queue", value: String(queue.snapshot.files.count))
                Button("Show Transfer Cache in Finder") {
                    let folder = FileManager.default.urls(for: .applicationSupportDirectory, in: .userDomainMask)[0].appendingPathComponent("TranscriptVault/Queue", isDirectory: true)
                    NSWorkspace.shared.open(folder)
                }
            } header: { Text("This Mac") } footer: {
                Text("Recordings and transcripts are stored on your server. Local transfer copies remain until you clear the upload queue. Originals on the recorder are never changed.")
            }
            if let error { Section { SettingsFeedback(error, isError: true) } }
        }
        .formStyle(.grouped)
        .onChange(of: native.server) { _, _ in status = "Not checked"; voiceConfigured = nil; error = nil }
    }
    private func check() {
        checking = true; error = nil
        Task {
            defer { checking = false }
            do {
                let value = try await native.json("/api/v1/me")
                status = "Connected"
                voiceConfigured = value["voice_configured"] as? Bool
            } catch { status = "Connection failed"; self.error = error.localizedDescription }
        }
    }
}

private struct SettingsFeedback: View {
    let message: String
    var isError = false
    init(_ message: String, isError: Bool = false) { self.message = message; self.isError = isError }
    var body: some View {
        Label(message, systemImage: isError ? "exclamationmark.circle" : "checkmark.circle")
            .font(.callout)
            .foregroundStyle(isError ? .red : .secondary)
            .textSelection(.enabled)
            .fixedSize(horizontal: false, vertical: true)
    }
}
