//! Headless keyboard test: events go straight to the egui widget in-process,
//! never through the OS, so nothing else on the machine can receive them.
//! Runs a real `$SHELL` in a pty and reads the grid back.

use std::time::{Duration, Instant};

use egui::{Event, Key, Modifiers};
use egui_kittest::Harness;
use terminal_view_spike::SpikeApp;

fn wait_for(harness: &mut Harness<'_, SpikeApp>, needle: &str) -> String {
    let start = Instant::now();
    loop {
        harness.step();
        let text = harness.state().screen_text();
        if text.contains(needle) {
            return text;
        }
        assert!(
            start.elapsed() < Duration::from_secs(8),
            "timed out waiting for {needle:?}; screen:\n{text}"
        );
        std::thread::sleep(Duration::from_millis(30));
    }
}

fn type_line(harness: &mut Harness<'_, SpikeApp>, s: &str) {
    harness.event(Event::Text(s.to_owned()));
    harness.key_press(Key::Enter);
}

#[test]
fn shell_echo_arrows_ctrl_c_paste() {
    // A bare, prompt-free shell keeps the grid predictable.
    let mut harness = Harness::builder()
        .with_size(egui::Vec2::new(900.0, 500.0))
        .build_eframe(|cc| {
            SpikeApp::new(cc.egui_ctx.clone(), Some("PS1='$ ' exec zsh -f".into()))
        });
    wait_for(&mut harness, "$");
    // egui_term ignores keyboard events unless the pointer is over the widget
    // (view.rs `process_input`), so park the pointer inside it first.
    harness.event(Event::PointerMoved(egui::pos2(400.0, 300.0)));
    harness.step();

    // Typed text and Enter reach the shell and its output comes back.
    let t0 = Instant::now();
    type_line(&mut harness, "echo one-two-three");
    wait_for(&mut harness, "one-two-three\n");
    eprintln!("echo round trip: {:?}", t0.elapsed());

    // Left arrow moves the cursor: "eco AB" + 4x Left + "h" -> "echo AB".
    harness.event(Event::Text("eco AB".into()));
    for _ in 0..4 {
        harness.key_press(Key::ArrowLeft);
    }
    harness.event(Event::Text("h".into()));
    harness.key_press(Key::Enter);
    wait_for(&mut harness, "\nAB\n");

    // Ctrl-C interrupts a running command.
    type_line(&mut harness, "sleep 30 && echo NOT-PRINTED");
    std::thread::sleep(Duration::from_millis(300));
    harness.key_press_modifiers(Modifiers::CTRL, Key::C);
    type_line(&mut harness, "echo after-interrupt");
    let text = wait_for(&mut harness, "after-interrupt\n");
    assert!(text.contains("^C") && !text.contains("\nNOT-PRINTED"), "Ctrl-C did not interrupt sleep; screen:\n{text}");

    // Paste arrives as a single Paste event and is written to the pty.
    harness.event(Event::Paste("echo pasted-text\n".into()));
    let text = wait_for(&mut harness, "\npasted-text\n");
    eprintln!("final screen:\n{text}");
}

/// Runs the real `claude` CLI in the widget with a one-word prompt, to see
/// whether its TUI draws in the egui terminal. Costs a few tokens, so it is
/// ignored by default: `cargo test -- --ignored claude_tui --nocapture`.
#[test]
#[ignore]
fn claude_tui() {
    let mut harness = Harness::builder()
        .with_size(egui::Vec2::new(1000.0, 600.0))
        // Run in a directory the user has already trusted in Claude Code, so
        // the test never answers a trust prompt on the user's behalf.
        .build_eframe(|cc| {
            SpikeApp::new(
                cc.egui_ctx.clone(),
                Some("cd /Users/sully/code_repos/personal/promptbox && exec claude".into()),
            )
        });
    harness.event(Event::PointerMoved(egui::pos2(400.0, 300.0)));
    // Claude's prompt box shows a "❯" marker once the TUI is up.
    let start = Instant::now();
    let text = loop {
        harness.step();
        let t = harness.state().screen_text();
        if t.contains('❯') && start.elapsed() > Duration::from_secs(3) {
            break t;
        }
        assert!(start.elapsed() < Duration::from_secs(30), "claude did not start:\n{t}");
        std::thread::sleep(Duration::from_millis(50));
    };
    eprintln!("=== claude start screen ===\n{text}");
    harness.event(Event::Text("Reply with exactly the single word pong".into()));
    harness.step();
    harness.key_press(Key::Enter);
    let start = Instant::now();
    let text = loop {
        harness.step();
        let t = harness.state().screen_text();
        if t.contains("⏺ pong") {
            break t;
        }
        assert!(start.elapsed() < Duration::from_secs(90), "no reply:\n{t}");
        std::thread::sleep(Duration::from_millis(100));
    };
    eprintln!("=== claude reply screen ===\n{text}");
    harness.event(Event::Text("/exit".into()));
    harness.step();
    harness.key_press(Key::Enter);
    let start = Instant::now();
    while !harness.state().exited && start.elapsed() < Duration::from_secs(10) {
        harness.step();
        std::thread::sleep(Duration::from_millis(100));
    }
    eprintln!("exited: {}", harness.state().exited);
}
