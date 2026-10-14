// The visionOS entry: the main browsing window and, from B2 on, the
// card windows. Everything it shows lives in SwitchboardUI so the iPad
// shell (Milestone D) draws the same views; this file only declares
// the scenes and where the registry lives.
import SwiftUI
import SwitchboardClientCore
import SwitchboardUI

@main
struct SwitchboardVisionApp: App {
    @State private var model = BrowserModel(
        manager: SwitchboardVisionApp.makeManager(),
        presentationStore: FilePresentationStore(url: SwitchboardVisionApp.support.appendingPathComponent("presentation.json")))

    init() {
        _ = LaunchGrace.launchedAt
        LaunchLog.file = Self.support.appendingPathComponent("launch.log")
        LaunchLog.note("launch pid \(ProcessInfo.processInfo.processIdentifier)")
    }

    var body: some Scene {
        // The main window carries a fixed value so that opening it
        // from a card window brings the open one forward rather than
        // making a second; the launch window gets the same value. The
        // system presents it at every launch, beside whatever it
        // restores: the app must not open it itself in the first
        // seconds (`LaunchGrace`).
        WindowGroup("Switchboard", id: MainWindow.id, for: Int.self) { _ in
            RootView(model: model, openOnStart: Self.openFromEnvironment)
                .task { await model.boot(invite: Self.inviteFromEnvironment) }
        } defaultValue: {
            MainWindow.value
        }
        .defaultLaunchBehavior(.presented)
        // A card window per request: the system places it, and brings
        // it forward when the same request is opened again (§7.2). It
        // boots the model too, since a relaunch may restore only card
        // windows.
        WindowGroup(for: CardWindowRequest.self) { $request in
            if let request {
                CardWindowView(model: model, request: request)
                    .task { await model.boot(invite: Self.inviteFromEnvironment) }
            }
        }
        .defaultSize(width: 760, height: 560)
        // The prompt window, one per app like the main one: the whole
        // draft of the session whose input is active, placed once and
        // left where it is while the cards come and go.
        WindowGroup("Prompt", id: PromptWindow.id, for: PromptWindow.Key.self) { _ in
            PromptWindowView(model: model)
                .task { await model.boot(invite: Self.inviteFromEnvironment) }
        } defaultValue: {
            PromptWindow.value
        }
        .defaultSize(width: 900, height: 420)
    }

    /// A dev aid: `SWITCHBOARD_INVITE` holds invites' JSON (the text of
    /// each host's QR code, one per line) and the app pairs with them
    /// on launch, so a simulator run needs no typing (`simctl launch`
    /// passes it as `SIMCTL_CHILD_SWITCHBOARD_INVITE`).
    static var inviteFromEnvironment: String? {
        ProcessInfo.processInfo.environment["SWITCHBOARD_INVITE"]
    }

    /// A dev aid: `SWITCHBOARD_OPEN` lists card windows to open on
    /// launch, `<record id>[:<mode>]` separated by commas.
    static var openFromEnvironment: [String] {
        (ProcessInfo.processInfo.environment["SWITCHBOARD_OPEN"] ?? "")
            .split(separator: ",").map { $0.trimmingCharacters(in: .whitespaces) }
    }

    /// The app's container directory, made on first use.
    static var support: URL {
        let support = FileManager.default.urls(for: .applicationSupportDirectory, in: .userDomainMask)[0]
        try? FileManager.default.createDirectory(at: support, withIntermediateDirectories: true)
        return support
    }

    /// The known servers in the app's container, the token and pin in
    /// the Keychain; nothing a host sent is ever written to disk.
    static func makeManager() -> ConnectionManager {
        return ConnectionManager(
            registry: FileRegistry(url: support.appendingPathComponent("servers.json")),
            credentials: KeychainCredentials(),
            deviceName: "Apple Vision Pro")
    }
}
