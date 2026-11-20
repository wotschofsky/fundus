#if os(macOS)
import CryptoKit
import Foundation
import Synchronization
import XCTest
@testable import VaultNative

@MainActor private final class MemoryNativeCredentials: NativeCredentialStorage {
    var values: [String: NativeOAuthCredentials] = [:]
    /// Saving or removing fails, as Keychain does when it cannot be written.
    var failSaves = false
    var failRemoves = false
    func read(server: String) -> NativeOAuthCredentials? { values[server] }
    func save(_ credentials: NativeOAuthCredentials) throws {
        if failSaves { throw CocoaError(.fileWriteUnknown) }
        values[credentials.server] = credentials
    }
    func remove(server: String) throws {
        if failRemoves { throw CocoaError(.fileWriteUnknown) }
        values[server] = nil
    }
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
/// Answers registration and code exchange, recording each request's path and body.
private final class SignInMockProtocol: URLProtocol, @unchecked Sendable {
    static let requests = Mutex<[(path: String, body: Data)]>([])
    override class func canInit(with request: URLRequest) -> Bool { true }
    override class func canonicalRequest(for request: URLRequest) -> URLRequest { request }
    override func startLoading() {
        // URLSession hands a POST body to a protocol as a stream.
        var body = request.httpBody ?? Data()
        if let stream = request.httpBodyStream {
            stream.open()
            defer { stream.close() }
            var buffer = [UInt8](repeating: 0, count: 4096)
            while case let count = stream.read(&buffer, maxLength: buffer.count), count > 0 { body.append(buffer, count: count) }
        }
        let path = request.url!.path, recorded = body
        Self.requests.withLock { $0.append((path, recorded)) }
        let reply = path == "/oauth/register" ? #"{"client_id":"synthetic-phone"}"#
            : #"{"access_token":"oa_synthetic-phone","refresh_token":"synthetic-phone-refresh","expires_in":3600,"scope":"vault:owner"}"#
        client?.urlProtocol(self, didReceive: HTTPURLResponse(url: request.url!, statusCode: 200, httpVersion: nil, headerFields: ["Content-Type": "application/json"])!, cacheStoragePolicy: .notAllowed)
        client?.urlProtocol(self, didLoad: Data(reply.utf8))
        client?.urlProtocolDidFinishLoading(self)
    }
    override func stopLoading() {}
}

/// Registration, code exchange and a refresh of a later grant answer at once. A refresh of the
/// first grant (`synthetic-old-refresh`), or a revoke, is held (`held`) until the test calls
/// `release()`, so it can finish a sign-in, grant stored, in between; while `failRefresh` is set,
/// that first grant's refresh is refused.
private final class ReconnectMockProtocol: URLProtocol, @unchecked Sendable {
    static let failRefresh = Mutex(false)
    /// A later grant's refresh is refused too, as after it was disconnected in Settings.
    static let failLaterRefresh = Mutex(false)
    /// Each code exchange issues a grant of its own.
    static let exchanges = Mutex(0)
    static let released = Mutex(false)
    static let held = Mutex<[String]>([])
    static func reset() {
        exchanges.withLock { $0 = 0 }
        released.withLock { $0 = false }
        held.withLock { $0 = [] }
    }
    static func release() { released.withLock { $0 = true } }
    /// Returns once a request for `path` is being held.
    static func holding(_ path: String) async throws {
        let deadline = Date().addingTimeInterval(5)
        while !held.withLock({ $0.contains(path) }) {
            guard Date() < deadline else { throw URLError(.timedOut) }
            try await Task.sleep(for: .milliseconds(5))
        }
    }
    private static func answer(_ deliver: @escaping @Sendable () -> Void, deadline: Date) {
        if released.withLock({ $0 }) || Date() > deadline { deliver() }
        else { DispatchQueue.global().asyncAfter(deadline: .now() + 0.005) { answer(deliver, deadline: deadline) } }
    }
    override class func canInit(with request: URLRequest) -> Bool { true }
    override class func canonicalRequest(for request: URLRequest) -> URLRequest { request }
    override func startLoading() {
        var body = request.httpBody ?? Data()
        if let stream = request.httpBodyStream {
            stream.open()
            defer { stream.close() }
            var buffer = [UInt8](repeating: 0, count: 4096)
            while case let count = stream.read(&buffer, maxLength: buffer.count), count > 0 { body.append(buffer, count: count) }
        }
        let url = request.url!, refresh = String(decoding: body, as: UTF8.self).contains("grant_type=refresh_token")
        let revoke = url.path == "/oauth/revoke"
        let grant = Self.exchanges.withLock { count in
            if url.path == "/oauth/token" && !refresh { count += 1 }
            return count > 1 ? "-\(count)" : ""
        }
        let reply = url.path == "/oauth/register" ? #"{"client_id":"synthetic-new-client"}"#
            : revoke ? "{}"
            : refresh ? #"{"access_token":"oa_synthetic-old-rotated","refresh_token":"synthetic-old-rotated","expires_in":3600,"scope":"vault:owner"}"#
            : #"{"access_token":"oa_synthetic-new\#(grant)","refresh_token":"synthetic-new-refresh\#(grant)","expires_in":3600,"scope":"vault:owner"}"#
        let firstGrantRefresh = String(decoding: body, as: UTF8.self).contains("refresh_token=synthetic-old-refresh")
        let refused = refresh && (firstGrantRefresh ? Self.failRefresh.withLock { $0 } : Self.failLaterRefresh.withLock { $0 })
        let deliver: @Sendable () -> Void = { [self] in
            client?.urlProtocol(self, didReceive: HTTPURLResponse(url: url, statusCode: refused ? 400 : 200, httpVersion: nil, headerFields: ["Content-Type": "application/json"])!, cacheStoragePolicy: .notAllowed)
            client?.urlProtocol(self, didLoad: Data((refused ? #"{"error":"invalid_grant"}"# : reply).utf8))
            client?.urlProtocolDidFinishLoading(self)
        }
        let firstGrant = String(decoding: body, as: UTF8.self).contains("refresh_token=synthetic-old-refresh")
        if (refresh && firstGrant) || revoke {
            Self.held.withLock { $0.append(url.path) }
            Self.answer(deliver, deadline: Date().addingTimeInterval(5))
        } else { deliver() }
    }
    override func stopLoading() {}
}

@MainActor final class NativeOAuthSessionTests: XCTestCase {
    private func phoneSession(_ storage: MemoryNativeCredentials) -> NativeVaultSession {
        let configuration = URLSessionConfiguration.ephemeral
        configuration.protocolClasses = [SignInMockProtocol.self]
        let suite = "NativeOAuthSessionTests-" + UUID().uuidString
        addTeardownBlock { UserDefaults().removePersistentDomain(forName: suite) }
        return NativeVaultSession(storage: storage, session: URLSession(configuration: configuration), preferences: UserDefaults(suiteName: suite)!)
    }
    private func parameters(_ items: [URLQueryItem]?) -> [String: String] {
        Dictionary((items ?? []).map { ($0.name, $0.value ?? "") }, uniquingKeysWith: { first, _ in first })
    }

    func testPhoneSignInRegistersAppSchemeAndRedeemsVerifiedCode() async throws {
        SignInMockProtocol.requests.withLock { $0 = [] }
        let storage = MemoryNativeCredentials()
        let session = phoneSession(storage)
        let origin = "https://vault.example.com"
        var opened: URL?
        try await session.authorize(server: origin, name: NativeVaultSession.phoneName, callback: NativeVaultSession.phoneCallback, state: "expected-state") { url in
            opened = url
            return URL(string: "dev.wsky.vaultuploader:/oauth?code=synthetic-code&state=expected-state&iss=https%3A%2F%2Fvault.example.com")!
        }
        let requests = SignInMockProtocol.requests.withLock { $0 }
        XCTAssertEqual(requests.map(\.path), ["/oauth/register", "/oauth/token"])
        let registration = try XCTUnwrap(JSONSerialization.jsonObject(with: requests[0].body) as? [String: Any])
        XCTAssertEqual(registration["client_name"] as? String, "Fundus for iPhone")
        XCTAssertEqual(registration["redirect_uris"] as? [String], ["dev.wsky.vaultuploader:/oauth"])
        XCTAssertEqual(registration["scope"] as? String, "vault:owner")
        XCTAssertEqual(registration["token_endpoint_auth_method"] as? String, "none")

        let page = try XCTUnwrap(opened.flatMap { URLComponents(url: $0, resolvingAgainstBaseURL: false) })
        XCTAssertEqual(page.scheme, "https")
        XCTAssertEqual(page.host, "vault.example.com")
        XCTAssertEqual(page.path, "/oauth/authorize")
        let query = parameters(page.queryItems)
        XCTAssertEqual(query["response_type"], "code")
        XCTAssertEqual(query["client_id"], "synthetic-phone")
        XCTAssertEqual(query["redirect_uri"], "dev.wsky.vaultuploader:/oauth")
        XCTAssertEqual(query["code_challenge_method"], "S256")
        XCTAssertEqual(query["scope"], "vault:owner")
        XCTAssertEqual(query["resource"], "https://vault.example.com/api/native")
        XCTAssertEqual(query["state"], "expected-state")

        var form = URLComponents()
        form.percentEncodedQuery = String(decoding: requests[1].body, as: UTF8.self)
        let exchange = parameters(form.queryItems)
        XCTAssertEqual(exchange["grant_type"], "authorization_code")
        XCTAssertEqual(exchange["client_id"], "synthetic-phone")
        XCTAssertEqual(exchange["code"], "synthetic-code")
        XCTAssertEqual(exchange["redirect_uri"], "dev.wsky.vaultuploader:/oauth")
        XCTAssertEqual(exchange["resource"], "https://vault.example.com/api/native")
        let verifier = try XCTUnwrap(exchange["code_verifier"])
        let challenge = Data(SHA256.hash(data: Data(verifier.utf8))).base64EncodedString()
            .replacingOccurrences(of: "+", with: "-").replacingOccurrences(of: "/", with: "_").replacingOccurrences(of: "=", with: "")
        XCTAssertEqual(query["code_challenge"], challenge)

        XCTAssertEqual(storage.values[origin]?.accessToken, "oa_synthetic-phone")
        XCTAssertEqual(storage.values[origin]?.clientID, "synthetic-phone")
        XCTAssertEqual(session.server, origin)
        XCTAssertTrue(session.isConnected)
        let connection = try await session.connection(for: origin)
        XCTAssertTrue(connection.nativeOwner)
        XCTAssertEqual(try connection.request("/api/v1/uploads").url?.path, "/api/native/v1/uploads")
    }

    func testPhoneSignInRedeemsNoCodeFromAForeignDeniedOrCancelledCallback() async throws {
        let origin = "https://vault.example.com"
        let callbacks = [
            "dev.wsky.vaultuploader:/oauth?code=synthetic-code&state=other&iss=https%3A%2F%2Fvault.example.com",
            "dev.wsky.vaultuploader:/oauth?code=synthetic-code&state=expected-state&iss=https%3A%2F%2Fother.example",
            "dev.wsky.vaultuploader:/oauth?error=access_denied&state=expected-state&iss=https%3A%2F%2Fvault.example.com",
            nil, // The user closed the sign-in sheet.
        ]
        for callback in callbacks {
            SignInMockProtocol.requests.withLock { $0 = [] }
            let storage = MemoryNativeCredentials()
            let session = phoneSession(storage)
            do {
                try await session.authorize(server: origin, name: NativeVaultSession.phoneName, callback: NativeVaultSession.phoneCallback, state: "expected-state") { _ in
                    guard let callback else { throw CancellationError() }
                    return URL(string: callback)!
                }
                XCTFail("\(callback ?? "cancel") must not connect")
            } catch {
                if callback?.contains("access_denied") == true { XCTAssertEqual(error.localizedDescription, "Vault access was not approved.") }
            }
            let paths = SignInMockProtocol.requests.withLock { $0.map(\.path) }
            XCTAssertEqual(paths, ["/oauth/register"], callback ?? "cancel")
            XCTAssertTrue(storage.values.isEmpty)
            XCTAssertFalse(session.isConnected)
        }
    }
    func testARefreshFinishingAfterANewSignInKeepsTheNewGrant() async throws {
        ReconnectMockProtocol.reset()
        let origin = "https://vault.example.com"
        let storage = MemoryNativeCredentials()
        storage.values[origin] = NativeOAuthCredentials(server: origin, clientID: "synthetic-old-client", accessToken: "oa_synthetic-old", refreshToken: "synthetic-old-refresh", expiresAt: .distantPast)
        let configuration = URLSessionConfiguration.ephemeral
        configuration.protocolClasses = [ReconnectMockProtocol.self]
        let suite = "NativeOAuthSessionTests-" + UUID().uuidString
        defer { UserDefaults().removePersistentDomain(forName: suite) }
        let session = NativeVaultSession(storage: storage, session: URLSession(configuration: configuration), preferences: UserDefaults(suiteName: suite)!)
        // An upload starts refreshing the old grant, and the owner connects again before it answers.
        async let refreshing = session.connection(for: origin)
        try await ReconnectMockProtocol.holding("/oauth/token")
        try await session.authorize(server: origin, name: NativeVaultSession.phoneName, callback: NativeVaultSession.phoneCallback, state: "synthetic-state") { _ in
            URL(string: "dev.wsky.vaultuploader:/oauth?code=synthetic-code&state=synthetic-state&iss=\(origin)")!
        }
        ReconnectMockProtocol.release()
        _ = try await refreshing
        XCTAssertEqual(storage.values[origin]?.refreshToken, "synthetic-new-refresh")
        let current = try await session.connection(for: origin)
        XCTAssertEqual(current.token, "oa_synthetic-new")
    }
    func testARefreshFailingAfterANewSignInKeepsTheNewGrantConnected() async throws {
        ReconnectMockProtocol.reset()
        ReconnectMockProtocol.failRefresh.withLock { $0 = true }
        defer { ReconnectMockProtocol.failRefresh.withLock { $0 = false } }
        let origin = "https://vault.example.com"
        let storage = MemoryNativeCredentials()
        storage.values[origin] = NativeOAuthCredentials(server: origin, clientID: "synthetic-old-client", accessToken: "oa_synthetic-old", refreshToken: "synthetic-old-refresh", expiresAt: .distantPast)
        let configuration = URLSessionConfiguration.ephemeral
        configuration.protocolClasses = [ReconnectMockProtocol.self]
        let suite = "NativeOAuthSessionTests-" + UUID().uuidString
        defer { UserDefaults().removePersistentDomain(forName: suite) }
        let session = NativeVaultSession(storage: storage, session: URLSession(configuration: configuration), preferences: UserDefaults(suiteName: suite)!)
        // The old grant's refresh is refused only after the owner has connected again.
        async let refreshing = session.connection(for: origin)
        try await ReconnectMockProtocol.holding("/oauth/token")
        try await session.authorize(server: origin, name: NativeVaultSession.phoneName, callback: NativeVaultSession.phoneCallback, state: "synthetic-state") { _ in
            URL(string: "dev.wsky.vaultuploader:/oauth?code=synthetic-code&state=synthetic-state&iss=\(origin)")!
        }
        ReconnectMockProtocol.release()
        let connection = try await refreshing
        XCTAssertEqual(connection.token, "oa_synthetic-new")
        XCTAssertTrue(session.isConnected)
        XCTAssertEqual(storage.values[origin]?.refreshToken, "synthetic-new-refresh")
    }
    func testASlowDisconnectDoesNotEraseASignInThatFinishesFirst() async throws {
        ReconnectMockProtocol.reset()
        let origin = "https://vault.example.com"
        let storage = MemoryNativeCredentials()
        let configuration = URLSessionConfiguration.ephemeral
        configuration.protocolClasses = [ReconnectMockProtocol.self]
        let suite = "NativeOAuthSessionTests-" + UUID().uuidString
        defer { UserDefaults().removePersistentDomain(forName: suite) }
        let session = NativeVaultSession(storage: storage, session: URLSession(configuration: configuration), preferences: UserDefaults(suiteName: suite)!)
        let connect = {
            try await session.authorize(server: origin, name: NativeVaultSession.phoneName, callback: NativeVaultSession.phoneCallback, state: "synthetic-state") { _ in
                URL(string: "dev.wsky.vaultuploader:/oauth?code=synthetic-code&state=synthetic-state&iss=\(origin)")!
            }
        }
        try await connect()
        // The revoke request is slow; the owner connects again before it answers.
        async let disconnecting: Void = session.signOut()
        try await ReconnectMockProtocol.holding("/oauth/revoke")
        try await connect()
        ReconnectMockProtocol.release()
        await disconnecting
        XCTAssertEqual(storage.values[origin]?.refreshToken, "synthetic-new-refresh-2")
        XCTAssertEqual(session.server, origin)
        XCTAssertTrue(session.isConnected)
    }
    func testARequestUnderTheNewGrantNeverWaitsOnTheOldGrantsRefusedRefresh() async throws {
        ReconnectMockProtocol.reset()
        ReconnectMockProtocol.failRefresh.withLock { $0 = true }
        defer { ReconnectMockProtocol.failRefresh.withLock { $0 = false } }
        let origin = "https://vault.example.com"
        let storage = MemoryNativeCredentials()
        storage.values[origin] = NativeOAuthCredentials(server: origin, clientID: "synthetic-old-client", accessToken: "oa_synthetic-old", refreshToken: "synthetic-old-refresh", expiresAt: .distantPast)
        let configuration = URLSessionConfiguration.ephemeral
        configuration.protocolClasses = [ReconnectMockProtocol.self]
        let suite = "NativeOAuthSessionTests-" + UUID().uuidString
        defer { UserDefaults().removePersistentDomain(forName: suite) }
        let session = NativeVaultSession(storage: storage, session: URLSession(configuration: configuration), preferences: UserDefaults(suiteName: suite)!)
        // The old grant's refresh is under way when the owner connects again; then a request made
        // under the new grant is refused and forces a refresh of its own.
        async let underOldGrant = session.connection(for: origin)
        try await ReconnectMockProtocol.holding("/oauth/token")
        try await session.authorize(server: origin, name: NativeVaultSession.phoneName, callback: NativeVaultSession.phoneCallback, state: "synthetic-state") { _ in
            URL(string: "dev.wsky.vaultuploader:/oauth?code=synthetic-code&state=synthetic-state&iss=\(origin)")!
        }
        // The new grant's refresh completes while the old grant's is still held.
        _ = try await session.connection(for: origin, forceRefresh: true)
        XCTAssertEqual(ReconnectMockProtocol.held.withLock { $0 }, ["/oauth/token"])
        ReconnectMockProtocol.release()
        _ = try await underOldGrant
        XCTAssertTrue(session.isConnected)
        XCTAssertNotEqual(storage.values[origin]?.refreshToken, "synthetic-old-refresh")
    }
    func testARotationThatCannotBeSavedIsStillUsedNotRedeemedAgain() async throws {
        await NativeOAuthMockProtocol.server.reset()
        let origin = "https://vault.example.com"
        let storage = MemoryNativeCredentials()
        storage.values[origin] = NativeOAuthCredentials(server: origin, clientID: "synthetic-client", accessToken: "oa_synthetic-old", refreshToken: "synthetic-old-refresh", expiresAt: .distantPast)
        storage.failSaves = true
        let configuration = URLSessionConfiguration.ephemeral
        configuration.protocolClasses = [NativeOAuthMockProtocol.self]
        let session = NativeVaultSession(storage: storage, session: URLSession(configuration: configuration))
        // The server rotated the token; reusing the old one would revoke the whole grant.
        let first = try await session.connection(for: origin)
        XCTAssertEqual(first.token, "oa_synthetic-next")
        let again = try await session.connection(for: origin)
        XCTAssertEqual(again.token, "oa_synthetic-next")
        let refreshes = await NativeOAuthMockProtocol.server.count()
        XCTAssertEqual(refreshes, 1)
    }
    func testALateRefreshDoesNotReconnectAGrantRevokedMeanwhile() async throws {
        ReconnectMockProtocol.reset()
        ReconnectMockProtocol.failLaterRefresh.withLock { $0 = true }
        defer { ReconnectMockProtocol.failLaterRefresh.withLock { $0 = false } }
        let origin = "https://vault.example.com"
        let storage = MemoryNativeCredentials()
        storage.values[origin] = NativeOAuthCredentials(server: origin, clientID: "synthetic-old-client", accessToken: "oa_synthetic-old", refreshToken: "synthetic-old-refresh", expiresAt: .distantPast)
        let configuration = URLSessionConfiguration.ephemeral
        configuration.protocolClasses = [ReconnectMockProtocol.self]
        let suite = "NativeOAuthSessionTests-" + UUID().uuidString
        defer { UserDefaults().removePersistentDomain(forName: suite) }
        let session = NativeVaultSession(storage: storage, session: URLSession(configuration: configuration), preferences: UserDefaults(suiteName: suite)!)
        // The old grant's refresh is under way; the owner connects again, then that new grant is
        // disconnected in Settings and its refresh refused.
        async let underOldGrant = session.connection(for: origin)
        try await ReconnectMockProtocol.holding("/oauth/token")
        try await session.authorize(server: origin, name: NativeVaultSession.phoneName, callback: NativeVaultSession.phoneCallback, state: "synthetic-state") { _ in
            URL(string: "dev.wsky.vaultuploader:/oauth?code=synthetic-code&state=synthetic-state&iss=\(origin)")!
        }
        do { _ = try await session.connection(for: origin, forceRefresh: true); XCTFail("The revoked grant must not refresh") } catch {}
        XCTAssertFalse(session.isConnected)
        // The old grant's refresh answering late must not mark the revoked one connected again.
        ReconnectMockProtocol.release()
        do { _ = try await underOldGrant; XCTFail("The revoked grant's token must not be handed out") } catch {}
        XCTAssertFalse(session.isConnected)
    }
    func testARefusedGrantIsForgottenNotRedeemedAgain() async throws {
        ReconnectMockProtocol.reset()
        ReconnectMockProtocol.release()
        ReconnectMockProtocol.failRefresh.withLock { $0 = true }
        defer { ReconnectMockProtocol.failRefresh.withLock { $0 = false } }
        let origin = "https://vault.example.com"
        let storage = MemoryNativeCredentials()
        storage.values[origin] = NativeOAuthCredentials(server: origin, clientID: "synthetic-old-client", accessToken: "oa_synthetic-old", refreshToken: "synthetic-old-refresh", expiresAt: .distantPast)
        let configuration = URLSessionConfiguration.ephemeral
        configuration.protocolClasses = [ReconnectMockProtocol.self]
        let session = NativeVaultSession(storage: storage, session: URLSession(configuration: configuration))
        // The server refuses the grant (revoked, or expired): only a new sign-in can help.
        do { _ = try await session.connection(for: origin); XCTFail("A refused grant must not connect") } catch {}
        XCTAssertNil(storage.values[origin])
        // Resuming again does not send the refused token a second time.
        do { _ = try await session.connection(for: origin); XCTFail("A refused grant must not connect") } catch {}
        XCTAssertEqual(ReconnectMockProtocol.held.withLock { $0 }.filter { $0 == "/oauth/token" }.count, 1)
    }
    func testARefusedGrantKeychainCannotForgetIsStillNeverRedeemedAgain() async throws {
        ReconnectMockProtocol.reset()
        ReconnectMockProtocol.release()
        ReconnectMockProtocol.failRefresh.withLock { $0 = true }
        defer { ReconnectMockProtocol.failRefresh.withLock { $0 = false } }
        let origin = "https://vault.example.com"
        let storage = MemoryNativeCredentials()
        storage.values[origin] = NativeOAuthCredentials(server: origin, clientID: "synthetic-old-client", accessToken: "oa_synthetic-old", refreshToken: "synthetic-old-refresh", expiresAt: .distantPast)
        storage.failRemoves = true
        let configuration = URLSessionConfiguration.ephemeral
        configuration.protocolClasses = [ReconnectMockProtocol.self]
        let session = NativeVaultSession(storage: storage, session: URLSession(configuration: configuration))
        do { _ = try await session.connection(for: origin); XCTFail("A refused grant must not connect") } catch {}
        // Keychain still holds the refused grant; this process never sends its token again.
        do { _ = try await session.connection(for: origin); XCTFail("A refused grant must not connect") } catch {}
        XCTAssertEqual(ReconnectMockProtocol.held.withLock { $0 }.filter { $0 == "/oauth/token" }.count, 1)
    }
    func testADisconnectedVaultKeychainCannotForgetStaysDisconnected() async throws {
        ReconnectMockProtocol.reset()
        ReconnectMockProtocol.release()
        let origin = "https://vault.example.com"
        let storage = MemoryNativeCredentials()
        let configuration = URLSessionConfiguration.ephemeral
        configuration.protocolClasses = [ReconnectMockProtocol.self]
        let suite = "NativeOAuthSessionTests-" + UUID().uuidString
        defer { UserDefaults().removePersistentDomain(forName: suite) }
        let session = NativeVaultSession(storage: storage, session: URLSession(configuration: configuration), preferences: UserDefaults(suiteName: suite)!)
        try await session.authorize(server: origin, name: NativeVaultSession.phoneName, callback: NativeVaultSession.phoneCallback, state: "synthetic-state") { _ in
            URL(string: "dev.wsky.vaultuploader:/oauth?code=synthetic-code&state=synthetic-state&iss=\(origin)")!
        }
        // Keychain cannot remove the item; the disconnected grant must not come back from it.
        storage.failRemoves = true
        await session.signOut()
        do { _ = try await session.connection(for: origin); XCTFail("A disconnected vault must stay disconnected") } catch {}
    }
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
