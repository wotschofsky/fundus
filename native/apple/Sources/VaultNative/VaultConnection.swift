import Foundation
import Security

public enum VaultCredentials {
    // Preserve the original fallback service so saved upload tokens remain available to Fundus.
    private static var service: String { (Bundle.main.bundleIdentifier ?? "dev.wsky.transcript-vault") + ".upload" }
    public static func save(_ token: String) throws {
        let query: [String: Any] = [kSecClass as String: kSecClassGenericPassword, kSecAttrService as String: service, kSecAttrAccount as String: "token"]
        SecItemDelete(query as CFDictionary)
        let attributes = query.merging([kSecValueData as String: Data(token.utf8), kSecAttrAccessible as String: kSecAttrAccessibleAfterFirstUnlockThisDeviceOnly]) { _, value in value }
        let status = SecItemAdd(attributes as CFDictionary, nil)
        guard status == errSecSuccess else { throw NSError(domain: NSOSStatusErrorDomain, code: Int(status)) }
    }
    public static func read() -> String? {
        let query: [String: Any] = [kSecClass as String: kSecClassGenericPassword, kSecAttrService as String: service, kSecAttrAccount as String: "token", kSecReturnData as String: true]
        var item: CFTypeRef?
        guard SecItemCopyMatching(query as CFDictionary, &item) == errSecSuccess, let data = item as? Data else { return nil }
        return String(data: data, encoding: .utf8)
    }
}
public struct VaultConnection: Sendable {
    public let base: URL
    public let token: String
    public let nativeOwner: Bool
    public init(server: String, token: String, nativeOwner: Bool = false) throws {
        guard !server.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty else {
            throw Self.invalidConnection("Enter your vault’s server address, for example https://vault.example.com.")
        }
        guard let url = URL(string: server), let host = url.host, !host.isEmpty,
              ["https", "http"].contains(url.scheme) else {
            throw Self.invalidConnection("Enter a complete server address starting with https://, for example https://vault.example.com.")
        }
        guard url.user == nil, url.password == nil, url.query == nil, url.fragment == nil,
              url.path.isEmpty || url.path == "/" else {
            throw Self.invalidConnection("Use only your vault’s server address, without a page path, query, or login details.")
        }
        guard url.scheme == "https" || ["localhost", "127.0.0.1", "::1"].contains(host) else { throw NSError(domain: "Vault", code: 1, userInfo: [NSLocalizedDescriptionKey: "Use your external HTTPS server address. HTTP is allowed only for local development."]) }
        guard !token.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty else {
            throw Self.invalidConnection("Enter an upload token. You can create one in your vault’s Settings.")
        }
        self.base = url; self.token = token; self.nativeOwner = nativeOwner
    }
    private static func invalidConnection(_ message: String) -> NSError {
        NSError(domain: "VaultConnection", code: 1, userInfo: [NSLocalizedDescriptionKey: message])
    }
    public func request(_ path: String, method: String = "GET", json: [String: Any]? = nil) throws -> URLRequest {
        guard var url = URL(string: path, relativeTo: base)?.absoluteURL, url.host == base.host, url.scheme == base.scheme, url.port == base.port else { throw URLError(.badURL) }
        if nativeOwner, url.path == "/api/v1" || url.path.hasPrefix("/api/v1/") {
            var components = URLComponents(url: url, resolvingAgainstBaseURL: false)!
            components.path = "/api/native" + url.path.dropFirst(4)
            guard let nativeURL = components.url else { throw URLError(.badURL) }
            url = nativeURL
        }
        var request = URLRequest(url: url); request.httpMethod = method
        request.setValue("Bearer \(token)", forHTTPHeaderField: "Authorization")
        if let json { request.httpBody = try JSONSerialization.data(withJSONObject: json); request.setValue("application/json", forHTTPHeaderField: "Content-Type") }
        return request
    }
    public func json(_ path: String, method: String = "GET", body: [String: Any]? = nil) async throws -> [String: Any] {
        let (data, response) = try await URLSession.shared.data(for: request(path, method: method, json: body))
        try Self.check(response, data)
        return (try? JSONSerialization.jsonObject(with: data)) as? [String: Any] ?? [:]
    }
    public static func check(_ response: URLResponse, _ data: Data = Data()) throws {
        guard let http = response as? HTTPURLResponse, (200..<300).contains(http.statusCode) else {
            let message = ((try? JSONSerialization.jsonObject(with: data)) as? [String: Any])?["error"] as? String ?? "Transfer failed. Check the connection and retry."
            throw NSError(domain: "Vault", code: (response as? HTTPURLResponse)?.statusCode ?? 0, userInfo: [NSLocalizedDescriptionKey: message])
        }
    }
}
