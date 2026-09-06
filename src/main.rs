//! Desktop entry point.

use switchboard::SwitchboardApp;
use switchboard::adapters::fakes::{FakeAgents, FakeEvents, FakeHost, FakeOpener, MemoryStore};
use switchboard::app::Services;

fn main() -> eframe::Result {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("switchboard=info"))
        .init();

    // Real adapters are wired in once the work items land; until then the
    // launcher runs on fakes so the window opens.
    let services = Services {
        store: Box::new(MemoryStore::default()),
        host: Box::new(FakeHost::default()),
        events: Box::new(FakeEvents::default()),
        agents: Box::new(FakeAgents::default()),
        opener: Box::new(FakeOpener::default()),
    };

    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([1100.0, 720.0])
            .with_min_inner_size([600.0, 400.0]),
        ..Default::default()
    };

    eframe::run_native(
        "Switchboard",
        options,
        Box::new(move |_cc| Ok(Box::new(SwitchboardApp::with_services(services)))),
    )
}
