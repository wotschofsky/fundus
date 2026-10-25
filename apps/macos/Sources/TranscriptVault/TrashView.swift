import SwiftUI

/// Recordings moved to Trash: hidden from the library and from chat agents until restored.
struct TrashView: View {
    @Environment(AppModel.self) private var app
    @State private var deleting: Entry?

    var body: some View {
        Group {
            if app.library.trash.isEmpty {
                ContentUnavailableView("Trash Is Empty", systemImage: "trash",
                                       description: Text("Recordings you move to Trash stay on the vault until you delete them here."))
            } else {
                List(app.library.trash) { entry in
                    HStack {
                        VStack(alignment: .leading, spacing: 2) {
                            Text(entry.displayTitle).lineLimit(1)
                            Text("\(entry.dateLine) · \(Format.duration(entry.duration))")
                                .font(.callout)
                                .foregroundStyle(.secondary)
                        }
                        Spacer()
                        Button("Restore") { Task { await app.setProcessing(.restore, entry: entry.id) } }
                        Button("Delete…", role: .destructive) { deleting = entry }
                    }
                    .padding(.vertical, 3)
                    .contextMenu {
                        Button("Restore") { Task { await app.setProcessing(.restore, entry: entry.id) } }
                        Button("Delete Permanently…", role: .destructive) { deleting = entry }
                    }
                }
                .listStyle(.inset)
            }
        }
        .navigationTitle("Trash")
        .navigationSubtitle(app.library.trash.isEmpty ? "" : "\(app.library.trash.count) \(app.library.trash.count == 1 ? "recording" : "recordings")")
        .task(id: app.server) { await app.library.refreshTrash() }
        .confirmationDialog("Delete “\(deleting?.displayTitle ?? "")” permanently?",
                            isPresented: .init(get: { deleting != nil }, set: { if !$0 { deleting = nil } }), presenting: deleting) { entry in
            Button("Delete Permanently", role: .destructive) { Task { await app.setProcessing(.deletePermanently, entry: entry.id) } }
        } message: { _ in
            Text("Its archived audio, transcript and contributed voice samples are deleted from the vault. This cannot be undone. Originals on the recorder are not affected.")
        }
    }
}
