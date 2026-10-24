import SwiftUI
import VaultNative

struct RemotePeopleSettings: View {
    @ObservedObject var library: RemoteLibraryModel
    @ObservedObject private var auth = NativeVaultSession.shared
    @State private var people: [RemotePerson] = []
    @State private var loading = false
    @State private var busy = false
    @State private var error: String?
    @State private var adding = false
    @State private var name = ""
    @State private var renaming: RemotePerson?
    @State private var deleting: RemotePerson?
    @State private var removingVoice: RemotePerson?

    var body: some View {
        Form {
            Section {
                Text("Confirm a speaker in a transcript to build their voice profile. Uncertain matches stay unidentified.")
                    .foregroundStyle(.secondary)
            }
            Section("People") {
                if loading && people.isEmpty { ProgressView().controlSize(.small) }
                if people.isEmpty && !loading {
                    Text("People appear here when you identify a speaker or add a person.").foregroundStyle(.secondary)
                }
                ForEach(people) { person in
                    HStack {
                        VStack(alignment: .leading, spacing: 2) {
                            Text(person.name)
                            Text(person.samples == 0 ? "No voice profile" : "\(person.samples) voice sample\(person.samples == 1 ? "" : "s")")
                                .font(.caption).foregroundStyle(.secondary)
                        }
                        Spacer()
                        Menu("Edit") {
                            Button("Rename…") { name = person.name; renaming = person }
                            Button("Remove Voice Data…") { removingVoice = person }.disabled(person.samples == 0)
                            Divider()
                            Button("Delete Person…", role: .destructive) { deleting = person }
                        }.fixedSize().disabled(busy)
                    }
                }
                HStack {
                    Button("Add Person…", systemImage: "plus") { name = ""; adding = true }.disabled(busy)
                    Spacer()
                    Button("Reload", systemImage: "arrow.clockwise") { Task { await load() } }.disabled(loading || busy)
                }
            }
            if let error { Section { Label(error, systemImage: "exclamationmark.circle").foregroundStyle(.red).textSelection(.enabled) } }
        }
        .formStyle(.grouped)
        .task(id: auth.server) { people = []; await load() }
        .onChange(of: library.peopleRevision) { _, _ in Task { await load() } }
        .alert("New Person", isPresented: $adding) {
            TextField("Name", text: $name)
            Button("Cancel", role: .cancel) {}
            Button("Add") { Task { await mutate("/api/v1/people", method: "POST", body: ["name": name]) } }.disabled(name.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty)
        } message: { Text("Assign this person to a speaker in a transcript to enroll their voice.") }
        .alert("Rename Person", isPresented: Binding(get: { renaming != nil }, set: { if !$0 { renaming = nil } })) {
            TextField("Name", text: $name)
            Button("Cancel", role: .cancel) {}
            Button("Rename") {
                if let person = renaming { Task { await mutate("/api/v1/people/\(RemoteJSON.component(person.id))", method: "PATCH", body: ["name": name]) } }
            }.disabled(name.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty)
        } message: { Text("Their name is updated in the recording archive.") }
        .confirmationDialog("Remove this voice profile?", isPresented: Binding(get: { removingVoice != nil }, set: { if !$0 { removingVoice = nil } }), presenting: removingVoice) { person in
            Button("Remove Voice Data", role: .destructive) { Task { await mutate("/api/v1/people/\(RemoteJSON.component(person.id))/voice", method: "DELETE") } }
        } message: { _ in Text("Their name and transcript assignments are kept. Confirm their voice again in a transcript to create new samples.") }
        .confirmationDialog("Delete this person?", isPresented: Binding(get: { deleting != nil }, set: { if !$0 { deleting = nil } }), presenting: deleting) { person in
            Button("Delete Person and Voice Data", role: .destructive) { Task { await mutate("/api/v1/people/\(RemoteJSON.component(person.id))", method: "DELETE") } }
        } message: { _ in Text("Their voice profile is removed and their speakers become unidentified. Transcript text is preserved.") }
    }

    @MainActor private func load() async {
        let server = auth.server
        guard !server.isEmpty else { people = []; return }
        loading = true; defer { loading = false }
        do {
            let value = try await NativeVaultSession.shared.json("/api/v1/people")
            guard !Task.isCancelled, auth.server == server else { return }
            people = (value["people"] as? [[String: Any]] ?? []).compactMap(RemotePerson.init)
            error = nil
        } catch is CancellationError {} catch { if auth.server == server { self.error = error.localizedDescription } }
    }
    @MainActor private func mutate(_ path: String, method: String, body: [String: Any]? = nil) async {
        guard !busy else { return }
        busy = true; defer { busy = false }
        do {
            _ = try await NativeVaultSession.shared.json(path, method: method, body: body)
            library.peopleDidChange()
            await load(); await library.reload()
            error = nil
        } catch { self.error = error.localizedDescription }
    }
}
