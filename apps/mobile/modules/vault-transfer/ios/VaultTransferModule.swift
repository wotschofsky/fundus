import ExpoModulesCore
import UIKit

public class VaultTransferModule: Module {
    public func definition() -> ModuleDefinition {
        Name("VaultTransfer")
        AsyncFunction("status") { () -> String in try await MainActor.run { try TransferQueue.shared.json() } }
        // Opens the vault's sign-in page; the owner connection stays in Keychain, never in JS.
        AsyncFunction("signIn") { (server: String, timezone: String) in try await TransferQueue.shared.signIn(server: server, timezone: timezone) }
        AsyncFunction("signOut") { try await TransferQueue.shared.signOut() }
        AsyncFunction("cancelSignIn") { await MainActor.run { NativeVaultSession.shared.cancelSignIn() } }
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
