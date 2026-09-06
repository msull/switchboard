# Working in this repo

## Commands

```sh
cargo test --locked                                  # unit + headless UI tests
cargo clippy --locked --all-targets -- -D warnings   # must be clean; pedantic is on
cargo fmt --all
cargo run --locked                                   # launch the app
```

The pre-commit hook runs all three checks. Enable it with
`git config core.hooksPath .githooks` on a fresh clone. Do not commit with
`--no-verify`.

## Milestone 1 contract (read before touching code)

`docs/design.md` is the spec; "Spike 0 results", "Durable store", "Trust
boundary", and the Milestone 1 gate are binding. `spikes/*/README.md` hold
verified commands and measured numbers; copy from them rather than
re-deriving. The shared types and traits in `src/core/model.rs`,
`src/core/action.rs` (the `AppAction`/`Effect` surface), and `src/ports/`
are the contract between work items: extend them additively, never
rename or remove, and say so in the commit message when you add.

Hard rules: no keystroke injection into windows (no System Events
`keystroke`, no `osascript` typing); tmux experiments only on the
`switchboard` socket or a `switchboard-test-*` socket; never write into a
project directory's `.claude/` or `.switchboard/`; never modify
`~/.claude/settings.json`; real agent runs use `--model haiku`-class cheap
settings and a one-turn prompt.

## Driving the live app

There is no keystroke injection in this repo. To put the running app into
a state, use `SWITCHBOARD_SCRIPT=<file>` (see `src/script.rs`) with
`SWITCHBOARD_DATA_DIR` and `SWITCHBOARD_TMUX_SOCKET` pointing at test
locations, then screenshot the window (`screencapture -l <window id>`).
Sending keys into the app's own test tmux panes with `tmux -L <test
socket> send-keys` is fine; sending keys to windows is not.

## Architecture rules

- `src/core/` is deterministic: no egui, no threads, no I/O, no wall clock.
  Time comes in through `Clock`. Everything enters via `AppCore::dispatch`
  and leaves as `Effect`s.
- `src/ports/` holds traits for outside capabilities. `src/adapters/`
  implements them, and every adapter ships a fake next to it for tests.
- `src/app.rs` runs effects and feeds results back as actions. Slow work
  goes on a worker thread that sends an action back over a channel; the
  result is still one dispatch.
- `src/ui.rs` only draws and turns widget events into actions. If logic
  appears in `ui.rs`, move it to the core.
- New feature = action variant + core transition + core test, then effect
  + adapter if it needs the outside world, then the widget and a UI test.

## Testing rules

- Core tests: dispatch actions with `Clock::at(ms)` and assert on state and
  returned effects. Never sleep.
- UI tests (`tests/ui.rs`): build the app with fakes through
  `with_services`, find widgets by label, `run_steps(2)` after a click.
  `type_text` only sends a Text event; call `.focus()` on the node first.
  Size the harness tall enough that everything you click is on screen.
- Tests that need real services are `#[ignore]` in their own test file.

## egui / eframe 0.36 gotchas

Much online material and training data shows the pre-0.36 API and does not
compile. When an API does not match expectations, read the crate source
under `~/.cargo/registry/src/*/egui-0.36*` rather than guessing.

- `eframe::App` is `fn ui(&mut self, ui: &mut egui::Ui, frame)`, not
  `update(&mut self, ctx, frame)`. Get the context with `ui.ctx()`.
- Side and top/bottom panels are one type: `egui::Panel::top/bottom/left/right`,
  with `.exact_size(..)` and `.resizable(false)`. Panels take `.show(ui, ..)`.
- `egui_kittest`: `Harness::builder().with_size(..).build_eframe(|cc| ..)`.
  Import `Role` from `egui::accesskit`.

## Platform-specific code

The crate forbids `unsafe`. Native calls (AppKit via `objc2`, etc.) go in a
small adapter behind `#[cfg(target_os = "...")]`, using safe wrapper crates.
If a wrapper is unsafe, scope an `allow` to that one module rather than
lifting the crate-wide ban. macOS-only dependencies go in the
`[target.'cfg(target_os = "macos")'.dependencies]` table.

## Style

- Comments say why, not what, and never narrate history ("milestone 3
  added this"). Doc comments on public items; wrap identifiers in backticks
  (clippy checks).
- Keep the README's Layout table current when files are added or moved.
- Dev aids (autostart, fake inputs) are environment variables prefixed with
  the app name, documented in the README's Development section.
- Secrets live in `.env`, which is git-ignored. Never print them.

## Working with the user

The user is learning Rust. When a change leans on a concept (ownership,
trait objects, lifetimes, `cfg`), explain it in a sentence or two at the
point where it matters. Prefer simple idiomatic code over clever code.
