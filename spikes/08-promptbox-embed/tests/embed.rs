use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use promptbox_embed_spike::Host;

#[test]
fn prompt_box_draws_in_a_panel_and_send_reaches_the_host() {
    let mut harness = Harness::builder()
        .with_size(egui::vec2(1100.0, 800.0))
        .build_eframe(|_cc| Host::headless());
    harness.run_steps(2);
    // Host chrome and Prompt Box chrome coexist.
    harness.get_by_label("claude-agent");
    harness.get_by_label("pong");
    harness.get_by_label("Undo");
    harness.get_by_label("Send →");
    let editor = harness.get_by_label("Prompt");
    editor.focus();
    editor.type_text("fix the failing test");
    harness.run_steps(2);
    harness.get_by_label("Send →").click();
    harness.run_steps(4);
    let app = harness.state();
    assert_eq!(app.sent, vec!["fix the failing test".to_owned()]);
    assert_eq!(app.prompt.core().doc().rendered(), "", "the editor cleared after Send");
    // The prompt box is confined to its panel: its editor sits below the conversation.
    harness.run_steps(1);
    let conv = harness.get_by_label("pong").rect().bottom();
    let editor_top = harness.get_by_label("Prompt").rect().top();
    assert!(editor_top > conv, "editor {editor_top} above conversation {conv}");
}
