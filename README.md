# Switchboard

A desktop app for organizing, managing, and automating agent coding
sessions. Built in Rust with [egui](https://docs.rs/egui) /
[eframe](https://docs.rs/eframe) from
[egui-app-template](../egui-app-template); design notes live in `docs/`.

Requires Rust 1.95 or newer.

## Development

```sh
cargo run --locked                                   # launch the app
cargo test --locked --workspace                      # unit + headless UI + tmux integration tests, every crate
cargo clippy --locked --workspace --all-targets -- -D warnings   # lint
cargo fmt --all                                      # format
./scripts/test-times.sh                              # every test timed serially, the slowest ten, the budget checked
```

Test times. The budget is no test over 5 s and the whole workspace
under two minutes locally; `scripts/test-times.sh` checks the first and
exits non-zero past it. Measured on an M-series Mac with the script
(serial):

```
slowest ten:
  2.26 s  gate  project_config_is_listed_but_never_run_until_approved
  2.19 s  gate  killing_the_attached_client_keeps_the_session
  1.52 s  switchboard (lib)  ui::session::tests::a_dropped_terminal_releases_its_descriptors
  1.51 s  dispatch/dispatch (lib)  health::tests::a_socket_that_never_answers_times_out_and_one_that_does_is_timed
  0.88 s  dispatch/first_slice  a_lost_unmark_is_sent_again_until_it_lands
  0.74 s  dispatch/first_slice  keep_or_no_key_leaves_history_alone
  0.70 s  dispatch/first_slice  a_rebase_that_changes_nothing_or_past_the_cap_is_a_question
  0.66 s  gate  restart_reattaches_to_live_sessions
  0.62 s  dispatch/first_slice  a_branch_pushed_by_an_agent_is_not_rewritten_after_a_send_back
  0.60 s  switchboard (lib)  adapters::tmux::tests::shell_session_with_env_cwd_and_scrollback
each binary, serially:
 40.07 s  dispatch/first_slice
  5.54 s  dispatch/dispatch (lib)
  5.44 s  gate
  3.48 s  switchboard (lib)
  1.11 s  ui
  0.28 s  control
 55.92 s  in all
```

`cargo test --locked --workspace` took 37 s of wall time in parallel on
the same machine, built. The two gate tests at the top spend most of
their time holding still on purpose (half-second and two-second windows
in which nothing may happen). Measure on a quiet machine: another
checkout running its tests at the same time makes every file operation
wait behind its device flushes, and the same script then reads ten
times slower.

Requirements: macOS, `tmux` 3.2 or newer (`brew install tmux`), Ghostty
for agent sessions, `cmake` (`brew install cmake`; the embedded Prompt Box
builds whisper.cpp), and `claude` and/or `codex` on `PATH`. The app tells
you at the bottom of the window when tmux is missing. Prompt Box is a git
dependency pinned by revision; if cargo cannot fetch it because a git
`insteadOf` rule turns the URL into ssh, run with
`CARGO_NET_GIT_FETCH_WITH_CLI=true` (or set `net.git-fetch-with-cli` in
`~/.cargo/config.toml`) so the git CLI's credentials are used. The
bundle declares microphone use (`NSMicrophoneUsageDescription`): a
bundled app without it gets silence from the microphone, no prompt and
no error, so listening looks dead.

Data lives in `~/Library/Application Support/Switchboard/`: one JSON file
per project under `projects/` (with a `.bak` of the previous version;
approvals of defined commands live inside these records),
`launch.log` (what the last launch loaded and chose for the main
window's frame, for a launch with no terminal to log to),
`operations.log` and `control.sock` (the control port: every command
another process sent, its reply, and the socket it came in on; see
`docs/dispatch.md`),
`settings.json` (theme, the active workspace, editor, global variables, file
side shown next to sessions and its tab, the directory each project's
file side starts at when narrowed, the screen to reopen on, the
Prompt Box switch and its trigger word, model, captions, and the screen
its caption bar and preview panel appear on, the sessions open in
windows of their own and where each window sits, where the main
window sits, the zoom of each display, the workflow round cap and the
user's workflow definitions),
`views.json` (schema v6: the workspaces, and the working sets: each
one's name, workspace, which sessions and files are on it, and where
each card sits on its grid, with a `.bak`; a set made in the global
workspace, "Everywhere", is in no real workspace and may hold cards
from all of them; a rule set stores its rule, such as "active in the
last 24 hours", and the sessions dismissed from it instead of cards),
the
`tmux.conf`, `claude-hooks.json` (passed to Claude Code
with `--settings`), `events.log` (the hook event log), `wake.sock`, and
`scrollback/` (one `<host>-r<n>.vt` per run of a command or service,
the last 20 runs kept), `renders/` (first pages of PDFs shown on cards,
rasterized by Quick Look), and `workflows/<run>/round-<n>/` (copies of
the plan, feedback, and response at the end of each review round). Sessions run
on a private tmux server (`tmux -L switchboard`), never on your default
one. Nothing is written into a project directory except what the Config
editor saves on your click.

### Defining commands and services

A project can declare its commands and services in
`<root>/.switchboard/project.json`, so an agent working in the project
can set them up for you:

```json
{
  "version": 1,
  "commands": [
    { "name": "rebundle", "command": "./scripts/bundle.sh" },
    { "name": "lint", "command": "cargo clippy", "cwd": "crates/app", "env": ["RUSTFLAGS"] },
    { "name": "report", "command": "make report", "output": ["out/report.pdf", "out/*.md"] }
  ],
  "services": [
    { "name": "web", "command": "npm run dev", "autostart": true }
  ],
  "show": ["manager", "orchard-backend"]
}
```

`show` lists folders under the root that the file side lists even when
the root's `.gitignore` hides them, such as sub-repositories checked out
inside a workspace directory. Each is walked as its own tree: its own
`.gitignore` applies, the root's does not. Entries must be relative
paths without `..`; others are skipped with a warning.

Names are 1 to 64 characters and unique across both lists; `cwd` is
relative to the project root and may not use `..`; `env` lists the
variable names the command expects (values come from the project's
environment, never from this file); `output` is a glob pattern or a
list of them, relative to the command's directory, naming the files a
run produces: those written during the run are listed on the command's
card and page afterwards, with a Markdown or PDF file shown in place.
An entry with an unknown field or
a bad value is skipped with a warning that names it; the rest still
load. The file is limited to 64 KiB and must not be a symlink.

Nothing in the file runs until you approve the entry in the Run tab,
where you see the command, its directory, and the variables it asks
for. Any change to an entry drops its approval; `autostart` is honored
only for approved services. The board's Config button opens the file
for editing, with the options listed beside it and the parse result shown
as you type; Save is the only time Switchboard writes into
`.switchboard/`.

## App bundle

```sh
./scripts/bundle.sh          # installs ~/Applications/Switchboard.app
./scripts/icon.sh            # regenerates the icon from assets/Switchboard.svg
```

The bundle carries both binaries (`switchboard` and `switchboard-hook`,
which the app locates next to its own executable). Raising a Ghostty
window needs Accessibility, and macOS ties that grant to the code
signature, and the Keychain ties its per-item "always allow" to the
signer's Team ID. The script signs with `CODESIGN_IDENTITY`, else the
first "Developer ID Application" identity (has a Team ID, so Keychain
items stay allowed across rebuilds), else a "Switchboard Dev" or "Prompt
Box Dev" certificate from Keychain Access (keeps the Accessibility grant,
but the Keychain asks for its items again after each build), else ad-hoc
(the grant is reset each rebuild). `scripts/icon.sh`
renders the SVG with AppKit into `assets/Switchboard.icns` for Finder and
`assets/icon-256.rgba`, which `src/main.rs` embeds for the Dock.

Dev aids, all environment variables:

- `SWITCHBOARD_DATA_DIR=<dir>`: use another data directory (keep the path
  short; the wake socket path has a 104-byte limit).
- `SWITCHBOARD_TMUX_SOCKET=<name>`: use another tmux socket name.
- `DISPATCH_DATA_DIR=<dir>`: where the Dispatch page looks for
  `dispatch.sock` and where its console opens (the `dispatch` binary
  reads the same variable). Tickets' trees go under
  `~/.dispatch/worktrees` regardless, or wherever `dispatch worktrees
  <path>` (also on the Dispatch page) points; the setting is
  `<dir>/settings.json`. `<dir>/events.jsonl` is the event log
  `dispatch events` and `wait` read, and `<dir>/runner.json` the
  runner's status after its last pass, which `dispatch health` reads.
  `<dir>/lock` is the writer lock, with a line naming its holder, and
  `<dir>/tickets/<id>/closing.lock` is held by whichever `dispatch` is
  finishing that ticket's close; both are lock files, not records.
- `SWITCHBOARD_SCRIPT=<file>`: run actions at startup, one per line, so
  the app can be put into a known state without clicking. See
  `src/script.rs` for the lines (`add-project`, `new-shell`, `new-claude`,
  `new-codex`, `new-service`, `show-board`, `show-session`, `show-document`,
  `files`, `side-position`, `terminal`, `select-file`, `set-env`, `set-secret`, `dotenv`, `environment`, `config`,
  `send`, `interrupt`, `return`, `kill`, `remove`, `approve`, `revoke`, `side`,
  `switchboard`, `working-set`, `new-working-set`, `clone-working-set`,
  `rename-working-set`, `delete-working-set`, `new-recent-set`,
  `set-hours`, `dismiss-from-set`, `add-to-working-set`,
  `add-file-to-working-set`, `arrange`, `show-message`, `clone-session`,
  `discard-to`, `undo-discard`, `review-plan`, `show-review`,
  `show-dispatch`, `show-ticket`, `close-ticket`,
  `review-file`, `review-continue`, `review-finalize`, `show-artifact`,
  `pop-out`, `close-pop-out`, `files-root`, `zoom`, `place-pop-out`,
  `place-card`, `new-workspace`, `workspace`, `workspace-global`,
  `move-project`, `move-working-set`, `controller`,
  `open-terminal`, `prompt-box`, `theme`, `sleep`).
- `SWITCHBOARD_CONTROLLER=<device>`: the nunchuk's serial port (default: the
  first `/dev/cu.usbmodem*`, waited for if absent).
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
  grid.rs                Working Set placement: default card sizes, first free spot, overlap, minimum size, the card a step away, a rule set's flowed layout
  controller.rs          the hand controller's meaning: the selected card per working set, Z's radial menu, the session C holds open
  definitions.rs         .switchboard/project.json entries -> records; hash-keyed approval
  env.rs                 environment resolution for a new session: global variables, opted-in .env files, the project's own; secrets by account name
  events.rs              hook events -> record activity (matched by record id, ordered by time)
  workflow.rs            plan review runs: reviewer and planner rounds as a state machine over records
  control.rs             the control port's commands run quietly under an operation id; read models in the wire's shapes
  dispatch.rs            Dispatch as the app shows it: the runner's last status as views, decisions answered as calls, the console session
  tests.rs               state-transition tests for the core
src/ports/               traits: store, host, events, agent, opener, transcript, secrets, project_config, round_files, artifacts, controller, control (the operations log), dispatch (Dispatch's port)
src/adapters/
  store.rs               JSON store: atomic writes, .bak, flock
  tmux.rs                tmux process host on the private socket
  hooks.rs               append-first event log + socket wake-up + hook settings JSON
  control.rs             the control port: operations.log (append-only, synced) and control.sock (one thread per connection, requests handed to the frame)
  dispatch.rs            the Dispatch port's client over <Dispatch data dir>/dispatch.sock, reconnecting after an error; the `dispatch` beside the app's binary
  dock.rs                Dock badge with the waiting-session count (macOS)
  controller.rs          the nunchuk over USB serial: a thread owns the port, reconnects, hands events over a channel
  files.rs               project file index: gitignore-aware scan, lazy children, fuzzy match
  git.rs                 branches, change counts, per-path status; finds repos one or two dirs down
  keychain.rs            secrets as generic-password items in the login Keychain (tests use a temp keychain)
  dotenv.rs              .env parser (opt-in per project) and .env.example names
  project_config.rs      reads and validates .switchboard/project.json (capped, no symlinks)
  round_files.rs         a workflow's round files on disk: probe, snapshot into the data dir, delete
  scrollback.rs          read the pipe-pane stream back as plain text (cold sessions)
  artifacts.rs           a run's declared outputs: glob under the cwd, modified since the run began
  agents.rs              Claude Code / Codex launch, resume, preflight, discovery
  transcript.rs          Claude Code transcript (JSONL) -> Conversation turns
  ghostty.rs             open, reveal, Ghostty window launch and raise
  fakes.rs               test doubles for every port
src/app.rs               SwitchboardApp: owns core + adapters; runs effects; polls host and events; answers control-port queries (`session.screen` with the project's secret values redacted)
src/script.rs            SWITCHBOARD_SCRIPT dev aid
src/ui/
  mod.rs                 UiState, draw loop (collect actions, then dispatch), keyboard, side panel tabs
  prompt_box.rs          the Prompt Box editor per agent session, one voice runtime bound to one of them
  theme.rs               the look: color tokens per theme, Source Serif 4, type scale, shared widgets (dot, kicker, buttons)
  rail.rs                left project rail (brand, All sessions, Dispatch with its pending decisions, projects with dots, Go to, Settings); a session's neighbours beside it
  switcher.rs            Settings menu and the toasts (notice, host error)
  board.rs               one project's board: run bar, agent and shell cards, command and service rows, pinned documents, notes
  files.rs               Files tab of the side panel: lazy tree (from the project root or a directory chosen as its top), fuzzy finder, bottom preview pane, right-click hand-offs
  run.rs                 Run tab of the side panel: commands and services, definitions, approval, last run
  runs.rs                a command or service as runs: the kicker (exit, duration, when), the card body with output or an artifact, the page with run history
  notes.rs               Notes tab of the side panel: the session's notes, edited in place
  runbar.rs              one button per command and service, under the board strip and the session header
  document.rs            read-only preview: Markdown, text, images, a PDF's first page; full view and the side pane's body
  markdown.rs            Markdown: prose through egui_commonmark, tables laid out here with content-sized columns
  palette.rs             quick-switcher (Cmd+K) over projects and sessions
  popout.rs              a session in a window of its own: the page and side panel in a viewport, frame saved on settle; the Dispatch window the same way
  zoom.rs                Cmd+= and Cmd+- per window, remembered per display; a pop-out's pass runs at its display's zoom
  env.rs                 Environment dialog: variables, secrets, .env opt-in, masked preview
  config.rs              project config editor: .switchboard/project.json as text, options listed, parse shown
  workflow.rs            plan review: the Review plan dialog, the run's page (rounds, plan with diff, feedback beside response, controls)
  dispatch.rs            the Dispatch pages: tickets with what waits on you (filtered by project) and the console; one ticket's stages, attempts, decisions and artifacts; Pop out
  cards.rs               the one card for every entry kind, the card grid, pinned document cards
  session.rs             session view: header, embedded terminal or conversation + message box
  switchboard.rs         every session across projects, waiting first
  working_set.rs         a working set: the user's grid of session and file cards from any project, or a rule set's cards with its hours field and dismiss controls
  dialogs.rs             add project / create session dialogs, the full-message and links-in-message dialogs
assets/fonts/            Source Serif 4 (Regular, Semibold, Italic; OFL), embedded by theme.rs
tests/ui.rs              headless flows via egui_kittest with fakes
tests/control.rs         the control port over a real socket with fakes: ops on records, logged replies, interrupted launches, a pane's screen with secrets named
tests/fixtures/          a small real Claude Code transcript for the parser tests
tests/live.rs            ignored: real claude / codex / Ghostty runs
tests/gate.rs            Milestone 1 gate: real store, tmux, hooks; agents ignored
scripts/test-times.sh    every test's time, serially, through libtest's `--report-time`; the slowest ten, each binary's total, the 5 s budget
scripts/ci-test.sh       CI's test step: the workspace, then one retry of the failed tests, warning `flaky:` for each that passes on it
control/                 switchboard-control: the control port's wire contract (requests, replies, views) and a blocking client; std + serde only
dispatch-control/        dispatch-control: the wire contract of Dispatch's own port (tickets as views, decide, queue, take, resume, close, worktrees) and a blocking client; std + serde only
dispatch/                the `dispatch` binary (docs/dispatch.md; docs/dispatch-agent-guide.md is the command-line guide for agents that take tickets; docs/dispatch-pipeline-improvements.md is the open list of pipeline changes drawn from tickets run so far): a ticket scheduler that drives Switchboard over the control port and never links the app
  src/main.rs            CLI: take, run, decide, decisions, status, queue, resume, close, worktrees; for a supervising agent, events, wait, show, report, tail and health (usage errors exit 64)
  src/events.rs          the event log `events.jsonl`: what a ticket write changed (`between`, pure), appended and synced before the record's rename, a `void` for a write that failed; read, followed, and waited on with each match checked against the record
  src/report.rs          how a ticket went, pure: stage time, plan and code review points counted by id, fix passes, rebases, the PR's size
  src/health.rs          the runner's call counters, written as `runner.json` after every pass, and the `health` check of that file and both sockets
  src/serve.rs           Dispatch's port on <data>/dispatch.sock while `run` is up: the records as views, the commands the CLI has, one handler under one lock
  src/pipeline.rs        the TOML pipeline file, parsed in full and validated; fingerprint of the copy a ticket runs
  src/ticket.rs          the ticket record: source, lanes (with a conflict seen and not yet brought up, and their last bring-up, `Refreshed`: whether the branch had commits, when, the rebaser's notes, the conflict it resolved and the head it reached), attempts (with their checks' head and exit, the attempt a review carries and its send-back note, the history rewrite a review attempt made), decisions, operation ledger, a close's progress (`CloseProgress`); the per-project queue and closing list
  src/scheduler.rs       the runner: stage executors (agent, gate-only lanes, review workflow), completion evidence, decisions, ledgered sends, parking and closing as sequences; bringing a lane up to its moved base, recorded with the rebaser's notes; a conflicted bring-up after the last code review gets one resolution review; a retake's kept branches deleted or asked about at the cut
  src/review.rs          the code review stage: rounds of several reviewers over a lane's branch, a fresh implementer per round, checks at every accepted head; a rerun carries the last attempt's settled and open points; style and the plan's decisions do not hold a round open; a reviewer's line that every point left is wording is a note, not a point; folds the fix rounds into the commits they amend as it completes, or keeps them when the user answers `keep` to a failed fold; the one-pass resolution review (docs/dispatch.md, "The code review stage")
  src/recover.rs         unanswered ledger operations resolved by class through find and op.status; nothing launched twice
  src/view.rs            the `Dispatch · <project>` working set, one card per queued ticket, redrawn through set.sync
  src/store.rs           Dispatch's data directory, flock, atomic writes; versioned `read_ticket`/`read_project` through `migrate`; `write_ticket_logged`, the one ticket write, which logs its events first
  src/git.rs             clones, worktrees and heads through fixed argv (a branch pushed only with a lease; a worktree removed, never forced; `changes`, and `uncommitted` for a close's preflight), a stage's checks as child processes polled by key (confined, when the pipeline says so, through `Confine`), commits replayed with `merge-tree`/`commit-tree`, the commits whose rebase conflicts read the same way, a branch moved by compare-and-swap, and whether the remote already holds a branch's commits; a kept branch read, deleted or renamed out of a retake's way; a range's commits and diff size for a report; with a fake
  src/confine.rs         a pipeline command's confinement: the seatbelt profile and wrapper built everywhere, used on macOS; elsewhere the command runs unconfined and says so
  src/history.rs         which commits a code review's fix rounds fold into: pure plans for `fold` and `one`
  src/github.rs          issues and pull requests through gh, with fakes
  src/bitbucket.rs       pull requests on Bitbucket Cloud through curl; credentials from the environment or <data>/env (NAME=value lines, mode 0600, never logged)
  src/port.rs            the Port trait over the control socket client; the connection remade after any error, a timeout included; `path()` names the socket for `health`
  src/template.rs        `{a.b}` substitution for prompts
  tests/first_slice.rs   the acceptance table against an in-memory Switchboard (tests/support)
  tests/live.rs          ignored: the first stage against a real Switchboard and a haiku agent
  tests/sandbox.rs       ignored, macOS: a confined gate's refused write fails the attempt and is named in checks.log
vendor/egui_term/        embedded terminal widget (Harzu/egui_term @ 31bbc7ab, egui 0.36; see SWITCHBOARD-PATCHES.md)
firmware/nunchuk/        CircuitPython for the Feather that reports the nunchuk's buttons and stick
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

A test that waits on a real process (a tmux pane, a child process, a
thread's result) polls to a deadline through its file's helper
(`poll_on` in the tmux adapter, `poll_to`, `wait_until` and `hold` in
`tests/gate.rs`, `frames_until` in `tests/ui.rs`, `poll_to` in
Dispatch's `git.rs`) and never sleeps a fixed time before an assert.
Dispatch's integration tests skip the device flush
(`store::skip_fsync_for_tests`) and move time with `Env::wait` rather
than stepping through it.

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
