import SwiftUI
import VaultNative

/// The current batch: each file's copy, lossless preparation and upload, then the server's receipt.
struct UploadsView: View {
    @Environment(AppModel.self) private var app

    var body: some View {
        Table(app.transfer.files) {
            TableColumn("Recording") { item in
                Text(item.name)
                    .lineLimit(1)
                    .truncationMode(.middle)
                    .help(item.name)
            }
            TableColumn("Status") { item in
                HStack(spacing: 8) {
                    if let fraction = Upload.fraction(of: item) {
                        ProgressView(value: fraction)
                            .controlSize(.small)
                            .frame(width: 80)
                    }
                    Text(item.error ?? Upload.status(of: item))
                        .foregroundStyle(item.error == nil ? Color.secondary : Color.red)
                        .lineLimit(1)
                        .help(item.error ?? "")
                }
            }
            .width(min: 160, ideal: 260)
        }
        .navigationTitle("Uploads")
        .navigationSubtitle(app.vaultName)
        .safeAreaInset(edge: .bottom, spacing: 0) {
            VStack(spacing: 0) {
                Divider()
                HStack {
                    TransferSummary(showsDetails: false)
                    Spacer()
                    if ["failed", "paused", "verified"].contains(app.transfer.state) {
                        Button("Clear") {
                            do { try app.queue.clear() } catch { app.report("Could not clear the uploads", error) }
                        }
                        .help("Remove the local copies of these files. Recordings already on the vault stay there.")
                    }
                }
                .padding(.horizontal, 16)
                .padding(.vertical, 10)
            }
            .background(.bar)
        }
    }
}

/// One line of transfer state with Pause/Resume, for the Import screen and Uploads.
struct TransferSummary: View {
    var showsDetails = true
    @Environment(AppModel.self) private var app

    var body: some View {
        let state = app.transfer
        HStack(spacing: 12) {
            VStack(alignment: .leading, spacing: 4) {
                Text(Upload.headline(state))
                    .font(.callout)
                    .foregroundStyle(state.state == "failed" ? .red : .primary)
                    .lineLimit(2)
                    .textSelection(.enabled)
                if app.isTransferring {
                    ProgressView(value: Upload.progress(state))
                        .controlSize(.small)
                        .frame(maxWidth: 220)
                }
            }
            if app.isTransferring {
                Button("Pause") { app.queue.pause() }
            } else if ["failed", "paused"].contains(state.state) {
                Button("Resume") { app.queue.resume() }
            }
            if showsDetails {
                Button("Details") { app.selection = .uploads }
                    .buttonStyle(.link)
            }
        }
    }
}

enum Upload {
    /// Whole-batch progress: preparation counts for a third of a file, the upload for the rest.
    static func progress(_ state: QueueState) -> Double {
        guard !state.files.isEmpty else { return 0 }
        let total = state.files.reduce(0.0) { sum, item in
            switch item.state {
            case "preparing": sum + 0.3 * (item.preparationProgress ?? 0)
            case "ready": sum + 0.3
            case "uploading": sum + 0.3 + 0.7 * (item.length > 0 ? Double(item.offset) / Double(item.length) : 0)
            case "uploaded", "verified": sum + 1
            default: sum
            }
        }
        return total / Double(state.files.count)
    }

    static func headline(_ state: QueueState) -> String {
        let done = state.files.filter { ["uploaded", "verified"].contains($0.state) }.count
        let count = state.files.count
        switch state.state {
        case "copying": return "Copying from the recorder…"
        case "uploading": return "Uploading \(done) of \(count) \(count == 1 ? "file" : "files")"
        case "verifying": return "Verifying on the vault…"
        case "verified": return "\(count) \(count == 1 ? "file" : "files") sent to the vault"
        case "paused": return "Upload paused"
        case "failed": return state.error ?? "Upload stopped"
        default: return ""
        }
    }

    static func status(of item: UploadItem) -> String {
        switch item.state {
        case "waiting": "Waiting"
        case "copying": "Copying"
        case "preparing": "Preparing lossless copy"
        case "ready": "Ready to upload"
        case "uploading": item.estimatedSecondsRemaining.map { "Uploading · \(Format.remaining($0))" } ?? "Uploading"
        case "uploaded": "Uploaded"
        case "verified": "Verified on the vault"
        case let other: other.capitalized
        }
    }

    static func fraction(of item: UploadItem) -> Double? {
        switch item.state {
        case "preparing": item.preparationProgress
        case "uploading": item.length > 0 ? Double(item.offset) / Double(item.length) : nil
        default: nil
        }
    }
}
