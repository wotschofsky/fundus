import AppKit
import SwiftUI

/// The small window that appears when a DJI mic is plugged in: its new recordings, ticked, and one
/// Import button. A floating panel instead of a SwiftUI scene, so it opens whatever the main window
/// is doing, and it does not take the keyboard from the app in front.
@MainActor
final class RecorderPrompt {
    private var panel: NSPanel?

    func show(_ app: AppModel) {
        let panel = self.panel ?? NSPanel(contentRect: .zero, styleMask: [.titled, .closable, .nonactivatingPanel], backing: .buffered, defer: true)
        self.panel = panel
        panel.title = "DJI Mic Connected"
        panel.level = .floating
        panel.hidesOnDeactivate = false
        panel.isReleasedWhenClosed = false
        // A fresh view each time, so recordings unticked last time are offered again.
        let host = NSHostingController(rootView: RecorderPromptView(close: { [weak panel] in panel?.close() }).environment(app))
        host.sizingOptions = .preferredContentSize
        panel.contentViewController = host
        panel.center()
        panel.orderFrontRegardless()
    }
}

private struct RecorderPromptView: View {
    let close: () -> Void
    @Environment(AppModel.self) private var app
    @State private var skipped: Set<String> = []

    private var proposed: [RecordingSession] { app.sources.showsRecorders ? app.sessions.proposed(server: app.uploadServer) : [] }
    private var chosen: [RecordingSession] { proposed.filter { !skipped.contains($0.id) } }

    var body: some View {
        VStack(alignment: .leading, spacing: 12) {
            Text(proposed.count == 1 ? "1 new recording" : "\(proposed.count) new recordings")
                .font(.headline)
            ForEach(proposed) { session in
                let trim = app.sessions.trim(for: session)
                Toggle(isOn: Binding(get: { !skipped.contains(session.id) },
                                     set: { if $0 { skipped.remove(session.id) } else { skipped.insert(session.id) } })) {
                    HStack {
                        Text(Format.sessionTitle(session.start(trimmedBy: trim), session.end(trimmedBy: trim)))
                        if Set(proposed.map(\.transmitter)).count > 1 {
                            Text(session.transmitter).foregroundStyle(.secondary)
                        }
                        Spacer()
                        Text(Format.duration(trim?.duration ?? session.duration))
                            .monospacedDigit()
                            .foregroundStyle(.secondary)
                    }
                }
                .toggleStyle(.checkbox)
            }
            Text(app.uploadServer.isEmpty ? "Connect a vault in Settings to import."
                 : app.canImport ? "Imports into \(app.uploadVaultName)." : "Importing can start once the current upload is done.")
                .font(.callout)
                .foregroundStyle(.secondary)
            HStack {
                Spacer()
                Button("Not Now", action: close)
                    .keyboardShortcut(.cancelAction)
                Button("Import \(chosen.count)") {
                    app.importSessions(chosen)
                    close()
                }
                .keyboardShortcut(.defaultAction)
                .disabled(chosen.isEmpty || !app.canImport)
            }
        }
        .padding(20)
        .frame(width: 400)
        .onChange(of: app.sources.shownFiles) { _, files in app.sessions.update(files, server: app.uploadServer) }
        // Unplugged, or imported from the main window meanwhile: nothing left to offer.
        .onChange(of: proposed.isEmpty) { _, empty in if empty { close() } }
    }
}
