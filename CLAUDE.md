# Working in this repo

Switchboard is a macOS desktop app (Rust, egui/eframe 0.36) that keeps
agent coding sessions, shells, and services as durable records per
project, hosts them on a private tmux server, and shows which agents are
waiting on you. The idea in one line: **workspaces are data, processes are
a cache.** Every feature asks "what is the on-disk record and how is it
rehydrated?" before "what does the pane look like?"

## Where to read first

- `docs/design.md` is the spec. "Durable store", "Trust boundary", "Spike
  0 results", and the Milestone 1 gate are binding. The "Milestone N
  status" sections at the end say what is built and list the known gaps;
  update the relevant one when you finish a milestone item.
- `README.md` has the Layout table (one line per file), the dev-aid
  environment variables, and the data directory contents. Keep all three
  current.
- `spikes/*/README.md` hold verified commands and measured numbers. Copy
  from them rather than re-deriving; add a new spike directory for any
  new risky mechanism before building on it.
- `docs/lessons.md` explains why the architecture is shaped this way.
- `vendor/egui_term/SWITCHBOARD-PATCHES.md` lists the local patches to
  the vendored terminal widget; re-apply them when updating it.

## Commands

```sh
cargo test --locked                                  # unit + headless UI + tmux tests
cargo clippy --locked --all-targets -- -D warnings   # must be clean; pedantic is on
cargo fmt --all
cargo run --locked                                   # launch the app
./scripts/bundle.sh                                  # ~/Applications/Switchboard.app
```

The pre-commit hook runs the first three. Enable it with
`git config core.hooksPath .githooks` on a fresh clone. Never commit with
`--no-verify`. CI runs the same three commands on Linux, so nothing
outside a `cfg(target_os = "macos")` table may need macOS to compile.

## Hard rules

- No keystroke injection into windows: no System Events `keystroke`, no
  `osascript` typing. Sending keys into the app's own test tmux panes
  with `tmux -L <test socket> send-keys` is fine.
- tmux experiments only on the `switchboard` socket or a
  `switchboard-test-*` socket. Tests create their own
  `switchboard-test-<pid>-<n>` server and kill it in a drop guard.
- Never write into a project directory's `.claude/` or `.switchboard/`.
  Nothing in a project directory is executed or parsed as config; a
  hostile `.switchboard/` must be ignored entirely.
- Never modify `~/.claude/settings.json`. Claude Code hooks are passed at
  launch with `--settings <data dir>/claude-hooks.json`.
- Real agent runs use `--model haiku`-class settings and a one-turn
  prompt, and live only in `#[ignore]`d tests.
- Keychain tests use a throwaway keychain file, never the login keychain.
- Secret values never reach a record, a log line, a shell command line,
  or a tool result. They travel as tmux `-e` flags; log names only.
- Agents are never resumed automatically (a resume costs money). The
  startup reconcile launches only trusted `autostart` services.

## Architecture

The layering is documented in `src/lib.rs` and enforced by habit:

- `src/core/` is deterministic: no egui, no threads, no I/O, no wall
  clock. Time comes in through `Clock`. Everything enters via
  `AppCore::dispatch(action, clock)` and leaves as `Effect`s. Startup is a
  reconcile (`StoreLoaded` then `HostListed`), never a launch. Every
  change to a workspace emits `Effect::Save`.
- `src/ports/` holds traits for outside capabilities. `src/adapters/`
  implements them, and `adapters/fakes.rs` holds a fake for every port.
  Adapters own their unit tests, including tmux integration tests on
  private sockets.
- `src/app.rs` owns core plus adapters. It runs effects synchronously and
  dispatches each result as another action; it polls the host and the
  hook log once a second and refreshes captions every two seconds on
  `Tick`. Codex id discovery is a poll, not a thread.
- Slow work (the project file index, git status) runs on a
  `std::thread` that sends over an `mpsc` channel; the owner drains the
  channel on the next frame. Anything the core needs to know still
  enters as one action.
- `src/ui/` only draws. `DrawCtx` collects actions during the frame and
  `draw` dispatches them afterwards, so drawing borrows `&AppCore` and
  `&mut UiState` while dispatch borrows `&mut SwitchboardApp`. `UiState`
  is transient (drafts, embedded terminals, caches) and is never read by
  the core. If logic appears under `ui/`, move it to the core.

Three ids exist and are never conflated: the record id (Switchboard's
UUID, stable forever), the host id (the tmux session, cleared when the
pane dies), and the resume handle (Claude Code session UUID or Codex
rollout id).

## Extending the contract

- `src/core/model.rs`, `src/core/action.rs` (`AppAction`/`Effect`), and
  `src/ports/` are the shared contract. Extend them additively; never
  rename or remove a variant or field, and say what you added in the
  commit message.
- A change to any type in `model.rs` that is serialized needs a
  `SCHEMA_VERSION` bump and a step in `adapters::store::migrate`. Readers
  accept older versions; the store refuses to write a newer one.
- Writes to records go through the store's atomic path (temp file,
  fsync, `.bak`, rename). Never write a record file any other way.

## Adding a feature

1. Action variant plus core transition plus a core test in
   `src/core/tests.rs`.
2. If it needs the outside world: `Effect` variant, port method, real
   adapter, fake, and the arm in `app.rs` that runs it.
3. Widget under `src/ui/` and a UI test in `tests/ui.rs`.
4. A `SWITCHBOARD_SCRIPT` line in `src/script.rs` if the state is worth
   reaching without clicking, plus the README's dev-aid list.
5. README Layout table for any new file; design.md status section for
   any milestone item.

## Driving the live app

There is no keystroke injection. Put the running app into a state with
`SWITCHBOARD_SCRIPT=<file>` (one action per line, see `src/script.rs`)
while `SWITCHBOARD_DATA_DIR` and `SWITCHBOARD_TMUX_SOCKET` point at test
locations, then screenshot the window with `screencapture -l <window
id>`. Keep the data dir path short: the wake socket path has a 104-byte
limit. Hook events can be injected by running `switchboard-hook <EventName>`
by hand with the hook JSON on stdin and `SWITCHBOARD_RECORD_ID` and
`SWITCHBOARD_DATA_DIR` set.

## Testing rules

- Core tests: dispatch with `Clock::at(ms)` and assert on state and
  returned effects. Never sleep.
- UI tests (`tests/ui.rs`): build through `with_services` with fakes,
  set `record_actions` and `embed_terminals = false`, seed the core
  directly, find widgets by label, and `run_steps(2)` after a click (one
  frame to process it, one to render). `type_text` only sends a Text
  event; call `.focus()` on the node first. Size the harness tall enough
  that everything you click is on screen. Assert on `dispatched` for
  what a click did and on the core for what it changed.
- tmux integration tests live beside the tmux adapter and in
  `tests/gate.rs`. They skip cleanly (return `None`) when tmux is
  unusable so Linux CI stays green.
- Tests that spend money or open windows are `#[ignore]`d in
  `tests/live.rs` and the agent items of `tests/gate.rs`, with the exact
  command to run each in the file's doc comment. Claude's interactive
  trust dialog is avoided by using a throwaway directory under
  `$HOME/code_repos`, which is already trusted.

## egui / eframe 0.36 gotchas

Much online material shows the pre-0.36 API and does not compile. When an
API does not match expectations, read the crate source under
`~/.cargo/registry/src/*/egui-0.36*` rather than guessing.

- `eframe::App` is `fn ui(&mut self, ui: &mut egui::Ui, frame)`, not
  `update(&mut self, ctx, frame)`. Get the context with `ui.ctx()`.
- Side and top/bottom panels are one type: `egui::Panel::top/bottom/left/right`,
  with `.exact_size(..)` and `.resizable(false)`. Panels take `.show(ui, ..)`.
- `egui_kittest`: `Harness::builder().with_size(..).build_eframe(|cc| ..)`.
  Import `Role` from `egui::accesskit`.
- Use `GAP` and `PAD` from `ui/mod.rs` for spacing so views line up.

## Platform-specific code

The crate forbids `unsafe`. Native calls (AppKit via `objc2`,
`security-framework`) go in a small adapter behind
`#[cfg(target_os = "macos")]` using safe wrapper crates. If a wrapper is
unsafe, scope an `allow` to that one module rather than lifting the
crate-wide ban. macOS-only dependencies go in the
`[target.'cfg(target_os = "macos")'.dependencies]` table.

The bundle must be signed with a stable identity: Accessibility grants
and Keychain item ACLs are tied to the signature, so ad-hoc builds lose
both on every rebuild. `scripts/bundle.sh` has the recipe.

## Style

- Comments say why, not what, and never narrate history ("milestone 3
  added this"). Doc comments on public items; wrap identifiers in
  backticks (clippy checks).
- Prefer simple idiomatic code over clever code. Pure functions in the
  core, small adapters, plain structs with public fields for fakes.
- Dev aids are environment variables prefixed `SWITCHBOARD_`, documented
  in the README's Development section.
- Local secrets for development live in `.env`, which is git-ignored.
  Never print them.

## Working with the user

The user is learning Rust. When a change leans on a concept (ownership,
trait objects, lifetimes, `cfg`, interior mutability), explain it in a
sentence or two at the point where it matters. Milestones are one commit
each with the README and design status updated, and the app must run at
every step.
