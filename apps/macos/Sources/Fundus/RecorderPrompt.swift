import AppKit
import SwiftUI

/// The small window that appears when a DJI mic is plugged in: its recordings, each with its own
/// Import button. A floating panel instead of a SwiftUI scene, so it opens whatever the main window
/// is doing, and it does not take the keyboard from the app in front.
@MainActor
final class RecorderPrompt {
    private var panel: NSPanel?

    func close() { panel?.close() }

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
        // A mic coming back while the window is open leaves it where it was put.
        if !panel.isVisible { panel.center() }
        panel.orderFrontRegardless()
    }
}

private struct RecorderPromptView: View {
    let close: () -> Void
    @Environment(AppModel.self) private var app

    /// Everything on the mic, newest first, imported ones too.
    private var listed: [RecordingSession] {
        app.sources.showsRecorders ? app.sessions.sessions.sorted { $0.start > $1.start } : []
    }
    private var proposed: [RecordingSession] { app.sources.showsRecorders ? app.sessions.proposed(server: app.uploadServer) : [] }
    private func length(_ session: RecordingSession) -> TimeInterval { app.sessions.trim(for: session)?.duration ?? session.duration }

    /// What the window has to show; anything that ends a wait for the mic changes it.
    private enum Shown: Equatable { case mic, folder, scanning, nothing }
    private var shown: Shown {
        app.sources.prefersChosenFolder ? .folder
            : app.sources.showsRecorders ? .mic
            : app.sources.isScanning ? .scanning : .nothing
    }

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
        // A folder chosen in the main window instead of the mic closes it. With no mic shown it waits:
        // taking one mic out of the case can make the others drop out for a moment too, so they get
        // time to mount again. A scan or a returning mic ends the wait.
        // ponytail: fixed grace; a mic that takes longer to come back reopens the window on arrival. Make
        // it a setting if some mic needs longer.
        .task(id: shown) {
            switch shown {
            case .folder: close()
            case .nothing:
                try? await Task.sleep(for: .seconds(10))
                // Asked again: a mic can start mounting after the wait ends but before SwiftUI cancels it.
                if !Task.isCancelled, shown == .nothing { close() }
            case .mic, .scanning: break
            }
        }
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
                Text(app.sources.showsRecorders ? "DJI Mic connected" : "DJI Mic")
                    .font(.title3.weight(.semibold))
                // With no mic shown, nothing is known about its recordings.
                Text(!app.sources.showsRecorders ? "Looking for the mic…"
                     : proposed.isEmpty ? "Nothing new to import"
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
                if listed.isEmpty {
                    Text(!app.sources.showsRecorders ? "No mic with recordings found yet."
                         : app.sources.isReading ? "Reading recordings…" : "No recordings on the mic.")
                        .foregroundStyle(.secondary)
                        .frame(maxWidth: .infinity, alignment: .leading)
                        .padding(12)
                }
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
            status(session, record: record, partly: partly)
        }
        .monospacedDigit()
        .padding(.horizontal, 12)
        .padding(.vertical, 8)
        .contentShape(Rectangle())
        .contextMenu { SessionBoundaryMenu(session: session) }
    }

    @ViewBuilder private func status(_ session: RecordingSession, record: SessionImportRecord?, partly: Bool) -> some View {
        if let record, record.stage == "complete" {
            Label("Imported", systemImage: "checkmark.circle.fill")
                .foregroundStyle(.green)
        } else if let record, record.stage != "uploading" {
            HStack(spacing: 6) {
                if !["failed", "paused"].contains(record.stage) { ProgressView().controlSize(.mini) }
                Text(Entry.describe(stage: record.stage, activeStage: record.activeStage, pauseRequested: record.pauseRequested))
            }
            .foregroundStyle(.secondary)
            .lineLimit(1)
        } else if record != nil || app.isQueued(session) {
            // Where the Import button was, from the moment it is pressed.
            UploadCircle(record: record)
                .frame(width: 24, height: 24)
        } else {
            Button("Import", systemImage: "square.and.arrow.down") { app.importNext(session) }
                .labelStyle(.iconOnly)
                .buttonStyle(.bordered)
                .buttonBorderShape(.circle)
                // A partly imported row goes up again only from the Import screen, as a deliberate choice.
                .disabled(app.uploadServer.isEmpty || partly)
                .help(!partly ? "Import into \(app.uploadVaultName)"
                      : "Some or all of this is already in \(app.uploadVaultName), or on its way there. "
                        + (session.recordings.count > 1 && !app.sessions.boundaryLocks(session, server: app.uploadServer, isQueued: app.isQueued).split
                           ? "Split it, or use the Import screen to import it anyway." : "Use the Import screen to import it anyway."))
        }
    }

    /// `Today`, `Yesterday`, or `Oct 24`.
    private func day(_ date: Date) -> String {
        Calendar.current.isDateInToday(date) ? "Today" : Calendar.current.isDateInYesterday(date) ? "Yesterday" : Format.day(date)
    }
}
