import Foundation
import Observation

struct VaultProfile: Codable, Identifiable, Equatable {
    var server: String
    var id: String { server }
    var name: String { URL(string: server)?.host ?? server }
}
@MainActor @Observable final class VaultProfiles {
    static let shared = VaultProfiles()
    private(set) var profiles: [VaultProfile]
    init() {
        profiles = UserDefaults.standard.data(forKey: "remote-vault-profiles").flatMap { try? JSONDecoder().decode([VaultProfile].self, from: $0) } ?? []
    }
    func remember(_ server: String) {
        guard !server.isEmpty else { return }
        profiles.removeAll { $0.server == server }
        profiles.insert(VaultProfile(server: server), at: 0)
        save()
    }
    func remove(_ server: String) { profiles.removeAll { $0.server == server }; save() }
    private func save() { UserDefaults.standard.set(try? JSONEncoder().encode(profiles), forKey: "remote-vault-profiles") }
}
