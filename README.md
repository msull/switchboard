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
cargo test --locked -p egui_term --lib               # the vendored terminal widget (outside the workspace)
cargo clippy --locked --workspace --all-targets -- -D warnings   # lint
cargo fmt --all                                      # format
(cd client && swift test)                            # the Swift client's packages, 81 tests (Xcode; not in the hook or CI)
(cd client && scripts/sim.sh run)                    # the visionOS app on the simulator (xcodegen; see client/README.md)
./scripts/test-times.sh                              # every test timed serially, the slowest ten, the budget checked
(cd tests/browser && npm ci && npx playwright install chromium)   # once, for the browser test
cargo test --locked --test remote -- --ignored the_page_works_in_a_real_browser --nocapture   # the web page in headless Chromium (not in the hook or CI)
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
`identity.json` (the remote port's identity: the server id, its display
name, the self-signed certificate and its key hash, and whether the
port listens on the local network and on which port; the certificate's
private key is in the Keychain) and `devices.json` (the devices paired
with this host: name, scope, when paired and last seen, and the SHA-256
of each bearer token, never the token; see `docs/remote-client.md`),
`settings.json` (theme, the active workspace, editor, global variables, file
side shown next to sessions and its tab, the directory each project's
file side starts at when narrowed, the screen to reopen on, the
Prompt Box switch and its trigger word, model, captions, and the screen
its caption bar and preview panel appear on, the sessions open in
windows of their own and where each window sits, where the main
window sits, the zoom of each display, the workflow round cap and the
user's workflow definitions, and the environment sets: each set's
variables, which of them are secret, and its AWS method),
`views.json` (schema v7: the workspaces, and the working sets: each
one's name, workspace, which sessions and files are on it, and where
each card sits on its grid, with a `.bak`; a set made in the global
workspace, "Everywhere", is in no real workspace and may hold cards
from all of them; a rule set stores its rule, such as "active in the
last 24 hours", the sessions dismissed from it, whether it shows
only running sessions, and its card size instead of cards),
the
`tmux.conf`, `claude-hooks.json` (passed to Claude Code
with `--settings`), `events.log` (the hook event log; a
`UserPromptSubmit` line carries `injected`, never the prompt; a `Stop`
line carries `tasks` and `crons`, kinds and schedules only),
`crons/` (one file per Claude Code session: a hash of each cron prompt
the last `Stop` listed, never the text; removed when the session starts
or ends, and swept after a day),
`wake.sock`, and
`scrollback/` (one `<host>-r<n>.vt` per run of a command or service,
the last 20 runs kept), `renders/` (first pages of PDFs shown on cards,
rasterized by Quick Look), and `workflows/<run>/round-<n>/` (copies of
the plan, feedback, and response at the end of each review round). Sessions run
on a private tmux server (`tmux -L switchboard`), never on your default
one. Nothing is written into a project directory except what the Config
editor saves on your click. Secret values live in the login Keychain,
never in these files, under the accounts `global/<VAR>`,
`project/<id>/<VAR>` and `set/<name>/<VAR>` (an environment set's), and
`remote-key:<server id>` (the remote port's private key, as PEM).

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
./scripts/bump.sh minor      # the next version into Cargo.toml, Cargo.lock and CHANGELOG.md, folding changelog.d/
./scripts/release.sh         # publishes the committed version as a GitHub release (--dry-run: build and zip only)
./scripts/icon.sh            # regenerates the icons from assets/Switchboard.svg and assets/vision/*.svg
```

The bundle carries the binaries side by side in `Contents/MacOS`:
`switchboard`, `switchboard-hook` (which the app locates next to its own
executable), `switchboard-env` and `switchboard-ask` (which Dispatch
locates next to its own) and `dispatch`. Raising a Ghostty
window needs Accessibility, and macOS ties that grant to the code
signature, and the Keychain ties its per-item "always allow" to the
signer's Team ID. The script signs with `CODESIGN_IDENTITY`, else the
first "Developer ID Application" identity (has a Team ID, so Keychain
items stay allowed across rebuilds), else a "Switchboard Dev" or "Prompt
Box Dev" certificate from Keychain Access (keeps the Accessibility grant,
but the Keychain asks for its items again after each build), else ad-hoc
(the grant is reset each rebuild). `scripts/icon.sh`
renders the SVG with AppKit into `assets/Switchboard.icns` for Finder and
`assets/icon-256.rgba`, which `src/main.rs` embeds for the Dock, and the
three layers of the visionOS icon (`assets/vision/*.svg`, the same
drawing split for the headset's parallax and circular mask) into the
client's asset catalog.

Releases are numbered by [semantic versioning](https://semver.org) and
listed in `CHANGELOG.md`; the version in `Cargo.toml` is the one source,
and it reaches the binary (`env!("CARGO_PKG_VERSION")`, shown at the
bottom of the Settings menu), the bundle's `Info.plist` and the host's
`/info` reply (the headset shows it beside each host). A release is
three steps:

```sh
./scripts/bump.sh minor          # or patch, major, or X.Y.Z: Cargo.toml, Cargo.lock, changelog.d/ into CHANGELOG.md
git commit -am "Release 0.3.0"   # after a look at the diff
./scripts/release.sh             # builds, signs, zips, tags v0.3.0, publishes with the changelog section
```

Every user-visible change adds a fragment, `changelog.d/<branch>.md`,
as it lands (format in `changelog.d/README.md`), so concurrent branches
never edit the same lines of `CHANGELOG.md`; the bump folds the
fragments into the new version's section and removes them.
`scripts/release.sh` builds the bundle through `bundle.sh` into a
staging directory, zips it with `ditto` (a plain `zip` would lose the
signature), writes the zip's SHA-256 as a file of its own, tags the
commit `v<version>` and attaches both to a GitHub release whose notes
are the changelog's section. It refuses a dirty tree, a version without
a changelog section, and a tag that exists.

A running Switchboard checks the repository's latest release twenty
seconds after launch and hourly (`core::updates`, the
`adapters::updates::GitHubReleases` source over `ureq`). A newer version
is a notice and an "Update" item on the rail: the click downloads the
zip to `$TMPDIR/switchboard-update`, checks it against the published
SHA-256, unpacks it with `ditto`, verifies the bundle's signature with
`codesign` and requires its Team ID to equal the running app's (so the
Accessibility and Keychain grants keyed on the signature survive), and
then offers "Restart to update": a helper script waits for the process
to exit, moves the old bundle to `.Switchboard.app.previous` beside it,
copies the new one in and relaunches it, putting the old one back if the
copy fails (its output is in `$TMPDIR/switchboard-update/swap.log`). A
bare `cargo run` binary, an unsigned bundle or one outside a writable
directory refuses to install and says so. A pre-release tag or one that
does not parse is never offered.

Another Mac installs a release by hand, with `tmux`, Ghostty and `gh`
installed:

```sh
gh release download v0.2.0 -R msull/switchboard -p 'Switchboard-*.zip' -D ~/Downloads
ditto -x -k ~/Downloads/Switchboard-0.2.0.zip ~/Applications
open ~/Applications/Switchboard.app
```

`gh` and `curl` set no quarantine flag, so Gatekeeper asks nothing of
the bundle, which is signed but not notarized; a browser download
carries the flag, and `xattr -dr com.apple.quarantine <app>` clears
it. From then on the app updates itself. That Mac makes its own
identity and key on first launch, so it is a second host to the remote
client, never a copy of this one.

Dev aids, all environment variables:

- `SWITCHBOARD_DATA_DIR=<dir>`: use another data directory (keep the path
  short; the wake socket path has a 104-byte limit). The app sets it in
  every pane it spawns, so `switchboard-env` in a shell asks the
  instance that launched it.
- `SWITCHBOARD_RECORD_TOKEN`: set by the app in every pane, beside
  `SWITCHBOARD_RECORD_ID`, and new at every spawn. `switchboard-env
  exec` sends it with `env.resolve`; the record keeps only its SHA-256.
  To try `switchboard-env` by hand, run it in a shell the app launched,
  after `env-setup unlock` (or the settings menu's Unlock environment
  setup) for the setup commands. `switchboard-ask` sends it too, with
  `session.ask`; `switchboard-ask "a question"` in a shell the app
  launched marks that shell's card as waiting on you, and
  `switchboard-ask --clear` takes the mark off. A Claude Code session
  may add `--confirm`, `--choice "<option>"` (once per option) or
  `--text`; the owner answers on the card, and the answer comes back
  as the session's next prompt, `Owner answered "<question>": <answer>`,
  once its turn has ended.
- `SWITCHBOARD_TMUX_SOCKET=<name>`: use another tmux socket name.
- `SWITCHBOARD_REMOTE_PORT=<port>`: bind the remote port there instead
  of the port in `identity.json`, so two instances can run on one
  machine; `0` picks a free one. The port answers `/api/v1/info` to
  anyone and every other read to a paired device; pair one from the
  settings menu's Remote devices dialog, and read with the token it
  was given (`curl --insecure` trusts any certificate; a real client
  pins the key hash the dialog shows):
  `curl -sk -H "Authorization: Bearer <token>" https://127.0.0.1:43187/api/v1/waiting`.
  The Swift client's command-line browser does the pinned version:
  `client/README.md`; the dialog's "Copy invite" puts the QR code's
  text, the page's URL with the code and key in its fragment, on the
  clipboard, so `switchboard-remote pair "$(pbpaste)"` pairs it, and
  opening it in a browser loads the port's web page (`web/`), which
  pairs into a cookie. Script lines `remote-listen lan <port>` and
  `pairing <code>` put an instance on the network with a window open.
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
  `<dir>/tickets/<id>/<stage>/<n>/<context>/<name>/` (mode 0700) holds a
  stage's evidence files. It is removed `evidence_keep_days` after the
  ticket closes, or at close with `--drop-evidence`.
  `<dir>/tickets/<id>/usage.json` caches the token counts, model names
  and timestamps read from the ticket's sessions' transcripts, never
  their content, so `report` and the Usage tab keep them after Claude
  Code deletes the transcripts.
  `<dir>/projects/<project>/supervisor/` holds a project's supervisor
  files: `seed.md`, the prompt its session starts from, `handoff.md`,
  which the session keeps, and `handoff.<stamp>.md`, each earlier
  hand-off kept when a fresh session replaced it. The supervisor works
  in `supervisor-<project>` under the worktrees root. The project's
  record `<dir>/projects/<project>.json` keeps the supervisor's sessions
  and, in `supervision.refusals`, the last 20 commands it was refused
  for want of a `may` capability.
  The overview's Start runs `dispatch run` as the app's `runner`
  service in that directory, and the runner reaches this app through its
  own `SWITCHBOARD_DATA_DIR`, so a dev instance should point
  `DISPATCH_DATA_DIR` at a throwaway directory before Start. A `cargo
  run` finds a runner to start only once `cargo build -p dispatch` has
  put `dispatch` beside the app binary; otherwise Start is refused.
- `SWITCHBOARD_SCRIPT=<file>`: run actions at startup, one per line, so
  the app can be put into a known state without clicking. See
  `src/script.rs` for the lines (`add-project`, `new-shell`, `new-claude`,
  `new-codex`, `new-service`, `show-board`, `show-session`, `show-document`,
  `show-file`, `files`, `side-position`, `terminal`, `select-file`,
  `set-env`, `set-secret`, `dotenv`, `environment`, `config`,
  `env-setup unlock|lock`, `pairing <code>|close`, `remote-devices`,
  `remote-listen lan|loopback <port>`,
  `send`, `interrupt`, `return`, `kill`, `remove`, `approve`, `revoke`, `side`,
  `switchboard`, `working-set`, `new-working-set`, `clone-working-set`,
  `rename-working-set`, `delete-working-set`, `new-recent-set`,
  `set-hours`, `running-only`, `card-scale`, `dismiss-from-set`, `add-to-working-set`,
  `add-file-to-working-set`, `arrange`, `show-message`, `clone-session`,
  `discard-to`, `undo-discard`, `review-plan`, `show-review`,
  `show-dispatch`, `dispatch-runner-start`, `dispatch-runner-stop`,
  `show-ticket`, `close-ticket`, `ticket-tab` (`<id>` and one of
  `timeline`, `issue`, `plan`, `notes`, `review`, `changes`, `usage`), `ticket-diff`,
  `review-file`, `review-continue`, `review-object`, `review-finalize`, `show-artifact`,
  `pop-out`, `close-pop-out`, `files-root`, `zoom`, `place-pop-out`,
  `place-card`, `new-workspace`, `workspace`, `workspace-global`,
  `move-project`, `move-working-set`, `controller`,
  `open-terminal`, `prompt-box`, `theme`, `sleep`).
- `SWITCHBOARD_CONTROLLER=<device>`: the nunchuk's serial port (default: the
  `/dev/cu.usbmodem*` whose USB product name is Feather or MagTag, waited
  for if absent; no other board is ever opened, since a port that never answers
  blocks the process in the kernel at exit).
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
src/bin/switchboard-env.rs   runs one child with a session's environment sets, resolved over the control port with its launch token; the sets' setup commands (std and the wire crate only)
src/bin/switchboard-ask.rs   marks the session it runs in as asking the owner a one-line question, or a confirm, a choice or a line of text whose answer comes back as its next prompt, or clears it, over the control port with its launch token (std and the wire crate only)
src/lib.rs               module tree and the layering rules
src/core/
  model.rs               durable data model (Project, SessionRecord, ResumeHandle, CardState)
  action.rs              AppAction, Effect, Clock, AppCore::dispatch, read model for the UI
  reconcile.rs           StoreLoaded / HostListed: card states, autostart services, spawn specs
  sessions.rs            launch, idempotent return, resume preflight, Codex serialization
  grid.rs                Working Set placement: default card sizes, first free spot, overlap, minimum size, the card a step away, a rule set's card size at its scale and flowed layout
  controller.rs          the hand controller's meaning: the selected card per working set, Z's radial menu, the session C holds open
  definitions.rs         .switchboard/project.json entries -> records; hash-keyed approval
  env.rs                 environment resolution for a new session: global variables, opted-in .env files, the project's own; secrets by account name
  file_refs.rs           file paths in an agent's prose as links: finder, rewrite given which name a file, per-message cache
  events.rs              hook events -> record activity (matched by record id, ordered by time)
  workflow.rs            plan review runs: reviewer and planner rounds as a state machine over records
  control.rs             the control port's commands run quietly under an operation id; read models in the wire's shapes; `ready_for_prompt`, the one verdict `session.prompt` and a view's `prompt_refusal` share
  remote.rs              the remote port's state: the host's identity and paired devices as loaded, the pairing window with its code, scope and failure count, a device's scope and revocation; `invite_url`, the page's URL with the code, key and id in its fragment that the QR carries
  updates.rs             the app's version and its update: the scheduled check, a newer release offered, downloaded on a click and installed on another; semantic versions, pre-releases never offered
  tickets.rs             Dispatch's tickets as the remote port says them: each ticket's summary with its standing worded as the Mac's page words it, its decisions, and alone its body, attempts and lanes; one value for the port's fingerprint and its replies
  revision.rs            the read-model fingerprints behind the remote port's revision: every exported view hashed with its time-derived fields held still, and the `changed`/`removed` diff between two passes (and a record's runs as the `runs` collection)
  diff.rs                a changed file's diff as held per ticket, lane and path, when a re-read is due, which unchanged runs fold
  dispatch.rs            Dispatch as the app shows it: the runner's last status as views, decisions answered as calls, a ticket's events and full view read once per change and its timeline, a project's supervisor chip, the console session
  tests.rs               state-transition tests for the core
src/ports/               traits: store, host, events, agent, opener, transcript, secrets, project_config, round_files, artifacts, changes (a branch's commits and files over its base, one file's diff), controller, control (the operations log), dispatch (Dispatch's port), remote (the identity and paired-device files, and the Bonjour advertisement), updates (the release source: the newest release, its bundle fetched and checked, the swap arranged)
src/adapters/
  store.rs               JSON store: atomic writes, .bak, flock
  updates.rs             GitHub Releases over `ureq`: the latest release parsed from the API, the zip staged under `$TMPDIR/switchboard-update` and checked against its `.sha256` asset, unpacked with `ditto`, verified with `codesign` and pinned to the running app's Team ID; the swap helper that replaces the bundle after exit and relaunches, keeping the old one aside
  tmux.rs                tmux process host on the private socket
  hooks.rs               append-first event log + socket wake-up + hook settings JSON
  control.rs             the control port: operations.log (append-only, synced) and control.sock (one thread per connection, requests handed to the frame)
  remote.rs              the remote port: identity.json and devices.json, the certificate and key (`rcgen`), the pinned client a device uses, and the TLS listener (`rustls`, one thread per connection, pools of 4 unauthenticated (3 per peer address, counted by a drop guard that frees the slot however the connection ends) and 12 paired, absolute deadlines before pairing (5 s handshake, 5 s head, 10 s body, enforced on each socket read beneath TLS) and the 30 s idle timeout after, every reply before pairing saying `Connection: close` and closing, `/pair`, `/info` and page rate limits, requests handed to the frame 8 per pass, the page's files answered on the connection thread and the connection closed after each; a browser's `Cookie`, `Origin` and `Host` kept on the request, extra reply headers such as `Set-Cookie`, and `no-store` and `nosniff` on every JSON reply); a connection the frame accepts an upgrade on becomes a WebSocket subscriber (`tungstenite`, non-blocking, fed from a queue of 256, `resync_required` then the close on overflow); a subscriber's own text frames (`watch`, `unwatch`) and its end go to the frame as `SubscriberFrame`s; the request head read under an 8 KiB bound, a subscriber's inbound frame under 16 KiB
  web.rs                 the remote port's web page: the three files of `web/` built in by path and type, the content security policy and the headers every page file carries
  bonjour.rs             `_switchboard._tcp` over `mdns-sd` while the remote port is on the local network (macOS; a stub elsewhere): the display name with a short id suffix, TXT `v`, `name`, `id`
  dispatch.rs            the Dispatch port's client over <Dispatch data dir>/dispatch.sock, reconnecting after an error; the `dispatch` beside the app's binary
  dock.rs                Dock badge with the waiting-session count (macOS)
  controller.rs          the nunchuk over USB serial: a thread owns the port, reconnects, hands events over a channel
  files.rs               project file index: gitignore-aware scan, lazy children, fuzzy match; the existing file a path an agent typed names; `open_pinned`, the descriptor walk the remote port reads a pinned file through (no symlink followed, a FIFO refused, dotenv refused, the version from the inode, change time and size)
  git.rs                 branches, change counts, per-path status; finds repos one or two dirs down; a branch's commits and numstat over its base, one file's diff (`GitChanges`)
  diff.rs                one file's `git diff` read into numbered hunks, changed words marked with `similar`
  keychain.rs            secrets as generic-password items in the login Keychain (tests use a temp keychain)
  dotenv.rs              .env parser (opt-in per project) and .env.example names
  project_config.rs      reads and validates .switchboard/project.json (capped, no symlinks)
  round_files.rs         a workflow's round files on disk: probe, snapshot into the data dir, delete
  scrollback.rs          read the pipe-pane stream back as plain text (cold sessions); the masked tail with its overlap and the log's version for the remote port
  artifacts.rs           a run's declared outputs: glob under the cwd, modified since the run began
  agents.rs              Claude Code / Codex launch, resume, preflight, discovery
  transcript.rs          Claude Code transcript (JSONL) -> Conversation turns and token totals per model, one per `message.id`, `<synthetic>` skipped; `read_masked` runs the mask over every text before the parser's caps
  ghostty.rs             open, reveal, Ghostty window launch and raise
  fakes.rs               test doubles for every port
src/app.rs               SwitchboardApp: owns core + adapters; runs effects; polls host and events; answers control-port queries (`session.screen` with the project's secret values redacted); serves the remote port (the views on the frame, a screen through the content worker with the policy resolved on the frame, the watch set, the revision per content key, and the control scope's `send`, `resume` and `trust` through the control socket's `serve` so the op is the idempotency key; a browser's token as the cookie named for the server id, a cookie-only write or upgrade held to the page's `Origin`, `/pair` with `transport: "cookie"`, and `/me`)
src/content.rs           the app's content caches and the port's content worker: parsed transcripts with the handle and file time each was read at (`ConversationCache`), read by the views through `DrawCtx`; `ContentWorker`, the one thread that reads content for a device (a pane's full window hashed as the source version; a Claude Code transcript parsed masked and kept on the worker by handle, file version and policy, served as a page of turns by number with the head of each tool call's text, as one turn whole, or as the last response, the three sharing one key; a run's log tail with the overlap that masks a value on the window's edge; a pinned file whole, cached by its version) and polls the watched keys every two seconds; the policy resolved on the worker from what the frame hands it (`PolicySpec`, `resolve_policy`), cached per spec; `redact`, the content keys, the policy with its fingerprint, the page, turn, tail and file caps, the versions and the watch set's bounds (128 keys, a minute idle)
src/script.rs            SWITCHBOARD_SCRIPT dev aid
src/ui/
  mod.rs                 UiState, draw loop (collect actions, then dispatch), keyboard, side panel tabs
  prompt_box.rs          the Prompt Box editor per agent session, one voice runtime bound to one of them; the draft a device reads, its tools and its dictation
  theme.rs               the look: color tokens per theme, Source Serif 4, type scale, shared widgets (dot, kicker, buttons)
  rail.rs                left project rail (brand, All sessions, Dispatch with its pending decisions, projects with dots, Go to, Settings); a session's neighbours beside it; its list scrolls between a fixed head and Go to / Settings
  switcher.rs            Settings menu and the toasts (notice, host error)
  updates.rs             the version and the update's state in the Settings menu, and the rail's item while a release waits on a click
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
  remote.rs              Remote devices dialog: the host's name, id, key and address, the pairing window with its QR code (the invite URL; a Mac-only port says so), what a browser's pairing trusts, the paired devices with scope and Revoke
  config.rs              project config editor: .switchboard/project.json as text, options listed, parse shown
  workflow.rs            plan review: the Review plan dialog, the run's page (rounds, plan with diff, feedback beside response, controls), the feedback block pinned under the header
  dispatch.rs            the Dispatch page: tickets with what waits on you (filtered by project), each project's supervisor chip (Open, Resume, Trust, Fresh behind a confirmation), and the console; Pop out
  diff.rs                a changed file's diff on the ticket page: highlighted hunks beside a line-number gutter, changed rows tinted, folds, a new-side number opens the file
  ticket.rs              one ticket's page: header, pending decisions pinned, tabs for timeline, issue, plan with its review rounds, notes, code review, branch changes read on a thread with a file's diff under its row, and usage (tokens by model, then each attempt's sessions with their time, read from the runner once per change of the ticket); the owner's feedback on a pending finalize pinned above the plan
  cards.rs               the one card for every entry kind, the card grid, pinned document cards
  session.rs             session view: header, embedded terminal or conversation + message box
  switchboard.rs         every session across projects, waiting first
  working_set.rs         a working set: the user's grid of session and file cards from any project, or a rule set's cards with its hours field and dismiss controls
  dialogs.rs             add project / create session dialogs, the full-message and links-in-message dialogs
assets/fonts/            Source Serif 4 (Regular, Semibold, Italic; OFL), embedded by theme.rs
web/                     the remote port's web page, no build step and no dependency: `index.html`, `app.css`, `app.js` (pair from the invite URL's fragment into a cookie, list sessions live by default and waiting first, narrowed by state, kind, project and a name search and sorted four ways, the choices kept in `localStorage`, a Claude Code session's conversation by turn with earlier turns on request or any other session's screen, a reply under `control`, polling hastened by the events socket; its routes in one table the host test reads), and `app.test.js` (its pure parts under `node --test`, run by `adapters::web`'s tests: without node they fail on CI and print `SKIPPED:` elsewhere)
tests/ui.rs              headless flows via egui_kittest with fakes
tests/control.rs         the control port over a real socket with fakes: ops on records, logged replies, interrupted launches, a non-replayable command logged before it runs and answered `interrupted` on a retry, a pane's screen with secrets named
tests/remote.rs          the remote port over a real TLS listener with fakes (the control scope: a read device refused by name, a send typed once per op and a repeat answered from the log, a resume launching once, trust answering the folder question once, the bad bodies; a conversation paged by turn, from `last` or `before` a held turn, a page carrying the head of a tool result and the turn alone carrying it whole, its last response at the conversation's revision, a shell refused, a watched conversation moving as the transcript grows; a command's runs, a run's output masked, a missing run refused, a watched run output moving as the log grows): /info to anyone, pairing with the host's key and code, a token that reads every view, revocation, scope, the allowlist, bounded strings, a redacted screen, a missing key remaking the identity, Bonjour following the listen preference, the events socket (`hello`, `changed` and `removed` from the diff, a rename and a trust question with no save, ten quiet passes emitting nothing, the event and the snapshot at one revision, overflow and revocation closing the socket), watched screens (`watch` answered `watching`, the poll emitting `changed` when a pane or its policy moves and nothing while it holds still, two reads of one text at one revision, the cap evicting the least recently read, `unwatch`, a closed socket and a revocation dropping their watches), stalled strangers filling only their address's share, a fresh paired connection beside two of them, every reply to a stranger closing its connection while a paired one stays open, a head with no newline closed early with no reply, an oversize frame ending a subscription, `/ops/{op}`, the fingerprint pass timed, the web page (its files and headers to anyone, its route table served, a browser's pairing into a cookie named per host, two hosts' cookies kept apart, the `Origin` rule on writes and upgrades, `/me` and the revoked-cookie inference, a page load freeing its slot, the page's rate window), and the client fixtures (`client/Fixtures`, rewritten with `SWITCHBOARD_WRITE_FIXTURES=1`)
tests/fixtures/          a small real Claude Code transcript for the parser tests
tests/browser/           the browser half of `tests/remote.rs`'s ignored `the_page_works_in_a_real_browser`: `page.mjs` drives the web page in headless Chromium step by step as the host answers, with Playwright pinned in `package.json` and `package-lock.json`
tests/live.rs            ignored: real claude / codex / Ghostty runs
tests/gate.rs            Milestone 1 gate: real store, tmux, hooks; agents ignored
tests/changelog_fold.rs  scripts/fold-changelog.sh on a scratch changelog: the old insertion with no fragments, the groups' order, Unreleased lines kept, fragments removed, a bad line changing nothing
scripts/test-times.sh    every test's time, serially, through libtest's `--report-time`; the slowest ten, each binary's total, the 5 s budget
scripts/ci-test.sh       CI's test step: the workspace, then one retry of the failed tests, warning `flaky:` for each that passes on it
scripts/bump.sh          the next version into Cargo.toml and Cargo.lock, and the changelog fragments and Unreleased section dated under it
scripts/fold-changelog.sh  the fold bump.sh runs: `changelog.d/` fragments grouped Added, Changed, Fixed into a new version section of CHANGELOG.md, then removed; every line checked before anything is written
pipelines/Switchboard.toml  this project's own Dispatch pipeline, the plan trial's result: haiku investigates, a sonnet facts stage settles the open questions with web search or a spike, a sonnet plan with a two-round review, then implement, code review and the pull request; a supervisor block for a Mac that runs the released app. Installed by copying it to `pipelines/Switchboard.toml` in Dispatch's data directory (a ticket runs the copy it froze at take); parsed and validated by a test in dispatch/src/pipeline.rs
scripts/release.sh       the signed bundle and its SHA-256 attached to a GitHub release tagged `v<version>`, with the changelog section as notes; what the app updates itself from
CHANGELOG.md             every released user-visible change by version; written only by a bump
changelog.d/             one fragment per branch, `<branch>.md`, of `Added:`, `Changed:` or `Fixed:` lines, folded into CHANGELOG.md by a bump (`README.md` there has the format)
client/                  the Swift client (docs/remote-client.md §7): `SwitchboardProtocol` (the wire views as Codable, qualified refs, the envelope, the event frame and resource keys), `SwitchboardTransport` (HTTPS and the events WebSocket with the SPKI-pinning delegate, Bonjour browsing), `SwitchboardClientCore` (known servers, credentials, one connection actor per host that subscribes before it snapshots and reconciles events by revision, holds reference-counted screen watches that are resent after a reconnect, the unified store), `SwitchboardUI` (SwiftUI browser and server views), the `switchboard-remote` command-line browser, tests, `Fixtures/` shared with `tests/remote.rs`, and the apps: `project.yml` (xcodegen writes the git-ignored Xcode project from it), `Apps/SwitchboardVision/` (the visionOS entry, thin over `SwitchboardUI`: the main window with list, grid and focus layouts and the filters, the card window group, the prompt window and the keys panel placed as the virtual keyboard is), `scripts/sim.sh` (build, run and screenshot on the simulator), `scripts/device.sh` (build, install and launch on a paired headset)
control/                 switchboard-control: the control port's wire contract (requests, replies, views, a session's transcript path among them; `session.resume` resumes an agent without a terminal and never launches fresh; `session.prompt` types a prompt only into a Claude Code agent between turns, else fails naming why) and a blocking client; std + serde only
dispatch-control/        dispatch-control: the wire contract of Dispatch's own port (tickets as views, a ticket's events after a cursor, decide, queue, take, resume, close, worktrees, a project's supervisor and `supervisor-fresh`, a ticket's `usage`: tokens by model and each session's time), how a bring-up reads, and a blocking client; std + serde only
dispatch/                the `dispatch` binary (docs/dispatch.md; docs/dispatch-agent-guide.md is the command-line guide for agents that take tickets; docs/dispatch-pipeline-guide.md builds a pipeline file up step by step, every TOML block in it checked by a test; docs/dispatch-pipeline-improvements.md is the open list of pipeline changes drawn from tickets run so far): a ticket scheduler that drives Switchboard over the control port and never links the app
  src/main.rs            CLI: take, run, runner, decide, decisions, status, queue, park, resume, close, restart, worktrees, supervisor; for a supervising agent, brief (with each subscription's undelivered lines), subscribe, unsubscribe, subscriptions, events (with `--follow --timeout`), wait, show, report, tail and health (usage errors, and a command the ticket's state never allows, exit 64); a supervisor session's command checked against what it may run first
  src/events.rs          the event log `events.jsonl`: what a ticket write changed (`between`, pure), appended and synced before the record's rename, a `void` for a write that failed; a bring-up's `by` and `conflicts` as the scheduler derives them; read, followed, and waited on with each match checked against the record, and held while its write is in flight; `next_burst`, the settled burst a wait and a delivery share; `line`, `clock` and `decide_hint`, how a line prints
  src/report.rs          how a ticket went, pure: stage time, the plan per lane, plan and code review points counted by id, fix passes, rebases, the PR's size, tokens by model per stage and ticket from `usage`
  src/usage.rs           tokens by model and time in turns per session, from Claude Code's transcripts: `scan` (one message per `message.id`, subagents folded in), `sessions_of` (every session a ticket names, with its window and kind), `attribute`; the `usage.json` cache rescanned with no lock held and merged by `read_ms` under it
  src/health.rs          the runner's call counters, written as `runner.json` after every pass (also read by `dispatch runner`), and the `health` check of that file and both sockets, repeated failures folded
  src/runner_cmd.rs      `dispatch runner stop|start|restart`: refused while a ticket runs a deploy, asked of the app over the control port under the writer lock, then waited for in `runner.json`
  src/serve.rs           Dispatch's port on <data>/dispatch.sock while `run` is up: the records as views (a per-lane plan once per lane), the commands the CLI has, a ticket's usage from its cache brought up to date, one handler under one lock
  src/pipeline.rs        the TOML pipeline file, parsed in full and validated; fingerprint of the copy a ticket runs
  src/ticket.rs          the ticket record: source, lanes (with a conflict seen and not yet brought up, and their last bring-up, `Refreshed`: whether the branch had commits, when, the rebaser's notes, the conflict it resolved and the head it reached), attempts (with their checks' head and exit, the attempt a review carries and its send-back note, the history rewrite a review attempt made, the agent kind it launched (`Attempt.agent`) and a plan review's sessions by role (`Attempt.run_sessions`)), decisions, each session's transcripts as Switchboard named them (`Ticket.transcripts`, `TranscriptRef`), operation ledger, a close's progress (`CloseProgress`), restarts with the heads recorded at each stage entry, the tree's last bring-up, also kept at each stage entry, holds and the services a ticket runs, who took, parked, resumed or closed it and the supervisor's refused answers and refused commands; the per-project queue and closing list, the project's supervisor (`Supervision`, with its `Subscription`s and the `Delivery` in flight) and its base tree
  src/scheduler.rs       the runner: stage executors (agent, gate-only lanes, gate-only command stages (a deploy), review workflow), completion evidence, decisions, ledgered sends, parking, closing and restarting as sequences (a deploy lost to a restart waited for, never signalled); bringing a lane up to its moved base, recorded with the rebaser's notes; bringing the ticket's tree up to its base when no chosen lane lives on its branch; a deploy stage whose lane the ticket did not choose deploys the lane's base from the project's shared detached tree; a conflicted bring-up after the last code review gets one resolution review; who brought a lane up and how many commits conflicted; resource holds (taken before a lane is brought up; only the first stage of a `needs` run brings lanes up) and services stepped through src/services.rs; a retake's kept branches deleted or asked about at the cut; a provider lagging Dispatch's own push waited on, not asked about
  src/services.rs        resource holds taken on entering each `needs` run (taken again past its first stage, the run's deploy and tester are asked about again) and released the pass after leaving it; who holds it and from which stage; a stage's services: the lane's before, a free port, a Switchboard service session, the readiness probe; stopped when the ticket leaves the resource's run, parks or closes, with the `stuck` question past the stop limit; a `retry` restarts only those confirmed stopped
  src/review.rs          the code review stage: rounds of several reviewers over a lane's branch, a fresh implementer per round, checks at every accepted head; a rerun carries the last attempt's settled and open points; style and the plan's decisions (every lane's, for a joined review) do not hold a round open; a reviewer's line that every point left is wording is a note, not a point; folds the fix rounds into the commits they amend as it completes, or keeps them when the user answers `keep` to a failed fold; checks each folded message's backticked names against the commit and the tree, and asks `rewrite | accept | park` when one is gone, a rewrite rewording the folded commits with the tree unchanged; the one-pass resolution review (docs/dispatch.md, "The code review stage")
  src/subscribe.rs       a supervisor's subscriptions on the project's record: `subscribe`, `unsubscribe`, what each has not delivered; the runner's own subscription at `take` and on each pass to every Active ticket not yet followed (`subscribe_active`), from the tail or one before a waiting `decision` line; the runner's pass that types every subscribed ticket's settled burst (`events::next_burst`) into `supervisor.current`'s pane through `session.prompt` once the session view's `prompt_refusal` is empty, the delivery written first and resent under its op after a lost reply, why it waits kept on the record
  src/supervisor.rs      a project's supervisor session from the live `[supervisor]` table: its seed and the hash that says it is stale, its launch flags, who runs a command (`SWITCHBOARD_RECORD_ID`) and what a supervisor may run (`SUPERVISOR_VERBS`) and where the owner answers what it may not (`OWNER_ROUTES`), the capabilities a table's `may` gives (`runner` and `restart`) and the refusal kept on the project's record without one; the runner's fresh (workspace setup, seed, the old session killed and kept, the hand-off rotated), kill and resume through a ledgered request on the project's record
  src/restart.rs         `dispatch restart`: a ticket put at its stage or an earlier one under a fresh copy of the live pipeline, applied by the park sequence; branches reset to the heads recorded at stage entry, heads needed only for branches that moved; a stage only the live file has may be the target; later work discarded, changed setup run again; a pull-request ticket ranged only when no branch moved; a note for the next agent
  src/recover.rs         unanswered ledger operations resolved by class through find and op.status; nothing launched twice; and the project's queue-set request, a lost `set.new` found and a lost `set.sync` superseded by a redraw
  src/view.rs            the `Dispatch · <project>` working set, one card per queued ticket, redrawn through set.sync; its requests on the project's record
  src/store.rs           Dispatch's data directory, flock, atomic writes; versioned `read_ticket`/`read_project` through `migrate`; `read_usage`/`write_usage` for a ticket's `usage.json`, versioned apart by `USAGE_VERSION`; `write_ticket_stamped`, the one ticket write, which logs its events first, moves `updated_ms` only on a change other than poll bookkeeping and skips an equal record
  src/git.rs             clones (with the project's `.githooks` as `core.hooksPath`), worktrees and heads through fixed argv, running no hook itself (a branch pushed only with a lease; a worktree removed, never forced; a detached worktree; `changes`, `uncommitted` for a close's preflight, and `tree_changes` for a tree's own dirt, its nested lanes left out), a stage's checks as child processes polled by key (confined, when the pipeline says so, through `Confine`), commits replayed with `merge-tree`/`commit-tree`, the commits whose rebase conflicts and the files a merge with the base conflicts in read the same way, a branch moved by compare-and-swap, whether the remote already holds a branch's commits, and whether a port is free and answers HTTP; a kept branch read, deleted or renamed out of a retake's way; a range's commits and diff size for a report; which names a commit's diff and a tree lack (`absent`); with a fake
  src/confine.rs         a pipeline command's confinement: the seatbelt profile and wrapper built everywhere, used on macOS; elsewhere the command runs unconfined and says so
  src/child.rs           the one place a runner child's `Command` is built, with the launch token removed
  src/history.rs         which commits a code review's fix rounds fold into: pure plans for `fold` and `one`; the backticked names a folded message is checked for, and how a name is found in a diff or a tree
  src/github.rs          issues and pull requests through gh, with fakes; a merged pull request's merge commit and the check runs on it (the base pipeline)
  src/bitbucket.rs       pull requests on Bitbucket Cloud through curl, conflicts read from the pull request's diffstat, the merge commit and the base pipeline's statuses or a named step on it; credentials from the environment or <data>/env (NAME=value lines, mode 0600, never logged)
  src/port.rs            the Port trait over the control socket client; the connection remade after any error, a timeout included; `path()` names the socket for `health`
  src/template.rs        `{a.b}` substitution for prompts
  tests/first_slice.rs   the acceptance table against an in-memory Switchboard (tests/support)
  tests/supervisor.rs    a project's supervisor made, replaced, killed, resumed and recovered against the in-memory Switchboard; through the CLI with `SWITCHBOARD_RECORD_ID` set, what a supervisor may and may not run; subscriptions delivered into its pane: held until settled, resent under the op after a lost reply, waiting on a busy or prompted pane, parks once, closes ending them, an older app, a Fresh inheriting the backlog
  tests/live.rs          ignored: the first stage against a real Switchboard and a haiku agent
  tests/sandbox.rs       ignored, macOS: a confined gate's refused write fails the attempt and is named in checks.log
vendor/egui_term/        embedded terminal widget (Harzu/egui_term @ 31bbc7ab, egui 0.36; see SWITCHBOARD-PATCHES.md)
firmware/nunchuk/        CircuitPython for the Feather that reports the nunchuk's buttons and stick
docs/research/           answers to one research question each, every claim cited to a file and line
spikes/                  Spike 0 evidence, and later spikes such as 13-dispatch-service (a Dispatch service as a Switchboard service session; which bind sees a dev server's port) 15-paste-buffer (a multi-line send as one tmux paste, bracketed only for a pane that asked) 16-base-pipeline (a merged pull request's merge commit and the base's run on it) 17-prompt-origin (telling a typed prompt from a task notification or a harness reminder) 18-tmux-argv (keeping spawn's environment values off every process's argv) 19-structured-ask (a card's answer reaching the pane as its next prompt) 20-stop-pending (what a Stop lists as background work and wakeups still in flight), 21-evidence-writes (which writes reach an evidence directory without a permission prompt) 23-web-page (what a browser's page meets on the remote port: the cookie and socket questions, not run, and five parallel connections against the four-slot pool), 24-headless-browser (headless Chromium keeping the `__Host-` cookie and connecting `wss://` on a self-signed IP-literal host) and 25-hooks-path (which git commands run which hook under `core.hooksPath`, and `-c core.hooksPath` to a missing directory running none)
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
A confined check's exit gets the longer `poll_within` deadline in
`git.rs`, because the sandbox wrapper's `log show` queries ride on it.
Dispatch's integration tests skip the device flush
(`store::skip_fsync_for_tests`) and move time with `Env::wait` rather
than stepping through it.

Tests that hit real services (a model, a paid API) go in their own
`tests/*.rs` file, marked `#[ignore]`, with the command to run them
documented here.

### Pre-commit hook

`.githooks/pre-commit` checks formatting, runs Clippy, and runs the tests
before each commit, including the vendored terminal widget's tests, which
`--workspace` does not reach because `vendor/egui_term` is excluded. It
never modifies or stages files. Enable it once per clone:

```sh
git config core.hooksPath .githooks
```

### Secrets

Local secrets go in `.env` (ignored by git). Read them from the app at
startup; never bake them into the binary or commit them.

## License

MIT, see `LICENSE`.
