//! UI tests. These run the real `eframe::App` headlessly via `egui_kittest`
//! with a fake clipboard and interact through accessible labels. Core
//! state transitions are covered by unit tests in `src/core`; these check
//! that the important flows are wired to widgets. No GPU needed.

use egui::accesskit::Role;
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use switchboard::SwitchboardApp;
use switchboard::adapters::clipboard::FakeClipboard;

fn harness_with(clipboard: FakeClipboard) -> Harness<'static, SwitchboardApp> {
    Harness::builder()
        .with_size(egui::vec2(480.0, 320.0))
        .build_eframe(move |_cc| SwitchboardApp::with_services(Box::new(clipboard)))
}

fn harness() -> Harness<'static, SwitchboardApp> {
    harness_with(FakeClipboard::default())
}

#[test]
fn shows_heading_and_default_greeting() {
    let harness = harness();
    harness.get_by_label("Switchboard");
    harness.get_by_label("Hello, World!");
    harness.get_by_label("Greeted 0 times");
}

#[test]
fn greet_button_increments_counter() {
    let mut harness = harness();

    // Two steps per click: one frame processes the click, the next renders
    // the result.
    harness.get_by_label("Greet").click();
    harness.run_steps(2);
    harness.get_by_label("Greet").click();
    harness.run_steps(2);

    harness.get_by_label("Greeted 2 times");
    assert_eq!(harness.state().core().greet_count(), 2);
}

#[test]
fn typing_a_name_updates_greeting() {
    let mut harness = harness();

    let input = harness.get_by_role_and_label(Role::TextInput, "Name");
    // `type_text` only sends a Text event; the widget must have focus first.
    input.focus();
    input.type_text("Ada");
    harness.run_steps(2);

    harness.get_by_label("Hello, Ada!");
    assert_eq!(harness.state().core().name(), "Ada");
}

#[test]
fn copy_greeting_toasts_on_success() {
    let mut harness = harness();
    harness.get_by_label("Copy greeting").click();
    harness.run_steps(2);
    harness.get_by_label("Greeting copied");
}

#[test]
fn copy_greeting_reports_clipboard_failure() {
    let mut harness = harness_with(FakeClipboard {
        fail_with: Some("no display".into()),
        ..Default::default()
    });
    harness.get_by_label("Copy greeting").click();
    harness.run_steps(2);
    harness.get_by_label("Copy failed: no display");
}
