import ServiceManagement
import SwiftUI

@main
struct FundusApp: App {
    @NSApplicationDelegateAdaptor private var delegate: AppDelegate
    @State private var app: AppModel

    init() {
        let app = AppModel()
        _app = State(initialValue: app)
        // For as long as the app runs, with or without its window, so a plugged-in mic is noticed.
        Task { await app.start() }
        Task { await app.run() }
    }

    var body: some Scene {
        Window("Fundus", id: "main") {
            RootView()
                .environment(app)
                .frame(minWidth: 780, minHeight: 480)
                // Without its window the app lives in the menu bar only, not in the Dock.
                .onAppear { NSApp.setActivationPolicy(.regular) }
                .onDisappear { NSApp.setActivationPolicy(.accessory) }
        }
        .defaultSize(width: 1060, height: 700)
        // The app starts as a menu bar app (LSUIElement); AppDelegate opens the window unless it
        // started at login. Restoring it would bring it back at login too.
        .defaultLaunchBehavior(.suppressed)
        .restorationBehavior(.disabled)
        .commands { AppCommands(app: app) }

        Settings {
            SettingsView()
                .environment(app)
        }

        MenuBarExtra {
            MenuBarMenu()
        } label: {
            MenuBarIcon(delegate: delegate)
        }
    }
}

/// The window, with the app in the Dock while it is open.
@MainActor private func showWindow(_ openWindow: OpenWindowAction) {
    NSApp.setActivationPolicy(.regular)
    openWindow(id: "main")
    NSApp.activate()
}

/// Keeps the app in the menu bar when its window closes, and starts it there at login.
@MainActor
final class AppDelegate: NSObject, NSApplicationDelegate {
    /// Set by the menu bar icon, which exists before launching finishes.
    var showWindow: (() -> Void)?

    func applicationShouldTerminateAfterLastWindowClosed(_ sender: NSApplication) -> Bool { false }

    func applicationDidFinishLaunching(_ notification: Notification) {
        // On by default, once: switched off in the menu or in System Settings, it stays off.
        if !UserDefaults.standard.bool(forKey: "login-item-offered") {
            UserDefaults.standard.set(true, forKey: "login-item-offered")
            do { try SMAppService.mainApp.register() } catch {
                Log.app.error("Could not open at login: \(error.localizedDescription, privacy: .public)")
            }
        }
        let event = NSAppleEventManager.shared().currentAppleEvent
        let atLogin = event?.eventID == kAEOpenApplication
            && event?.paramDescriptor(forKeyword: keyAEPropData)?.enumCodeValue == keyAELaunchedAsLogInItem
        if !atLogin { showWindow?() }
    }

    /// Opened again from Finder or Spotlight while it runs in the menu bar.
    func applicationShouldHandleReopen(_ sender: NSApplication, hasVisibleWindows: Bool) -> Bool {
        if !hasVisibleWindows { showWindow?() }
        return true
    }
}

private struct MenuBarIcon: View {
    let delegate: AppDelegate
    @Environment(\.openWindow) private var openWindow

    var body: some View {
        Image(systemName: "waveform")
            .onAppear { delegate.showWindow = { showWindow(openWindow) } }
    }
}

private struct MenuBarMenu: View {
    @Environment(\.openWindow) private var openWindow
    @State private var opensAtLogin = SMAppService.mainApp.status == .enabled

    var body: some View {
        Button("Open Fundus") { showWindow(openWindow) }
        Toggle("Open at Login", isOn: Binding(get: { opensAtLogin }, set: { on in
            do { try on ? SMAppService.mainApp.register() : SMAppService.mainApp.unregister() } catch {
                Log.app.error("Could not change opening at login: \(error.localizedDescription, privacy: .public)")
            }
            opensAtLogin = SMAppService.mainApp.status == .enabled
        }))
        Divider()
        Button("Quit Fundus") { NSApp.terminate(nil) }
            .keyboardShortcut("q")
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
