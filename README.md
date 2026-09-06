# Switchboard

A desktop app for organizing, managing, and automating agent coding
sessions. Built in Rust with [egui](https://docs.rs/egui) /
[eframe](https://docs.rs/eframe) from
[egui-app-template](../egui-app-template); design notes live in `docs/`.

Requires Rust 1.95 or newer.

## Development

```sh
cargo run --locked                                   # launch the app
cargo test --locked                                  # unit + headless UI + tmux integration tests
cargo clippy --locked --all-targets -- -D warnings   # lint
cargo fmt --all                                      # format
```

Requirements: macOS, `tmux` 3.2 or newer (`brew install tmux`), Ghostty
for agent sessions, and `claude` and/or `codex` on `PATH`. The app tells
you at the bottom of the window when tmux is missing.

Data lives in `~/Library/Application Support/Switchboard/`: one JSON file
per project under `projects/` (with a `.bak` of the previous version), the
tmux config and socket name, `claude-hooks.json` (passed to Claude Code
with `--settings`), `events.log` (the hook event log), `wake.sock`, and
`scrollback/`. Sessions run on a private tmux server (`tmux -L
switchboard`), never on your default one. Nothing is written into a
project directory.

## App bundle

```sh
./scripts/bundle.sh          # installs ~/Applications/Switchboard.app
./scripts/icon.sh            # regenerates the icon from assets/Switchboard.svg
```

The bundle carries both binaries (`switchboard` and `switchboard-hook`,
which the app locates next to its own executable). Raising a Ghostty
window needs Accessibility, and macOS ties that grant to the code
signature: the script signs with `CODESIGN_IDENTITY`, else a "Switchboard
Dev" or "Prompt Box Dev" certificate from Keychain Access, else ad-hoc
(in which case the grant is reset each rebuild). `scripts/icon.sh`
renders the SVG with AppKit into `assets/Switchboard.icns` for Finder and
`assets/icon-256.rgba`, which `src/main.rs` embeds for the Dock.

Dev aids, all environment variables:

- `SWITCHBOARD_DATA_DIR=<dir>`: use another data directory (keep the path
  short; the wake socket path has a 104-byte limit).
- `SWITCHBOARD_TMUX_SOCKET=<name>`: use another tmux socket name.
- `SWITCHBOARD_SCRIPT=<file>`: run actions at startup, one per line, so
  the app can be put into a known state without clicking. See
  `src/script.rs` for the lines (`add-project`, `new-shell`, `new-claude`,
  `new-codex`, `new-service`, `show-board`, `show-session`, `return`,
  `kill`, `switchboard`).
- `SWITCHBOARD_TMUX=<path>`: tmux binary to use.
- `RUST_LOG=switchboard=debug`: verbose logging.

Live tests that spend money or open windows are `#[ignore]`d:

```sh
cargo test --test live -- --ignored --nocapture     # one cheap claude, codex, and Ghostty run each
cargo test --test gate -- --ignored --nocapture     # the Milestone 1 gate items that need real agents
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
src/ports/               traits: store, host, events, agent, opener, transcript
src/adapters/
  store.rs               JSON store: atomic writes, .bak, flock
  tmux.rs                tmux process host on the private socket
  hooks.rs               append-first event log + socket wake-up + hook settings JSON
  agents.rs              Claude Code / Codex launch, resume, preflight, discovery
  transcript.rs          Claude Code transcript (JSONL) -> Conversation turns
  ghostty.rs             open, reveal, Ghostty window launch and raise
  fakes.rs               test doubles for every port
src/app.rs               SwitchboardApp: owns core + adapters; runs effects; polls host and events
src/script.rs            SWITCHBOARD_SCRIPT dev aid
src/ui/
  mod.rs                 UiState, draw loop (collect actions, then dispatch), keyboard
  switcher.rs            top bar (project strip, badge, add project) and bottom bar
  board.rs               one project's board of cards, pinned documents, notes
  cards.rs               session and document cards, state colors
  session.rs             session view: header, notes, embedded terminal or conversation + message box
  switchboard.rs         every session across projects, waiting first
  dialogs.rs             add project / create session dialogs
tests/ui.rs              headless flows via egui_kittest with fakes
tests/fixtures/          a small real Claude Code transcript for the parser tests
tests/live.rs            ignored: real claude / codex / Ghostty runs
tests/gate.rs            Milestone 1 gate: real store, tmux, hooks; agents ignored
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
