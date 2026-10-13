import SwiftUI
import SwitchboardClientCore

/// The main window: the unified browser and the hosts screen as tabs.
public struct RootView: View {
    @Bindable var model: BrowserModel

    public init(model: BrowserModel) {
        self.model = model
    }

    public var body: some View {
        TabView {
            NavigationStack {
                BrowserView(model: model)
                    .navigationTitle("Sessions")
            }
            .tabItem { Label("Sessions", systemImage: "rectangle.grid.2x2") }
            NavigationStack {
                ServersView(model: model)
                    .navigationTitle("Hosts")
            }
            .tabItem { Label("Hosts", systemImage: "desktopcomputer") }
        }
    }
}
