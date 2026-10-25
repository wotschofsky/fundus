import SwiftUI

@main
struct TranscriptVaultApp: App {
    @State private var app = AppModel()

    var body: some Scene {
        Window("Transcript Vault", id: "main") {
            RootView()
                .environment(app)
                .frame(minWidth: 780, minHeight: 480)
        }
        .defaultSize(width: 1060, height: 700)
        .commands { AppCommands(app: app) }

        Settings {
            SettingsView()
                .environment(app)
        }
    }
}

private struct AppCommands: Commands {
    let app: AppModel

    var body: some Commands {
        CommandGroup(replacing: .newItem) {
            Button("Import Audio Files…") { app.chooseAudioFiles() }
                .keyboardShortcut("o")
                .disabled(!app.canImport)
            Button("Choose Source Folder…") { app.chooseSourceFolder() }
                .keyboardShortcut("o", modifiers: [.command, .shift])
        }
        CommandMenu("Recorder") {
            Button("Rescan") { app.sources.refresh(); if let folder = app.sources.folder { app.sources.chooseFolder(folder) } }
                .keyboardShortcut("r")
            Button("Eject All Mics") { Task { await app.ejectRecorders() } }
                .keyboardShortcut("e")
                .disabled(!app.canEject)
        }
        CommandGroup(after: .sidebar) {
            Button("Import") { app.selection = .importer }
                .keyboardShortcut("1")
            Button("Trash") { app.selection = .trash }
                .keyboardShortcut("2")
            Divider()
            Button("Reload Library") { Task { await app.refreshAll() } }
                .keyboardShortcut("r", modifiers: [.command, .shift])
                .disabled(!app.isConnected)
            Button("Open Vault in Browser") { app.openInBrowser() }
                .disabled(app.server.isEmpty)
        }
    }
}
