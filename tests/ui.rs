//! UI tests: the real `eframe::App` headlessly via `egui_kittest` with
//! fake adapters. Extended by the UI work item.

use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use switchboard::SwitchboardApp;
use switchboard::adapters::fakes::{FakeAgents, FakeEvents, FakeHost, FakeOpener, MemoryStore};
use switchboard::app::Services;

fn harness() -> Harness<'static, SwitchboardApp> {
    let services = Services {
        store: Box::new(MemoryStore::default()),
        host: Box::new(FakeHost::default()),
        events: Box::new(FakeEvents::default()),
        agents: Box::new(FakeAgents::default()),
        opener: Box::new(FakeOpener::default()),
    };
    Harness::builder()
        .with_size(egui::vec2(1100.0, 720.0))
        .build_eframe(move |_cc| SwitchboardApp::with_services(services))
}

#[test]
fn shows_the_switchboard_heading() {
    let harness = harness();
    harness.get_by_label("Switchboard");
}
