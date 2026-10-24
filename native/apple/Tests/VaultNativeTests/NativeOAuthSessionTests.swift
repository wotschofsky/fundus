#if os(macOS)
import Foundation
import XCTest
@testable import VaultNative

@MainActor private final class MemoryNativeCredentials: NativeCredentialStorage {
    var values: [String: NativeOAuthCredentials] = [:]
    func read(server: String) -> NativeOAuthCredentials? { values[server] }
    func save(_ credentials: NativeOAuthCredentials) { values[credentials.server] = credentials }
    func remove(server: String) { values[server] = nil }
}
private actor NativeOAuthMockServer {
    private var refreshes = 0
    func reset() { refreshes = 0 }
    func count() -> Int { refreshes }
    func respond(_ request: URLRequest) async -> (Int, Data) {
        guard request.url?.path == "/oauth/token" else { return (404, Data()) }
        refreshes += 1
        try? await Task.sleep(for: .milliseconds(40))
        return (200, Data(#"{"access_token":"oa_synthetic-next","refresh_token":"synthetic-rotated","expires_in":3600,"scope":"vault:owner"}"#.utf8))
    }
}
private final class NativeOAuthMockProtocol: URLProtocol, @unchecked Sendable {
    static let server = NativeOAuthMockServer()
    private var work: Task<Void, Never>?
    override class func canInit(with request: URLRequest) -> Bool { true }
    override class func canonicalRequest(for request: URLRequest) -> URLRequest { request }
    override func startLoading() {
        work = Task {
            let (status, data) = await Self.server.respond(request)
            let response = HTTPURLResponse(url: request.url!, statusCode: status, httpVersion: nil, headerFields: ["Content-Type": "application/json"])!
            client?.urlProtocol(self, didReceive: response, cacheStoragePolicy: .notAllowed)
            client?.urlProtocol(self, didLoad: data)
            client?.urlProtocolDidFinishLoading(self)
        }
    }
    override func stopLoading() { work?.cancel() }
}

@MainActor final class NativeOAuthSessionTests: XCTestCase {
    func testConcurrentUploadsShareOneRotatingRefresh() async throws {
        await NativeOAuthMockProtocol.server.reset()
        let storage = MemoryNativeCredentials()
        storage.values["https://vault.example.com"] = NativeOAuthCredentials(server: "https://vault.example.com", clientID: "synthetic-client", accessToken: "oa_synthetic-old", refreshToken: "synthetic-old-refresh", expiresAt: .distantPast)
        let configuration = URLSessionConfiguration.ephemeral
        configuration.protocolClasses = [NativeOAuthMockProtocol.self]
        let session = NativeVaultSession(storage: storage, session: URLSession(configuration: configuration))
        async let first = session.connection(for: "https://vault.example.com")
        async let second = session.connection(for: "https://vault.example.com/")
        async let third = session.connection(for: "https://vault.example.com:443")
        let connections = try await [first, second, third]
        XCTAssertTrue(connections.allSatisfy { $0.nativeOwner && $0.token == "oa_synthetic-next" })
        let count = await NativeOAuthMockProtocol.server.count()
        XCTAssertEqual(count, 1)
        XCTAssertEqual(storage.values["https://vault.example.com"]?.refreshToken, "synthetic-rotated")
        _ = try await session.connection(for: "https://vault.example.com")
        let after = await NativeOAuthMockProtocol.server.count()
        XCTAssertEqual(after, 1)
    }

    func testRestoresSavedProfileWithoutBrowserAndNeverKeepsPreviousVaultConnected() async throws {
        let storage = MemoryNativeCredentials()
        let origin = "https://vault.example.com"
        storage.values[origin] = NativeOAuthCredentials(server: origin, clientID: "synthetic-client", accessToken: "oa_synthetic", refreshToken: "synthetic-refresh", expiresAt: Date().addingTimeInterval(3600))
        let suite = "NativeOAuthSessionTests-" + UUID().uuidString
        let preferences = UserDefaults(suiteName: suite)!
        defer { preferences.removePersistentDomain(forName: suite) }
        let configuration = URLSessionConfiguration.ephemeral
        configuration.protocolClasses = [NativeOAuthMockProtocol.self]
        let session = NativeVaultSession(storage: storage, session: URLSession(configuration: configuration), preferences: preferences)
        try await session.use(server: origin)
        XCTAssertEqual(session.server, origin)
        XCTAssertTrue(session.isConnected)
        do {
            try await session.use(server: "https://other.example")
            XCTFail("An unconnected profile must require sign in")
        } catch { }
        XCTAssertEqual(session.server, "https://other.example")
        XCTAssertFalse(session.isConnected)
        XCTAssertNotNil(storage.values[origin])
    }

    func testNativeRequestsUseDedicatedAudienceAndPreserveTusLocations() throws {
        let connection = try VaultConnection(server: "https://vault.example.com", token: "oa_synthetic", nativeOwner: true)
        XCTAssertEqual(try connection.request("/api/v1/entries?limit=1").url?.absoluteString, "https://vault.example.com/api/native/v1/entries?limit=1")
        XCTAssertEqual(try connection.request("https://vault.example.com/api/native/v1/uploads/123", method: "HEAD").url?.path, "/api/native/v1/uploads/123")
        XCTAssertEqual(try connection.request("https://vault.example.com/api/v1/uploads/123", method: "HEAD").url?.path, "/api/native/v1/uploads/123")
        XCTAssertThrowsError(try connection.request("https://other.example/api/v1/entries"))
        let legacy = try VaultConnection(server: "https://vault.example.com", token: "synthetic-upload")
        XCTAssertEqual(try legacy.request("/api/v1/uploads").url?.path, "/api/v1/uploads")
    }

    func testAuthorizationResponseChecksStateIssuerAndDuplicateParameters() throws {
        let origin = "https://vault.example.com"
        let valid = URL(string: "http://127.0.0.1:57231/callback?state=expected&iss=https%3A%2F%2Fvault.example.com&code=synthetic-code")!
        XCTAssertEqual(try NativeVaultSession.authorizationCode(from: valid, state: "expected", issuer: origin), "synthetic-code")
        XCTAssertThrowsError(try NativeVaultSession.authorizationCode(from: valid, state: "wrong", issuer: origin))
        XCTAssertThrowsError(try NativeVaultSession.authorizationCode(from: valid, state: "expected", issuer: "https://other.example"))
        XCTAssertThrowsError(try NativeVaultSession.authorizationCode(from: URL(string: valid.absoluteString + "&code=other")!, state: "expected", issuer: origin))
    }

    func testLoopbackCallbackIgnoresWrongStateAndAcceptsMatchingBrowserResponse() async throws {
        let listener = try NativeLoopbackCallback(state: "expected-state")
        defer { listener.cancel() }
        let callback = try await listener.start()
        XCTAssertEqual(callback.host, "127.0.0.1")
        XCTAssertNotNil(callback.port)
        let waiting = Task { try await listener.wait() }
        let (_, wrong) = try await URLSession.shared.data(from: URL(string: callback.absoluteString + "?state=wrong&code=synthetic")!)
        XCTAssertEqual((wrong as? HTTPURLResponse)?.statusCode, 400)
        let (_, correct) = try await URLSession.shared.data(from: URL(string: callback.absoluteString + "?state=expected-state&code=synthetic&iss=https%3A%2F%2Fvault.example.com")!)
        XCTAssertEqual((correct as? HTTPURLResponse)?.statusCode, 200)
        let result = try await waiting.value
        XCTAssertEqual(try NativeVaultSession.authorizationCode(from: result, state: "expected-state", issuer: "https://vault.example.com"), "synthetic")
    }
}
#endif
