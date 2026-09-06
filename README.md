# Switchboard

A desktop app for organizing, managing, and automating agent coding
sessions. Built in Rust with [egui](https://docs.rs/egui) /
[eframe](https://docs.rs/eframe) from
[egui-app-template](../egui-app-template); design notes live in `docs/`.

Requires Rust 1.95 or newer.

## Development

```sh
cargo run --locked                                   # launch the app
cargo test --locked                                  # unit tests + headless UI tests
cargo clippy --locked --all-targets -- -D warnings   # lint
cargo fmt --all                                      # format
```

### Layout

```
src/main.rs              launcher: wires real adapters, opens the window
src/bin/switchboard-hook.rs  helper Claude Code hooks call (std only)
src/lib.rs               module tree and the layering rules
src/core/
  model.rs               durable data model (Project, SessionRecord, ResumeHandle, CardState)
  action.rs              AppAction, Effect, Clock, AppCore::dispatch (reconcile, return, events)
src/ports/               traits: store, host, events, agent, opener
src/adapters/
  store.rs               JSON store: atomic writes, .bak, flock
  tmux.rs                tmux process host on the private socket
  hooks.rs               append-first event log + socket wake-up + hook settings JSON
  agents.rs              Claude Code / Codex launch, resume, preflight, discovery
  ghostty.rs             open, reveal, Ghostty window launch and raise
  fakes.rs               test doubles for every port
src/app.rs               SwitchboardApp: owns core + adapters; runs effects; polls host
src/ui/                  switcher, board of cards, session view, switchboard view
tests/ui.rs              headless flows via egui_kittest with fakes
vendor/egui_term/        embedded terminal widget (Harzu/egui_term @ 31bbc7ab, egui 0.36)
spikes/                  Spike 0 evidence
```

The flow for any feature: the UI dispatches an `AppAction`; the core
updates its state and returns `Effect`s; the app runs each effect through
an adapter and dispatches the result as another action. Add a capability by
adding a port trait, a real adapter, a fake, and an `Effect` variant. The
core never touches egui, threads, files, or the network.

### Testing approach

Everything enters `AppCore::dispatch(action, clock)` and leaves as effects,
so core tests are plain state-transition tests with an explicit clock and
no sleeping. Adapters get their own tests where they have logic. UI tests
run the real `eframe::App` headlessly with fake adapters, find widgets by
label, and advance frames with `harness.run_steps(2)`: one frame to
process the click, one to render its result.

Tests that hit real services (a model, a paid API) go in their own
`tests/*.rs` file, marked `#[ignore]`, with the command to run them
documented here.

### Pre-commit hook

`.githooks/pre-commit` checks formatting, runs Clippy, and runs the tests
before each commit. It never modifies or stages files. Enable it once per
clone:

```sh
git config core.hooksPath .githooks
```

### Secrets

Local secrets go in `.env` (ignored by git). Read them from the app at
startup; never bake them into the binary or commit them.

## License

MIT, see `LICENSE`.
