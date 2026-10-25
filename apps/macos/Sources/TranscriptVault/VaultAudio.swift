import AVFoundation
import UniformTypeIdentifiers
import VaultNative

/// Streams an entry's playback audio from the vault. AVPlayer asks for byte ranges; each range is
/// fetched with a current owner token, so playback keeps working after the hour-long access token rotates.
final class VaultAudioLoader: NSObject, AVAssetResourceLoaderDelegate, @unchecked Sendable {
    private let server: String
    private let path: String
    private let queue = DispatchQueue(label: "vault.audio")
    /// Accessed on `queue` only.
    private var requests: [ObjectIdentifier: Task<Void, Never>] = [:]
    private static let session = URLSession(configuration: .ephemeral, delegate: NoRedirects(), delegateQueue: nil)
    /// AVPlayer asks for large or open-ended ranges; answering in slices keeps memory flat and
    /// starts playback sooner. AVFoundation requests the rest itself.
    private static let slice = 2 * 1024 * 1024

    private init(server: String, entryID: String) {
        self.server = server
        self.path = VaultAPI.entryPath(entryID, "/audio")
    }

    /// The asset to play. Keep the loader alive as long as the asset: AVFoundation holds it weakly.
    static func asset(server: String, entryID: String) -> (AVURLAsset, VaultAudioLoader) {
        let loader = VaultAudioLoader(server: server, entryID: entryID)
        let asset = AVURLAsset(url: URL(string: "vault-audio://playback/\(VaultAPI.component(entryID)).m4a")!)
        asset.resourceLoader.setDelegate(loader, queue: loader.queue)
        return (asset, loader)
    }

    func resourceLoader(_ resourceLoader: AVAssetResourceLoader,
                        shouldWaitForLoadingOfRequestedResource loadingRequest: AVAssetResourceLoadingRequest) -> Bool {
        let key = ObjectIdentifier(loadingRequest)
        let request = Unchecked(loadingRequest)
        requests[key] = Task { [self] in
            await serve(request.value)
            queue.async { self.requests[key] = nil }
        }
        return true
    }

    func resourceLoader(_ resourceLoader: AVAssetResourceLoader, didCancel loadingRequest: AVAssetResourceLoadingRequest) {
        requests.removeValue(forKey: ObjectIdentifier(loadingRequest))?.cancel()
    }

    private func serve(_ loading: AVAssetResourceLoadingRequest) async {
        let start = loading.dataRequest.map { $0.currentOffset } ?? 0
        let wanted = loading.dataRequest.map { $0.requestsAllDataToEndOfResource ? Self.slice : $0.requestedLength } ?? 2
        let length = max(1, min(wanted, Self.slice))
        let result: Result<(Data, HTTPURLResponse), Error>
        do { result = .success(try await fetch(bytes: start..<(start + Int64(length)))) } catch { result = .failure(error) }
        // Answer on the delegate queue, so a seek's cancellation cannot race the response.
        let request = Unchecked(loading)
        queue.async {
            let loading = request.value
            guard !loading.isCancelled, !loading.isFinished else { return }
            switch result {
            case .success(let (data, response)):
                if let info = loading.contentInformationRequest {
                    info.contentType = UTType.mpeg4Audio.identifier
                    info.isByteRangeAccessSupported = true
                    info.contentLength = Self.totalLength(response) ?? Int64(data.count)
                }
                loading.dataRequest?.respond(with: data)
                loading.finishLoading()
            case .failure(let error):
                if error is CancellationError || (error as? URLError)?.code == .cancelled { return }
                Log.audio.error("Playback range failed: \(error.localizedDescription, privacy: .public)")
                loading.finishLoading(with: error)
            }
        }
    }

    private func fetch(bytes range: Range<Int64>) async throws -> (Data, HTTPURLResponse) {
        for attempt in 0...1 {
            let connection = try await NativeVaultSession.connection(server: server, forceRefresh: attempt > 0)
            var request = try connection.request(path)
            request.setValue("bytes=\(range.lowerBound)-\(range.upperBound - 1)", forHTTPHeaderField: "Range")
            let (data, response) = try await Self.session.data(for: request)
            guard let http = response as? HTTPURLResponse else { throw URLError(.badServerResponse) }
            if http.statusCode == 401 && attempt == 0 { continue }
            try VaultConnection.check(http, data)
            return (data, http)
        }
        throw URLError(.userAuthenticationRequired)
    }

    /// `Content-Range: bytes 0-1/12345` → 12345.
    private static func totalLength(_ response: HTTPURLResponse) -> Int64? {
        if let range = response.value(forHTTPHeaderField: "Content-Range"), let total = range.split(separator: "/").last {
            return Int64(total)
        }
        return response.statusCode == 200 ? response.expectedContentLength : nil
    }
}

/// Bearer tokens must never follow a redirect to another origin.
private final class NoRedirects: NSObject, URLSessionTaskDelegate, Sendable {
    func urlSession(_ session: URLSession, task: URLSessionTask, willPerformHTTPRedirection response: HTTPURLResponse,
                    newRequest request: URLRequest, completionHandler: @escaping (URLRequest?) -> Void) {
        completionHandler(nil)
    }
}

/// Hands an AVFoundation request to a task; it is only touched from that task afterwards.
private struct Unchecked<Value>: @unchecked Sendable {
    let value: Value
    init(_ value: Value) { self.value = value }
}
