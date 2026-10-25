import Foundation
import VaultNative

/// Typed calls to the vault's owner API through the signed-in native session.
@MainActor
enum VaultAPI {
    static func get<T: Decodable>(_ path: String, as type: T.Type = T.self) async throws -> T {
        try decode(T.self, from: await NativeVaultSession.shared.data(path))
    }

    @discardableResult
    static func send(_ path: String, method: String = "POST", body: [String: Any]? = nil) async throws -> Data {
        try await NativeVaultSession.shared.data(path, method: method, body: body)
    }

    static func decode<T: Decodable>(_ type: T.Type, from data: Data) throws -> T {
        do { return try decoder.decode(type, from: data) } catch {
            Log.vault.error("Could not read \(String(describing: type), privacy: .public): \(error, privacy: .public)")
            throw VaultAPIError.unreadable
        }
    }

    nonisolated static func entryPath(_ id: String, _ suffix: String = "") -> String {
        "/api/v1/entries/\(component(id))\(suffix)"
    }

    nonisolated static func component(_ value: String) -> String {
        value.addingPercentEncoding(withAllowedCharacters: .alphanumerics.union(CharacterSet(charactersIn: "-._~"))) ?? value
    }

    nonisolated static func timestamp(_ date: Date) -> String {
        date.formatted(.iso8601)
    }

    private static let decoder: JSONDecoder = {
        let decoder = JSONDecoder()
        decoder.keyDecodingStrategy = .convertFromSnakeCase
        // `.iso8601` reads fractional seconds and UTC offsets alike.
        decoder.dateDecodingStrategy = .custom { decoder in
            let text = try decoder.singleValueContainer().decode(String.self)
            if let date = try? Date(text, strategy: .iso8601) { return date }
            throw DecodingError.dataCorrupted(.init(codingPath: decoder.codingPath, debugDescription: "Not an ISO 8601 date: \(text)"))
        }
        return decoder
    }()
}

enum VaultAPIError: LocalizedError {
    case unreadable
    var errorDescription: String? { "The vault sent a response this app could not read. Update the server or the app." }
}
