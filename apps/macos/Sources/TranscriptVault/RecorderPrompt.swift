import AppKit
import SwiftUI

/// The small window that appears when a DJI mic is plugged in: its recordings, each with its own
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
        let host = NSHostingController(rootView: RecorderPromptView(close: { [weak panel] in panel?.close() }).environment(app))
        host.sizingOptions = .preferredContentSize
        // The hidden title bar would otherwise add its height above the header.
        host.safeAreaRegions = []
        panel.contentViewController = host
        panel.center()
        panel.orderFrontRegardless()
    }
}

private struct RecorderPromptView: View {
    let close: () -> Void
    @Environment(AppModel.self) private var app

    /// Newest first, imported ones too; fragments too short to be worth importing are left out.
    private var listed: [RecordingSession] {
        guard app.sources.showsRecorders else { return [] }
        return app.sessions.sessions.filter { $0.duration >= SourceSessions.worthImporting }.sorted { $0.start > $1.start }
    }
    private var proposed: [RecordingSession] { app.sources.showsRecorders ? app.sessions.proposed(server: app.uploadServer) : [] }
    private func length(_ session: RecordingSession) -> TimeInterval { app.sessions.trim(for: session)?.duration ?? session.duration }

    var body: some View {
        VStack(alignment: .leading, spacing: 16) {
            header
            recordings
            HStack {
                if app.uploadServer.isEmpty {
                    Label("Connect a vault in Settings", systemImage: "exclamationmark.circle")
                } else {
                    Label(app.uploadVaultName, systemImage: "books.vertical")
                }
                Spacer(minLength: 8)
                let zone = TimeZone(identifier: app.transfer.timezone) ?? .current
                Label("Recorder clock \(Format.utcOffset(zone))", systemImage: "clock")
                    .help("Recording times are read in \(zone.identifier). Change it in Settings.")
            }
            .font(.caption)
            .foregroundStyle(.secondary)
            .lineLimit(1)
            .truncationMode(.middle)
        }
        .padding(24)
        .frame(width: 400)
        .overlay(alignment: .topTrailing) {
            Button(action: close) {
                Image(systemName: "xmark")
                    .font(.system(size: 9, weight: .bold))
                    .foregroundStyle(.secondary)
                    .frame(width: 18, height: 18)
                    .background(.quaternary, in: Circle())
            }
            .buttonStyle(.plain)
            .focusEffectDisabled()
            .keyboardShortcut(.cancelAction)
            // On the same lines as the header's top and the list's edge.
            .padding(24)
            .help("Close")
            .accessibilityLabel("Close")
        }
        .onChange(of: app.sources.shownFiles) { _, files in app.sessions.update(files, server: app.uploadServer) }
        // Unplugged: nothing left to show.
        .onChange(of: listed.isEmpty) { _, empty in if empty { close() } }
    }

    private var header: some View {
        HStack(spacing: 16) {
            // App icons have a transparent margin: drawn larger, the icon itself fills the 48 points
            // laid out for it, level with the close button.
            Image(nsImage: NSApp.applicationIconImage)
                .resizable()
                .frame(width: 60, height: 60)
                .padding(-6)
                .accessibilityHidden(true)
            VStack(alignment: .leading, spacing: 2) {
                Text("DJI Mic connected")
                    .font(.title3.weight(.semibold))
                Text(proposed.isEmpty ? "Everything is imported"
                     : "\(proposed.count == 1 ? "1 new recording" : "\(proposed.count) new recordings") · \(Format.duration(proposed.map(length).reduce(0, +)))")
                    .font(.callout)
                    .foregroundStyle(.secondary)
                    .monospacedDigit()
            }
        }
    }

    private var recordings: some View {
        let showsMic = Set(listed.map(\.transmitter)).count > 1
        return ScrollView {
            VStack(spacing: 0) {
                ForEach(Array(listed.enumerated()), id: \.element.id) { index, session in
                    if index > 0 { Divider().padding(.leading, 12) }
                    row(session, showsMic: showsMic)
                }
            }
        }
        .frame(maxHeight: 330)
        .background(Color.primary.opacity(0.04), in: RoundedRectangle(cornerRadius: 12))
        .overlay(RoundedRectangle(cornerRadius: 12).strokeBorder(Color.primary.opacity(0.07)))
        .clipShape(RoundedRectangle(cornerRadius: 12))
    }

    private func row(_ session: RecordingSession, showsMic: Bool) -> some View {
        let trim = app.sessions.trim(for: session)
        let start = session.start(trimmedBy: trim), end = session.end(trimmedBy: trim)
        let record = app.sessions.record(for: session, server: app.uploadServer)
        let partly = record == nil && app.sessions.partlyImported(session, server: app.uploadServer)
        return HStack(spacing: 12) {
            VStack(alignment: .leading, spacing: 2) {
                Text(Format.timeRange(start, end))
                    .fontWeight(.medium)
                    .foregroundStyle(record?.stage == "complete" ? .secondary : .primary)
                Text([day(start), Format.duration(length(session)), showsMic ? session.transmitter : nil, partly ? "partly imported" : nil]
                    .compactMap { $0 }.joined(separator: " · "))
                    .font(.caption)
                    .foregroundStyle(.secondary)
            }
            Spacer(minLength: 8)
            status(session, record: record)
        }
        .monospacedDigit()
        .padding(.horizontal, 12)
        .padding(.vertical, 8)
    }

    @ViewBuilder private func status(_ session: RecordingSession, record: SessionImportRecord?) -> some View {
        if let record {
            if record.stage == "complete" {
                Label("Imported", systemImage: "checkmark.circle.fill")
                    .foregroundStyle(.green)
            } else {
                HStack(spacing: 6) {
                    if !["failed", "paused"].contains(record.stage) { ProgressView().controlSize(.mini) }
                    Text(Entry.describe(stage: record.stage, activeStage: record.activeStage, pauseRequested: record.pauseRequested))
                }
                .foregroundStyle(.secondary)
                .lineLimit(1)
            }
        } else if app.waitingSessions.contains(where: { $0.id == session.id }) {
            Label("Waiting", systemImage: "hourglass")
                .foregroundStyle(.secondary)
                .help("Imports when the current upload is done")
        } else {
            Button("Import", systemImage: "square.and.arrow.down") { app.importNext(session) }
                .labelStyle(.iconOnly)
                .buttonStyle(.bordered)
                .buttonBorderShape(.circle)
                .disabled(app.uploadServer.isEmpty)
                .help("Import into \(app.uploadVaultName)")
        }
    }

    /// `Today`, `Yesterday`, or `Oct 24`.
    private func day(_ date: Date) -> String {
        Calendar.current.isDateInToday(date) ? "Today" : Calendar.current.isDateInYesterday(date) ? "Yesterday" : Format.day(date)
    }
}
