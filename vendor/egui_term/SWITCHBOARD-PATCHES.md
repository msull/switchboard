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
