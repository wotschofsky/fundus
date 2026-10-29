#if os(macOS)
import AppKit
import Combine
import CryptoKit
import Foundation
@preconcurrency import Network
import Security

struct NativeOAuthCredentials: Codable, Sendable {
    let server: String
    let clientID: String
    let accessToken: String
    let refreshToken: String
    let expiresAt: Date
}

@MainActor protocol NativeCredentialStorage {
    func read(server: String) throws -> NativeOAuthCredentials?
    func save(_ credentials: NativeOAuthCredentials) throws
    func remove(server: String) throws
}

@MainActor private struct NativeKeychainStorage: NativeCredentialStorage {
    // Preserve the original fallback service so existing connections survive the Fundus rename.
    private var service: String { (Bundle.main.bundleIdentifier ?? "dev.wsky.transcript-vault") + ".native-oauth" }
    private func query(_ server: String) -> [String: Any] {
        [kSecClass as String: kSecClassGenericPassword, kSecAttrService as String: service, kSecAttrAccount as String: server]
    }
    func read(server: String) throws -> NativeOAuthCredentials? {
        var query = query(server)
        query[kSecReturnData as String] = true
        var result: CFTypeRef?
        let status = SecItemCopyMatching(query as CFDictionary, &result)
        if status == errSecItemNotFound { return nil }
        guard status == errSecSuccess, let data = result as? Data else { throw NativeOAuthFailure.keychain }
        return try JSONDecoder().decode(NativeOAuthCredentials.self, from: data)
    }
    func save(_ credentials: NativeOAuthCredentials) throws {
        let query = query(credentials.server)
        let data = try JSONEncoder().encode(credentials)
        let status = SecItemUpdate(query as CFDictionary, [kSecValueData as String: data] as CFDictionary)
        if status == errSecSuccess { return }
        guard status == errSecItemNotFound else { throw NativeOAuthFailure.keychain }
        var attributes = query
        attributes[kSecValueData as String] = data
        attributes[kSecAttrAccessible as String] = kSecAttrAccessibleAfterFirstUnlockThisDeviceOnly
        guard SecItemAdd(attributes as CFDictionary, nil) == errSecSuccess else { throw NativeOAuthFailure.keychain }
    }
    func remove(server: String) throws {
        let status = SecItemDelete(query(server) as CFDictionary)
        guard [errSecSuccess, errSecItemNotFound].contains(status) else { throw NativeOAuthFailure.keychain }
    }
}

private enum NativeOAuthFailure: LocalizedError {
    case message(String), keychain, cancelled
    var errorDescription: String? {
        switch self {
        case .message(let message): message
        case .keychain: "Could not access the vault connection in macOS Keychain. Unlock Keychain and reconnect."
        case .cancelled: "Connection cancelled."
        }
    }
}
private final class NativeNoRedirects: NSObject, URLSessionTaskDelegate, @unchecked Sendable {
    func urlSession(_ session: URLSession, task: URLSessionTask, willPerformHTTPRedirection response: HTTPURLResponse, newRequest request: URLRequest, completionHandler: @escaping (URLRequest?) -> Void) {
        completionHandler(nil)
    }
}

/// Native-owner OAuth is separate from MCP read-only access. Secrets are stored
/// per origin in Keychain. Refresh tasks are shared so concurrent uploads cannot
/// redeem the same rotating refresh token twice.
@MainActor public final class NativeVaultSession: ObservableObject {
    public static let shared = NativeVaultSession()
    @Published public private(set) var server = ""
    @Published public private(set) var isConnected = false
    @Published public private(set) var isSigningIn = false
    @Published public private(set) var lastError: String?
    private let storage: any NativeCredentialStorage
    private let session: URLSession
    private let preferences: UserDefaults
    private var refreshes: [String: Task<NativeOAuthCredentials, Error>] = [:]
    /// Keychain reads are slow enough to matter on every request; the stored item stays the source of truth.
    private var cached: [String: NativeOAuthCredentials] = [:]
    private var login: Task<Void, Error>?
    private static let preference = "NativeVaultServer"

    public convenience init() {
        let configuration = URLSessionConfiguration.ephemeral
        configuration.timeoutIntervalForRequest = 30
        configuration.timeoutIntervalForResource = 60
        self.init(storage: NativeKeychainStorage(), session: URLSession(configuration: configuration, delegate: NativeNoRedirects(), delegateQueue: nil))
        if let remembered = UserDefaults.standard.string(forKey: Self.preference),
           let normalized = try? Self.origin(remembered),
           (try? stored(normalized)) != nil {
            server = normalized
            isConnected = true
        }
    }
    init(storage: any NativeCredentialStorage, session: URLSession, preferences: UserDefaults = .standard) {
        self.storage = storage
        self.session = session
        self.preferences = preferences
    }

    private func stored(_ server: String) throws -> NativeOAuthCredentials? {
        if let credentials = cached[server] { return credentials }
        let credentials = try storage.read(server: server)
        cached[server] = credentials
        return credentials
    }
    private func store(_ credentials: NativeOAuthCredentials) throws {
        try storage.save(credentials)
        cached[credentials.server] = credentials
    }

    static func origin(_ supplied: String) throws -> String {
        let connection = try VaultConnection(server: supplied.trimmingCharacters(in: .whitespacesAndNewlines), token: "address-validation")
        var components = URLComponents(url: connection.base, resolvingAgainstBaseURL: false)!
        components.scheme = components.scheme?.lowercased()
        components.host = components.host?.lowercased()
        components.path = ""
        if components.port == (components.scheme == "https" ? 443 : 80) { components.port = nil }
        return components.url!.absoluteString.trimmingCharacters(in: CharacterSet(charactersIn: "/"))
    }
    private static func random() throws -> String {
        var bytes = [UInt8](repeating: 0, count: 48)
        guard SecRandomCopyBytes(kSecRandomDefault, bytes.count, &bytes) == errSecSuccess else {
            throw NativeOAuthFailure.message("Could not create a secure connection request. Try again.")
        }
        return Data(bytes).base64EncodedString().replacingOccurrences(of: "+", with: "-").replacingOccurrences(of: "/", with: "_").replacingOccurrences(of: "=", with: "")
    }

    public func signIn(server supplied: String) async throws {
        guard !isSigningIn else { throw NativeOAuthFailure.message("Finish the current connection request first.") }
        let origin = try Self.origin(supplied)
        isSigningIn = true
        lastError = nil
        let task = Task { try await performSignIn(server: origin) }
        login = task
        defer { login = nil; isSigningIn = false }
        do { try await withTaskCancellationHandler { try await task.value } onCancel: { task.cancel() } }
        catch {
            lastError = error.localizedDescription
            throw error
        }
    }
    public func cancelSignIn() { login?.cancel() }

    /// Select a previously connected vault without opening the browser. Missing
    /// or revoked credentials leave that vault disconnected, never showing the
    /// preceding vault's data under a different profile name.
    public func use(server supplied: String) async throws {
        let origin = try Self.origin(supplied)
        server = origin
        isConnected = false
        lastError = nil
        guard try stored(origin) != nil else {
            throw NativeOAuthFailure.message("Connect this app to this vault first.")
        }
        do {
            _ = try await connection(for: origin)
            isConnected = true
            preferences.set(origin, forKey: Self.preference)
        } catch {
            lastError = error.localizedDescription
            throw error
        }
    }

    private func performSignIn(server: String) async throws {
        let state = try Self.random(), verifier = try Self.random()
        let receiver = try NativeLoopbackCallback(state: state)
        defer { receiver.cancel() }
        let callback = try await receiver.start()
        let registration = try await oauth(server: server, path: "/oauth/register", json: [
            "client_name": "Fundus for Mac", "redirect_uris": [callback.absoluteString],
            "token_endpoint_auth_method": "none", "scope": "vault:owner",
            "grant_types": ["authorization_code", "refresh_token"], "response_types": ["code"]
        ])
        guard let client = registration["client_id"] as? String else { throw NativeOAuthFailure.message("The server did not register this app. Update the server and try again.") }
        let challenge = Data(SHA256.hash(data: Data(verifier.utf8))).base64EncodedString().replacingOccurrences(of: "+", with: "-").replacingOccurrences(of: "/", with: "_").replacingOccurrences(of: "=", with: "")
        var authorization = URLComponents(string: server + "/oauth/authorize")!
        authorization.queryItems = [
            .init(name: "response_type", value: "code"), .init(name: "client_id", value: client),
            .init(name: "redirect_uri", value: callback.absoluteString), .init(name: "code_challenge", value: challenge),
            .init(name: "code_challenge_method", value: "S256"), .init(name: "scope", value: "vault:owner"),
            .init(name: "resource", value: server + "/api/native"), .init(name: "state", value: state)
        ]
        try Task.checkCancellation()
        guard NSWorkspace.shared.open(authorization.url!) else { throw NativeOAuthFailure.message("Could not open your browser. Set a default browser and try again.") }
        let returned = try await receiver.wait()
        let code = try Self.authorizationCode(from: returned, state: state, issuer: server)
        let tokens = try await oauth(server: server, path: "/oauth/token", form: [
            "grant_type": "authorization_code", "client_id": client, "code": code,
            "redirect_uri": callback.absoluteString, "code_verifier": verifier, "resource": server + "/api/native"
        ])
        let credentials = try Self.credentials(tokens, server: server, client: client)
        try Task.checkCancellation()
        try store(credentials)
        self.server = server
        isConnected = true
        preferences.set(server, forKey: Self.preference)
    }

    static func authorizationCode(from url: URL, state: String, issuer: String) throws -> String {
        let items = URLComponents(url: url, resolvingAgainstBaseURL: false)?.queryItems ?? []
        func single(_ name: String) -> String? {
            let matches = items.filter { $0.name == name }
            return matches.count == 1 ? matches[0].value : nil
        }
        guard single("state") == state, single("iss") == issuer else { throw NativeOAuthFailure.message("The connection response could not be verified. Start connecting again.") }
        if let error = single("error") {
            throw NativeOAuthFailure.message(error == "access_denied" ? "Vault access was not approved." : "The server could not approve this connection. Try again.")
        }
        guard let code = single("code"), !code.isEmpty else { throw NativeOAuthFailure.message("The server did not return an authorization code. Try connecting again.") }
        return code
    }
    private static func credentials(_ response: [String: Any], server: String, client: String) throws -> NativeOAuthCredentials {
        guard response["scope"] as? String == "vault:owner",
              let access = response["access_token"] as? String, !access.isEmpty,
              let refresh = response["refresh_token"] as? String, !refresh.isEmpty,
              let expires = response["expires_in"] as? Double, expires > 0 else {
            throw NativeOAuthFailure.message("The server returned an incomplete native connection. Update the server and reconnect.")
        }
        return NativeOAuthCredentials(server: server, clientID: client, accessToken: access, refreshToken: refresh, expiresAt: Date().addingTimeInterval(expires))
    }

    public static func connection(server: String, forceRefresh: Bool = false) async throws -> VaultConnection {
        try await shared.connection(for: server, forceRefresh: forceRefresh)
    }
    func connection(for supplied: String, forceRefresh: Bool = false) async throws -> VaultConnection {
        let origin = try Self.origin(supplied)
        guard let credentials = try stored(origin) else {
            guard let token = VaultCredentials.read(), !token.isEmpty else { throw NativeOAuthFailure.message("Connect this app to your vault first.") }
            return try VaultConnection(server: origin, token: token)
        }
        guard credentials.server == origin else {
            throw NativeOAuthFailure.message("The saved connection does not match this vault. Connect the app again.")
        }
        let current = !forceRefresh && credentials.expiresAt.timeIntervalSinceNow > 60
            ? credentials : try await refresh(credentials)
        return try VaultConnection(server: origin, token: current.accessToken, nativeOwner: true)
    }
    private func refresh(_ credentials: NativeOAuthCredentials) async throws -> NativeOAuthCredentials {
        if let running = refreshes[credentials.server] { return try await running.value }
        let task = Task { () throws -> NativeOAuthCredentials in
            let result = try await oauth(server: credentials.server, path: "/oauth/token", form: [
                "grant_type": "refresh_token", "client_id": credentials.clientID,
                "refresh_token": credentials.refreshToken, "resource": credentials.server + "/api/native"
            ])
            let updated = try Self.credentials(result, server: credentials.server, client: credentials.clientID)
            try Task.checkCancellation()
            try store(updated)
            return updated
        }
        refreshes[credentials.server] = task
        defer { refreshes[credentials.server] = nil }
        do {
            let updated = try await task.value
            if server == credentials.server { isConnected = true; lastError = nil }
            return updated
        }
        catch { if server == credentials.server { isConnected = false }; throw error }
    }

    public func request(_ path: String, method: String = "GET", json: [String: Any]? = nil) async throws -> URLRequest {
        guard !server.isEmpty else { throw NativeOAuthFailure.message("Connect this app to your vault first.") }
        return try await connection(for: server).request(path, method: method, json: json)
    }
    public func json(_ path: String, method: String = "GET", body: [String: Any]? = nil) async throws -> [String: Any] {
        let data = try await data(path, method: method, body: body)
        return (try? JSONSerialization.jsonObject(with: data)) as? [String: Any] ?? [:]
    }
    /// The response body of an owner API call; an expired access token is refreshed and retried once.
    public func data(_ path: String, method: String = "GET", body: [String: Any]? = nil) async throws -> Data {
        guard !server.isEmpty else { throw NativeOAuthFailure.message("Connect this app to your vault first.") }
        var connection = try await connection(for: server)
        var result = try await session.data(for: connection.request(path, method: method, json: body))
        if (result.1 as? HTTPURLResponse)?.statusCode == 401, connection.nativeOwner {
            connection = try await self.connection(for: server, forceRefresh: true)
            result = try await session.data(for: connection.request(path, method: method, json: body))
        }
        try VaultConnection.check(result.1, result.0)
        return result.0
    }
    public func signOut() async {
        cancelSignIn()
        let origin = server
        if let credentials = try? stored(origin) {
            _ = try? await oauth(server: origin, path: "/oauth/revoke", form: ["client_id": credentials.clientID, "token": credentials.refreshToken])
        }
        refreshes[origin]?.cancel()
        refreshes[origin] = nil
        try? storage.remove(server: origin)
        cached[origin] = nil
        isConnected = false
        server = ""
        lastError = nil
        preferences.removeObject(forKey: Self.preference)
    }

    private func oauth(server: String, path: String, json: [String: Any]? = nil, form: [String: String]? = nil) async throws -> [String: Any] {
        var request = URLRequest(url: URL(string: server + path)!)
        request.httpMethod = "POST"
        if let json {
            request.setValue("application/json", forHTTPHeaderField: "Content-Type")
            request.httpBody = try JSONSerialization.data(withJSONObject: json)
        } else if let form {
            let allowed = CharacterSet.alphanumerics.union(CharacterSet(charactersIn: "-._~"))
            request.httpBody = form.sorted { $0.key < $1.key }.map { key, value in
                key.addingPercentEncoding(withAllowedCharacters: allowed)! + "=" + value.addingPercentEncoding(withAllowedCharacters: allowed)!
            }.joined(separator: "&").data(using: .utf8)
            request.setValue("application/x-www-form-urlencoded", forHTTPHeaderField: "Content-Type")
        }
        let (data, response) = try await session.data(for: request)
        guard let http = response as? HTTPURLResponse else { throw NativeOAuthFailure.message("The server returned an invalid connection response.") }
        let value = (try? JSONSerialization.jsonObject(with: data)) as? [String: Any] ?? [:]
        guard (200..<300).contains(http.statusCode) else {
            if value["error"] as? String == "invalid_grant" { throw NativeOAuthFailure.message("This connection has expired or was revoked. Connect the app again.") }
            throw NativeOAuthFailure.message("The server could not connect this app. Check the server address and try again.")
        }
        return value
    }
}

@MainActor final class NativeLoopbackCallback {
    private let listener: NWListener
    private let queue = DispatchQueue(label: "vault.oauth.loopback")
    private let expectedState: String
    private var ready: CheckedContinuation<URL, Error>?
    private var callback: CheckedContinuation<URL, Error>?
    private var result: Result<URL, Error>?
    private var port: UInt16?
    private var timeout: Task<Void, Never>?
    init(state: String) throws {
        expectedState = state
        let parameters = NWParameters.tcp
        parameters.requiredLocalEndpoint = .hostPort(host: .ipv4(.loopback), port: .any)
        listener = try NWListener(using: parameters)
    }
    func start() async throws -> URL {
        try await withTaskCancellationHandler {
            try await withCheckedThrowingContinuation { continuation in
            ready = continuation
            listener.stateUpdateHandler = { [weak self] state in
                Task { @MainActor in
                    guard let self else { return }
                    switch state {
                    case .ready:
                        guard let port = self.listener.port?.rawValue else { return }
                        self.port = port
                        self.ready?.resume(returning: URL(string: "http://127.0.0.1:\(port)/callback")!)
                        self.ready = nil
                    case .failed:
                        self.finish(.failure(NativeOAuthFailure.message("Could not listen for the browser response. Try connecting again.")))
                    default: break
                    }
                }
            }
            listener.newConnectionHandler = { [weak self] connection in
                Task { @MainActor in self?.receive(connection, data: Data()) }
            }
            listener.start(queue: queue)
            timeout = Task { [weak self] in
                do { try await Task.sleep(for: .seconds(300)) } catch { return }
                self?.finish(.failure(NativeOAuthFailure.message("The connection request timed out. Try connecting again.")))
            }
            }
        } onCancel: { Task { @MainActor in self.cancel() } }
    }
    func wait() async throws -> URL {
        if let result { return try result.get() }
        return try await withTaskCancellationHandler {
            try await withCheckedThrowingContinuation { callback = $0 }
        } onCancel: { Task { @MainActor in self.cancel() } }
    }
    func cancel() { finish(.failure(NativeOAuthFailure.cancelled)) }
    private func finish(_ result: Result<URL, Error>) {
        guard self.result == nil else { return }
        self.result = result
        if let ready { ready.resume(with: result); self.ready = nil }
        callback?.resume(with: result)
        callback = nil
        timeout?.cancel()
        listener.cancel()
    }
    private func receive(_ connection: NWConnection, data: Data) {
        if data.isEmpty { connection.start(queue: queue) }
        connection.receive(minimumIncompleteLength: 1, maximumLength: max(1, 16384 - data.count)) { [weak self] bytes, _, complete, error in
            Task { @MainActor in
                guard let self else { connection.cancel(); return }
                var buffer = data
                if let bytes { buffer.append(bytes) }
                if buffer.count < 16384, buffer.range(of: Data("\r\n\r\n".utf8)) == nil, !complete, error == nil {
                    self.receive(connection, data: buffer)
                    return
                }
                let line = String(decoding: buffer, as: UTF8.self).components(separatedBy: "\r\n").first ?? ""
                let pieces = line.split(separator: " ")
                guard pieces.count == 3, pieces[0] == "GET", pieces[1].hasPrefix("/callback?"),
                      let port = self.port,
                      let url = URL(string: "http://127.0.0.1:\(port)" + pieces[1]),
                      let items = URLComponents(url: url, resolvingAgainstBaseURL: false)?.queryItems,
                      items.filter({ $0.name == "state" }).count == 1,
                      items.first(where: { $0.name == "state" })?.value == self.expectedState else {
                    self.reply(connection, status: "400 Bad Request", message: "This connection request was not recognized.", result: nil)
                    return
                }
                self.reply(connection, status: "200 OK", message: "Connection request received. You can close this tab and return to Fundus.", result: url)
            }
        }
    }
    private func reply(_ connection: NWConnection, status: String, message: String, result: URL?) {
        let content = Data(message.utf8)
        let header = "HTTP/1.1 \(status)\r\nContent-Type: text/plain; charset=utf-8\r\nContent-Length: \(content.count)\r\nCache-Control: no-store\r\nConnection: close\r\n\r\n"
        connection.send(content: Data(header.utf8) + content, completion: .contentProcessed { [weak self] _ in
            connection.cancel()
            if let result { Task { @MainActor in self?.finish(.success(result)) } }
        })
    }
}
#endif
