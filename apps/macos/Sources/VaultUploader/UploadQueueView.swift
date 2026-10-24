import SwiftUI
import VaultNative

struct UploadQueueView: View {
    @ObservedObject var queue: TransferQueue
    let addFiles: () -> Void
    @State private var error: String?

    var body: some View {
        VStack(spacing: 0) {
            if queue.snapshot.files.isEmpty {
                ContentUnavailableView("No uploads", systemImage: "square.and.arrow.up", description: Text("Connect a microphone to choose recordings, or add audio files."))
                    .frame(maxWidth: .infinity, maxHeight: .infinity)
            } else {
                Table(queue.snapshot.files) {
                    TableColumn("Recording") { item in
                        Text(item.name).lineLimit(1).truncationMode(.middle).help(item.name)
                    }
                    TableColumn("Status") { item in
                        VStack(alignment: .leading, spacing: 4) {
                            Text(item.state.capitalized).foregroundStyle(.secondary)
                            if item.state == "preparing", let progress = item.preparationProgress {
                                ProgressView(value: progress)
                            }
                            if item.state == "uploading", let eta = item.estimatedSecondsRemaining {
                                Text(Format.remaining(eta)).font(.caption).foregroundStyle(.secondary)
                            }
                            if item.state == "uploading", item.length > 0 {
                                ProgressView(value: Double(item.offset), total: Double(item.length))
                            }
                            if let failure = item.error { Text(failure).foregroundStyle(.red) }
                        }
                        .padding(.vertical, 5)
                    }
                    .width(min: 100, ideal: 140, max: 180)
                }
                .scrollContentBackground(.hidden)
            }
            if let message = error ?? queue.snapshot.error {
                Label(message, systemImage: "exclamationmark.circle")
                    .foregroundStyle(.red).font(.callout)
                    .textSelection(.enabled)
                    .frame(maxWidth: .infinity, alignment: .leading)
                    .padding(16)
            }
        }
        .navigationTitle("Uploads")
        .navigationSubtitle(URL(string: queue.snapshot.server)?.host ?? "Vault Uploader")
        .safeAreaInset(edge: .bottom, spacing: 0) {
            if queue.snapshot.state != "idle" || !queue.snapshot.files.isEmpty {
                HStack(spacing: 12) {
                    if ["copying", "uploading", "verifying"].contains(queue.snapshot.state) {
                        ProgressView().controlSize(.small)
                    }
                    Text(queue.snapshot.state == "verified" ? "Verified on server" : queue.snapshot.state.capitalized)
                        .foregroundStyle(.secondary)
                    Spacer()
                    if !queue.snapshot.files.isEmpty && ["failed", "paused"].contains(queue.snapshot.state) {
                        Button("Resume") { error = nil; queue.resume() }
                    }
                    if ["copying", "uploading", "verifying"].contains(queue.snapshot.state) {
                        Button("Pause") { queue.pause() }
                    }
                    if ["failed", "paused", "verified"].contains(queue.snapshot.state) {
                        Button("Clear") {
                            do { try queue.clear(); error = nil }
                            catch { self.error = error.localizedDescription }
                        }
                    }
                }
                .padding(.horizontal, 16)
                .padding(.vertical, 12)
                .background(.bar)
            }
        }
    }
}
