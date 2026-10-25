import Foundation
import Observation
import VaultNative

/// The connected vault's recordings, Trash and people, shared by every window and Settings.
@MainActor
@Observable
final class Library {
    private(set) var entries: [Entry] = []
    private(set) var trash: [Entry] = []
    private(set) var people: [Person] = []
    private(set) var hasMore = false
    private(set) var hasLoaded = false
    private(set) var isLoading = false
    private(set) var error: String?
    private var nextCursor: Int?
    /// Bumped when the vault changes, so a late response from the previous vault is dropped.
    private var generation = 0

    var isProcessing: Bool { entries.contains(where: \.isProcessing) }
    func entry(_ id: String) -> Entry? { entries.first { $0.id == id } ?? trash.first { $0.id == id } }

    func reset() {
        generation += 1
        entries = []; trash = []; people = []
        hasMore = false; hasLoaded = false; isLoading = false
        nextCursor = nil; error = nil
    }

    /// Reloads the newest page and keeps older pages that were already loaded.
    func refresh() async {
        guard NativeVaultSession.shared.isConnected else { return }
        let generation = generation
        isLoading = true
        defer { if generation == self.generation { isLoading = false } }
        do {
            let page = try await VaultAPI.get("/api/v1/entries?limit=100", as: Page<Entry>.self)
            guard generation == self.generation else { return }
            var merged = page.items
            if page.nextCursor != nil, let last = page.items.last, let boundary = entries.firstIndex(where: { $0.id == last.id }) {
                let fresh = Set(page.items.map(\.id))
                merged += entries[(boundary + 1)...].filter { !fresh.contains($0.id) }
            }
            entries = merged
            nextCursor = page.nextCursor == nil ? nil : merged.count
            hasMore = nextCursor != nil
            hasLoaded = true
            error = nil
        } catch {
            fail(error, generation)
        }
    }

    func loadMore() async {
        guard let cursor = nextCursor, !isLoading else { return }
        let generation = generation
        isLoading = true
        defer { if generation == self.generation { isLoading = false } }
        do {
            let page = try await VaultAPI.get("/api/v1/entries?limit=100&cursor=\(cursor)", as: Page<Entry>.self)
            guard generation == self.generation else { return }
            let known = Set(entries.map(\.id))
            entries += page.items.filter { !known.contains($0.id) }
            nextCursor = page.nextCursor
            hasMore = nextCursor != nil
        } catch {
            fail(error, generation)
        }
    }

    func refreshTrash() async {
        let generation = generation
        do {
            // ponytail: the 100 most recent trashed recordings; page Trash if anyone keeps more than that.
            let page = try await VaultAPI.get("/api/v1/entries?trash=true&limit=100", as: Page<Entry>.self)
            if generation == self.generation { trash = page.items }
        } catch {
            fail(error, generation)
        }
    }

    func refreshPeople() async {
        let generation = generation
        do {
            let page = try await VaultAPI.get("/api/v1/people", as: Page<Person>.self)
            if generation == self.generation { people = page.items }
        } catch {
            fail(error, generation)
        }
    }

    private func fail(_ error: Error, _ generation: Int) {
        guard generation == self.generation, !(error is CancellationError) else { return }
        self.error = error.localizedDescription
    }

    // MARK: Recordings

    enum Action {
        case trash, restore, deletePermanently, pause, resume, reanalyzeSpeakers
    }

    func perform(_ action: Action, on id: String) async throws {
        switch action {
        case .trash: try await VaultAPI.send(VaultAPI.entryPath(id, "/trash"))
        case .restore: try await VaultAPI.send(VaultAPI.entryPath(id, "/restore"))
        case .deletePermanently: try await VaultAPI.send(VaultAPI.entryPath(id), method: "DELETE")
        case .pause: try await VaultAPI.send(VaultAPI.entryPath(id, "/pause"))
        case .resume: try await VaultAPI.send(VaultAPI.entryPath(id, "/retry"))
        case .reanalyzeSpeakers: try await VaultAPI.send(VaultAPI.entryPath(id, "/reanalyze"))
        }
        if action == .trash || action == .deletePermanently { entries.removeAll { $0.id == id } }
        if action == .restore || action == .deletePermanently { trash.removeAll { $0.id == id } }
        if [.trash, .restore, .deletePermanently].contains(action) { await refreshTrash() }
        await refresh()
    }

    /// `title` and `recordedAt` are sent only when given: a sent title is kept as a manual one and
    /// is never replaced by a generated title again.
    func update(_ id: String, title: String?, labels: [String], recordedAt: Date?) async throws {
        var body: [String: Any] = ["labels": labels]
        if let title { body["title"] = title }
        if let recordedAt { body["recorded_at"] = VaultAPI.timestamp(recordedAt) }
        try await VaultAPI.send(VaultAPI.entryPath(id), method: "PATCH", body: body)
        await refresh()
    }

    // MARK: Speakers and people

    /// Assigns a person, or with `person == nil` either keeps the speaker unidentified (`locked`) or
    /// hands it back to automatic recognition.
    func assign(_ person: Person?, to speaker: String, in entry: String, locked: Bool = true) async throws {
        var body: [String: Any] = ["person_id": person?.id ?? NSNull()]
        if person == nil { body["locked"] = locked }
        try await VaultAPI.send(VaultAPI.entryPath(entry, "/speakers/\(VaultAPI.component(speaker))"), method: "PATCH", body: body)
        await refreshPeople()
    }

    func addPerson(named name: String) async throws -> Person {
        let data = try await VaultAPI.send("/api/v1/people", body: ["name": name.trimmingCharacters(in: .whitespacesAndNewlines)])
        let person = try VaultAPI.decode(Person.self, from: data)
        await refreshPeople()
        return person
    }

    func rename(_ person: Person, to name: String) async throws {
        try await VaultAPI.send("/api/v1/people/\(VaultAPI.component(person.id))", method: "PATCH",
                                body: ["name": name.trimmingCharacters(in: .whitespacesAndNewlines)])
        await refreshPeople()
    }

    func delete(_ person: Person) async throws {
        try await VaultAPI.send("/api/v1/people/\(VaultAPI.component(person.id))", method: "DELETE")
        await refreshPeople()
    }

    func removeVoiceData(_ person: Person) async throws {
        try await VaultAPI.send("/api/v1/people/\(VaultAPI.component(person.id))/voice", method: "DELETE")
        await refreshPeople()
    }
}
