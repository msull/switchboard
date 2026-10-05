# Switchboard patches to the vendored `egui_term`

Upstream: https://github.com/Harzu/egui_term at `31bbc7ab` (2026-07-30).
Besides `Cargo.toml` (egui 0.35 to 0.36) and a `cargo fmt` pass, these
source changes were made. Re-apply them when updating the vendored copy.

1. **Keyboard input follows focus, not the pointer** (`src/view.rs`,
   `TerminalView::process_input`). Upstream returned early unless the
   widget both had focus *and* contained the pointer, so typing was
   dropped whenever the mouse rested elsewhere (found in spike 04). Now
   keyboard events (`Text`, `Key`, `Copy`, `Paste`) need only focus, and
   pointer events (`MouseWheel`, `PointerButton`, `PointerMoved`) need
   only the pointer.

2. **`BackendSettings.env`** (`src/backend/settings.rs`,
   `src/backend/mod.rs`). Upstream never sets `TERM`, so the child
   inherited whatever the parent had (`xterm-ghostty` when launched from
   Ghostty). The new `env` map is passed through to
   `alacritty_terminal::tty::Options::env`; Switchboard sets
   `TERM=xterm-256color` and `COLORTERM=truecolor`.

3. **The event subscription thread ends with the terminal**
   (`src/backend/mod.rs`, `TerminalBackend::new`). Upstream looped
   forever on `recv()`, so once the backend was dropped and its channel
   closed the thread spun at full speed and never released what it
   held; one per terminal ever opened. Now a closed channel on either
   side ends the thread.

4. **A paste honours bracketed-paste mode** (`src/view.rs`,
   `paste_bytes` and the paste arm of `process_keyboard_event`).
   Upstream wrote a paste raw, so each newline reached the application
   as an Enter and Claude Code submitted the first line and lost the
   rest. Now, when the terminal mode has `BRACKETED_PASTE` (DECSET
   2004), the text is framed in `ESC[200~` .. `ESC[201~`, line breaks
   inside are LF, and ESC is dropped so the text cannot close the
   bracket early and run the rest as keystrokes. Without the mode, each
   line break becomes CR, the byte Enter sends. Under Switchboard the
   widget's pty runs a tmux client, and tmux enables 2004 at its client
   on every attach, so the bracketed branch is the one taken; tmux then
   strips the markers for a pane that did not ask for them, which is
   what keeps a plain shell pane running pasted lines one by one. The
   ^V hotfix for a plain paste off macOS is unchanged. The tests run
   with `cargo test --locked -p egui_term --lib` from the repo root.
