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
        // making a second; the launch window gets the same value. It
        // is the one window a launch presents: the others are never
        // restored, so the app always opens to the browser, and the
        // app itself opens nothing in the first seconds (`LaunchGrace`).
        WindowGroup("Switchboard", id: MainWindow.id, for: Int.self) { _ in
            RootView(model: model, openOnStart: Self.openFromEnvironment)
                .task { await model.boot(invite: Self.inviteFromEnvironment) }
        } defaultValue: {
            MainWindow.value
        }
        .defaultLaunchBehavior(.presented)
        // A card window per request: the system places it, and brings
        // it forward when the same request is opened again (§7.2). It
        // opens in portrait, a page rather than a screen: a
        // conversation is read down, and a narrow card leaves room
        // for the next one beside it.
        WindowGroup(for: CardWindowRequest.self) { $request in
            if let request {
                CardWindowView(model: model, request: request)
            }
        }
        .defaultSize(width: 580, height: 900)
        .restorationBehavior(.disabled)
        // A file window per request: a file the agent named, opened
        // from a card by a tap on its name.
        WindowGroup(for: FileWindowRequest.self) { $request in
            if let request {
                FileWindowView(model: model, request: request)
            }
        }
        .defaultSize(width: 720, height: 640)
        .restorationBehavior(.disabled)
        // The prompt window, one per app like the main one: the whole
        // draft of the session whose input is active, placed once and
        // left where it is while the cards come and go.
        WindowGroup("Prompt", id: PromptWindow.id, for: PromptWindow.Key.self) { _ in
            PromptWindowView(model: model)
        } defaultValue: {
            PromptWindow.value
        }
        .defaultSize(width: 900, height: 420)
        .restorationBehavior(.disabled)
        // The keys panel, one per app: sized to its keys and placed
        // where the system puts the virtual keyboard, low and close,
        // so it is reached by touch like the keyboard it stands in for.
        WindowGroup("Keys", id: KeysWindow.id, for: KeysWindow.Key.self) { _ in
            KeysPanelView(model: model)
        } defaultValue: {
            KeysWindow.value
        }
        .windowResizability(.contentSize)
        .defaultWindowPlacement { _, _ in WindowPlacement(.utilityPanel) }
        .restorationBehavior(.disabled)
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
