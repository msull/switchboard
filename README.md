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
  action.rs              AppAction, Effect, Clock, AppCore::dispatch, read model for the UI
  reconcile.rs           StoreLoaded / HostListed: card states, autostart services, spawn specs
  sessions.rs            launch, idempotent return, resume preflight, Codex serialization
  events.rs              hook events -> record activity (matched by record id, ordered by time)
  tests.rs               state-transition tests for the core
src/ports/               traits: store, host, events, agent, opener
src/adapters/
  store.rs               JSON store: atomic writes, .bak, flock
  tmux.rs                tmux process host on the private socket
  hooks.rs               append-first event log + socket wake-up + hook settings JSON
  agents.rs              Claude Code / Codex launch, resume, preflight, discovery
  ghostty.rs             open, reveal, Ghostty window launch and raise
  fakes.rs               test doubles for every port
src/app.rs               SwitchboardApp: owns core + adapters; runs effects; polls host
src/ui/
  mod.rs                 UiState, draw loop (collect actions, then dispatch), keyboard
  switcher.rs            top bar (project strip, badge, add project) and bottom bar
  board.rs               one project's board of cards, pinned documents, notes
  cards.rs               session and document cards, state colors
  session.rs             session view: header, notes, embedded terminal or Ghostty note
  switchboard.rs         every session across projects, waiting first
  dialogs.rs             add project / create session dialogs
tests/ui.rs              headless flows via egui_kittest with fakes
tests/gate.rs            Milestone 1 gate: the app driven through dispatch on a real store, tmux, and hooks
tests/live.rs            real claude / codex / Ghostty runs (all ignored)
vendor/egui_term/        embedded terminal widget (Harzu/egui_term @ 31bbc7ab, egui 0.36; see SWITCHBOARD-PATCHES.md)
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

`tests/gate.rs` is the Milestone 1 gate: one test per gate item, driving
`SwitchboardApp` through `dispatch` and `poll_now` on a real `JsonStore`,
a private `switchboard-test-gate-*` tmux server, and the real hook helper,
with a fake opener in place of Ghostty. The three that run a real agent
are ignored; each costs a fraction of a cent:

```sh
cargo test --test gate -- --ignored claude_sessions_map_to_their_cards --nocapture
cargo test --test gate -- --ignored hook_events_while_down_apply_in_order --nocapture
cargo test --test gate -- --ignored codex_launches_bind_distinct_ids --nocapture
```

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
