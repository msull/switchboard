# Spike 8: Can Prompt Box be embedded as the agent session's message box?

**Answer: yes, and it is cheap.** Prompt Box (`~/code_repos/personal/promptbox`,
commit `6532a59`) is Rust on the same egui/eframe 0.36 as Switchboard, is a
library crate with its drawing behind one function, and takes its clipboard
and history adapters by trait. Drawn into a bottom panel of a stand-in host
with **no change to promptbox**, it shows its header row, editor, AI row,
and footer inside the panel, and a `Clipboard` adapter that keeps the text
turns Send into a hand-off to the host instead of a copy.

Run on 2026-09-15. Files: `src/lib.rs` (the host), `tests/embed.rs`
(headless proof), `embed.png` (the window).

## What was tried

```
$ cargo build            # promptbox by path, whisper.cpp via cmake included
Finished `dev` profile in 33.9s    # shared target dir already had eframe
$ cargo test
test prompt_box_draws_in_a_panel_and_send_reaches_the_host ... ok
```

The host draws a header panel, a fake conversation in the centre, and
`promptbox::ui::draw(&mut app, ui)` inside `Panel::bottom("sb-message")`
of 320 px. The test types into the editor (label "Prompt"), clicks
"Send →", and checks the text arrived at the host and the editor cleared.
The editor sits below the conversation, so Prompt Box's own panels nest
inside the host's panel rather than taking over the window.

`embed.png` is the live window: Prompt Box's top bar (status, project
picker, Start listening, Debug, Dock, CC, Pin, settings), the editor,
the AI row, the footer buttons, all inside the panel.

## How it plugs in

- `PromptBoxApp::with_services(clipboard, history)` builds the app with
  any `Clipboard` and `HistoryStore`. Send writes the prompt to the
  clipboard port and saves history, then clears; a `Clipboard` adapter
  that hands the text to Switchboard is the whole bridge. The spike's
  `PaneSink` does that; Switchboard's version would dispatch `SendInput`.
- Per frame the host calls `app.pump()` (workers, recognizer, mic) and
  passes the returned delay to `request_repaint_after`, as Prompt Box's
  own `eframe::App::ui` does.
- `promptbox::ui::install_symbol_font(ctx)` once at creation.
- Prompt Box's `apply_window_level` (theme, always-on-top) is only called
  from its own `eframe::App` impl, so embedding skips it: no viewport
  commands, and Switchboard's theme stays in charge.

## What the real thing needs from promptbox ("updating it as needed")

1. **An embedded mode** that hides what belongs to a window of its own:
   Dock, Pin, the theme setting, the Debug menu, and probably the
   project picker. A `PromptBoxApp::set_embedded(true)` read by the top
   bar is enough.
2. **A sink port instead of the clipboard trick.** Cleaner than abusing
   `Clipboard`: a `PromptSink` trait (or a `Typist` the host can set)
   so Send delivers to the host and the clipboard is untouched, which
   is what "without going through copy" asks. Today `typist` is set
   only in `new()`; `with_services` leaves the fake.
3. **A way to set the text**: `AppAction::ReplaceText` exists, so priming
   a cloned session's prompt or Switchboard's per-session drafts is
   already possible; a `load_text` convenience would keep the undo
   history sensible.
4. **Shortcut scope.** `handle_shortcuts` consumes ⌘Return, ⌘Z, ⌘L and
   friends from the whole context every frame. Embedded, that should
   happen only while the editor has focus, or Switchboard's own keys
   (⌘T, ⌘R, Escape) and text fields would lose them.
5. **Panel ids.** Prompt Box uses `Panel::top("top")` and
   `Panel::bottom("bottom")`, `"ai-row"`, `"notifications"`. They nest
   inside the host panel today, but they are global ids; salting them
   avoids a clash with any Switchboard panel of the same name.

## What Switchboard needs

- **Dependency.** The repo is public (`github.com/msull/promptbox`), so a
  git dependency pinned to a rev works with `--locked` in CI; a path
  dependency would not exist on the runner. Local development can use
  `[patch]` to point at the sibling checkout.
- **Build.** whisper.cpp builds through cmake: CI needs `cmake` and
  `libasound2-dev` (cpal) on the runner (the workflow comment already
  anticipates this). Build cost here was 34 s incremental; a cold CI
  build adds a few minutes and `Swatinem/rust-cache` keeps it.
- **One Prompt Box, many sessions.** The whisper model (141 MB on disk)
  and the microphone are process-wide, so there should be one embedded
  `PromptBoxApp`, not one per session. Switching sessions swaps the
  text in and out (`ReplaceText` / `input_drafts`) rather than making
  a new app. Its data directory (`~/Library/Application
  Support/promptbox`: model, settings, history, tools) is shared with
  the standalone app through `FileStore::default_dir()`, which is what
  you want: one OpenAI key, one model download, one tool folder.
- **Session-view layout.** The message panel becomes a taller,
  resizable bottom panel holding Prompt Box; the existing drop-a-file
  and Escape-interrupts behaviours move to the host around it.
- **Permissions.** Microphone (and Accessibility, if paste is ever used)
  are granted to the signed Switchboard bundle, not to Prompt Box.

## Risks seen

- Prompt Box's `CentralPanel::default()` inside the host panel worked,
  but it is the one place egui 0.36 layouts are easy to get wrong; keep
  the kittest test from this spike in Switchboard's UI tests.
- Two apps' worth of per-frame work in one window: `pump()` requests
  repaints while listening, which is fine, but the recognizer thread
  and mic must be stopped when the session view closes.
