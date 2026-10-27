import AppKit
import SwiftUI

/// The small window that appears when a DJI mic is plugged in: its new recordings, ticked, and one
/// Import button. A floating panel instead of a SwiftUI scene, so it opens whatever the main window
/// is doing, and it does not take the keyboard from the app in front.
@MainActor
final class RecorderPrompt {
    private var panel: NSPanel?

    func show(_ app: AppModel) {
        let panel = self.panel ?? NSPanel(contentRect: .zero, styleMask: [.titled, .fullSizeContentView, .nonactivatingPanel], backing: .buffered, defer: true)
        self.panel = panel
        panel.title = "DJI Mic Connected"
        panel.titleVisibility = .hidden
        panel.titlebarAppearsTransparent = true
        for button in [NSWindow.ButtonType.closeButton, .miniaturizeButton, .zoomButton] {
            panel.standardWindowButton(button)?.isHidden = true
        }
        panel.isMovableByWindowBackground = true
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
    @State private var greeted = false

    private var proposed: [RecordingSession] { app.sources.showsRecorders ? app.sessions.proposed(server: app.uploadServer) : [] }
    private var chosen: [RecordingSession] { proposed.filter { !skipped.contains($0.id) } }
    private func length(_ session: RecordingSession) -> TimeInterval { app.sessions.trim(for: session)?.duration ?? session.duration }

    var body: some View {
        VStack(alignment: .leading, spacing: 16) {
            header
            recordings
            footer
        }
        .padding(24)
        .frame(width: 400)
        .onChange(of: app.sources.shownFiles) { _, files in app.sessions.update(files, server: app.uploadServer) }
        // Unplugged, or imported from the main window meanwhile: nothing left to offer.
        .onChange(of: proposed.isEmpty) { _, empty in if empty { close() } }
    }

    private var header: some View {
        HStack(spacing: 16) {
            Image(systemName: "mic.fill")
                .font(.title2.weight(.semibold))
                .foregroundStyle(.white)
                .frame(width: 48, height: 48)
                .background(Color.accentColor.gradient, in: Circle())
                .symbolEffect(.bounce, value: greeted)
                .task {
                    try? await Task.sleep(for: .milliseconds(250))
                    greeted = true
                }
                .accessibilityHidden(true)
            VStack(alignment: .leading, spacing: 2) {
                Text("DJI Mic connected")
                    .font(.title3.weight(.semibold))
                Text("\(proposed.count == 1 ? "1 new recording" : "\(proposed.count) new recordings") · \(Format.duration(proposed.map(length).reduce(0, +)))")
                    .font(.callout)
                    .foregroundStyle(.secondary)
                    .monospacedDigit()
            }
        }
    }

    private var recordings: some View {
        let showsMic = Set(proposed.map(\.transmitter)).count > 1
        return VStack(spacing: 0) {
            ForEach(Array(proposed.enumerated()), id: \.element.id) { index, session in
                if index > 0 { Divider().padding(.leading, 44) }
                row(session, showsMic: showsMic)
            }
        }
        .background(Color.primary.opacity(0.04), in: RoundedRectangle(cornerRadius: 12))
        .overlay(RoundedRectangle(cornerRadius: 12).strokeBorder(Color.primary.opacity(0.07)))
    }

    private func row(_ session: RecordingSession, showsMic: Bool) -> some View {
        let isChosen = !skipped.contains(session.id)
        let trim = app.sessions.trim(for: session)
        let start = session.start(trimmedBy: trim), end = session.end(trimmedBy: trim)
        return Button {
            if isChosen { skipped.insert(session.id) } else { skipped.remove(session.id) }
        } label: {
            HStack(spacing: 12) {
                Image(systemName: isChosen ? "checkmark.circle.fill" : "circle")
                    .font(.title3)
                    .foregroundStyle(isChosen ? Color.accentColor : Color.secondary)
                    .contentTransition(.symbolEffect(.replace))
                VStack(alignment: .leading, spacing: 2) {
                    Text(Format.timeRange(start, end))
                        .fontWeight(.medium)
                    Text(showsMic ? "\(day(start)) · \(session.transmitter)" : day(start))
                        .font(.caption)
                        .foregroundStyle(.secondary)
                }
                Spacer()
                Text(Format.duration(length(session)))
                    .foregroundStyle(.secondary)
            }
            .monospacedDigit()
            .padding(.horizontal, 12)
            .padding(.vertical, 8)
            .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
        .accessibilityValue(isChosen ? "Will be imported" : "Skipped")
    }

    private var footer: some View {
        HStack(spacing: 8) {
            Group {
                if app.uploadServer.isEmpty {
                    Label("Connect a vault in Settings", systemImage: "exclamationmark.circle")
                } else if !app.canImport {
                    Label("Waiting for the current upload", systemImage: "hourglass")
                } else {
                    Label(app.uploadVaultName, systemImage: "books.vertical")
                }
            }
            .font(.caption)
            .foregroundStyle(.secondary)
            .lineLimit(1)
            .truncationMode(.middle)
            Spacer(minLength: 8)
            Button("Not Now", action: close)
                .keyboardShortcut(.cancelAction)
            Button("Import \(chosen.count)") {
                app.importSessions(chosen)
                close()
            }
            .keyboardShortcut(.defaultAction)
            .buttonStyle(.borderedProminent)
            .disabled(chosen.isEmpty || !app.canImport)
        }
        .controlSize(.large)
    }

    /// `Today`, `Yesterday`, or `Oct 24`.
    private func day(_ date: Date) -> String {
        Calendar.current.isDateInToday(date) ? "Today" : Calendar.current.isDateInYesterday(date) ? "Yesterday" : Format.day(date)
    }
}
