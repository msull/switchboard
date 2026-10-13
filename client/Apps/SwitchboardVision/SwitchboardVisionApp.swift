// The visionOS entry: the main browsing window and, from B2 on, the
// card windows. Everything it shows lives in SwitchboardUI so the iPad
// shell (Milestone D) draws the same views; this file only declares
// the scenes and where the registry lives.
import SwiftUI
import SwitchboardClientCore
import SwitchboardUI

@main
struct SwitchboardVisionApp: App {
    @State private var model = BrowserModel(manager: SwitchboardVisionApp.makeManager())

    var body: some Scene {
        WindowGroup("Switchboard") {
            RootView(model: model)
                .task { await model.boot(invite: Self.inviteFromEnvironment) }
        }
    }

    /// A dev aid: `SWITCHBOARD_INVITE` holds an invite's JSON (the text
    /// of the host's QR code) and the app pairs with it on launch, so a
    /// simulator run needs no typing (`simctl launch` passes it as
    /// `SIMCTL_CHILD_SWITCHBOARD_INVITE`).
    static var inviteFromEnvironment: String? {
        ProcessInfo.processInfo.environment["SWITCHBOARD_INVITE"]
    }

    /// The known servers in the app's container, the token and pin in
    /// the Keychain; nothing a host sent is ever written to disk.
    static func makeManager() -> ConnectionManager {
        let support = FileManager.default.urls(for: .applicationSupportDirectory, in: .userDomainMask)[0]
        try? FileManager.default.createDirectory(at: support, withIntermediateDirectories: true)
        return ConnectionManager(
            registry: FileRegistry(url: support.appendingPathComponent("servers.json")),
            credentials: KeychainCredentials(),
            deviceName: "Apple Vision Pro")
    }
}
