import ExpoModulesCore
import UIKit

public class VaultTransferModule: Module {
    public func definition() -> ModuleDefinition {
        Name("VaultTransfer")
        AsyncFunction("status") { () -> String in try await MainActor.run { try TransferQueue.shared.json() } }
        AsyncFunction("configure") { (server: String, token: String, timezone: String) in try await MainActor.run { try TransferQueue.shared.configure(server: server, token: token, timezone: timezone) } }
        AsyncFunction("enqueue") { (files: [[String: String]]) in
            let urls = files.compactMap { URL(string: $0["uri"] ?? "") }
            try await TransferQueue.shared.enqueue(urls)
        }
        AsyncFunction("resume") { await MainActor.run { TransferQueue.shared.resume() } }
        AsyncFunction("pause") { await MainActor.run { TransferQueue.shared.pause() } }
        AsyncFunction("clear") { try await MainActor.run { try TransferQueue.shared.clear() } }
    }
}
public class VaultTransferAppDelegateSubscriber: ExpoAppDelegateSubscriber {
    public func application(_ application: UIApplication, handleEventsForBackgroundURLSession identifier: String, completionHandler: @escaping () -> Void) {
        guard identifier == TransferQueue.sessionIdentifier else { completionHandler(); return }
        Task { @MainActor in TransferQueue.shared.backgroundCompletion = completionHandler }
    }
}
