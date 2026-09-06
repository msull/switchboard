# Switchboard

## Status

Design draft, 2026-09-05. Revised the same day to put persistence at the
center, then again after Spike 0 and an external review, which added the
durable store contract, the trust boundary, the host lifecycle (agents
always inside tmux, Ghostty attaches), reconcile-on-start, and the
Milestone 1 gate.

## The problem

A good workspace takes real effort to set up: named agent sessions
mid-task, a dev server, a few shells in the right directories. Those get
revisited weeks or a month later. Today they live in terminal
multiplexers, and a reboot, an app update, or a crash loses all of it.
Finding, previewing, and opening the files a session is working on means
leaving for Finder or an editor. And nothing shows, across every project,
which agents are waiting for a decision.

Switchboard is a tool with persistence as the first design constraint, a
file browser beside the sessions, and one view over all of them.

Prior art: Cmux (a macOS multiplexer on embedded Ghostty, aimed at agent
sessions) has the per-project workspace and side-by-side sessions, and
recent versions have grown some session restore. It is GPL and its control
socket is closed to outside processes by default. Switchboard does not
integrate with or embed it; it is a reference for what the session side
should feel like, nothing more.

## The core idea: workspaces are data, processes are a cache

Nothing survives a reboot except what is on disk. So a workspace is never
"a set of running processes." It is a **description** on disk of what
should be running, plus everything needed to bring each piece back:

- for a shell: its directory and environment profile;
- for a command or service: its command line, directory, env profile;
- for an agent session: all of the above plus the agent's own **resume
  handle** (Claude Code's session id, for example), the name you gave it,
  and your notes about where it was;
- for every session: the scrollback, on disk, so what happened is
  readable even when nothing is running.

Running processes are a cache of that description. Switchboard starts
them on demand, reattaches to them if they are still alive, and writes
enough back to the description (last seen, exit code, updated resume
handle) that the next start picks up where the last left off. Quitting
Switchboard, restarting it, or rebooting the machine changes only whether
the cache is warm.

This is the discipline the whole app follows: every feature asks "what is
the on-disk record, and how is it rehydrated?" before "what does the pane
look like?"

## Priorities, in order

1. **A workspace comes back.** After quit, crash, update, or reboot, every
   project shows its sessions by name with a one-click return. Agent
   sessions resume their conversation where the agent supports it. A
   month-old workspace is as usable as one from this morning.
2. **Fast to the file.** Open a project, find a file, read it, open it
   elsewhere: seconds, keyboard-driven.
3. **One place for the running things.** What is running, in which
   project, since when, is it healthy.
4. **Stay out of the way.** Switchboard organizes; it does not wrap or
   reinvent the agents, editors, or shells. If the native tool is better
   at something, hand off to it.
5. **Portable core.** The workspace model and session registry do not
   depend on egui, so a CLI or another front end can reuse them.

## Concepts

### Project

- `name`, `root` (absolute path), optional `notes` (Markdown), optional
  tags for grouping (client, personal, code).
- Concrete projects: one per client; one each for Prompt Box and
  Switchboard; one for the PTA role; one for Cub Scouts.
- **Layout kinds**, detected rather than declared: `root` is itself a git
  repo, or `root` contains repos in subdirectories with notes and tooling
  beside them. Both are shown the same way; git decorations attach to
  whichever directories are repos.
- Project-local records live in `<root>/.switchboard/` so they travel
  with the folder. The global list of projects lives in the platform data
  directory. Both are plain files (JSON or TOML), human-readable and
  hand-editable, so a broken app never locks the user out of their own
  workspace records.

### Workspace record

One per project, the durable heart of the app. Contains the list of
sessions with, per session:

- `id` (Switchboard's own, stable, never reused), `name`, `kind` (agent,
  command, service, shell), `cwd`, `env profile`, `created`, `last seen`,
  `notes`;
- what to run: for launches Switchboard composes itself (agents), a
  structured `argv`; for user-authored commands and services, the command
  string plus the shell that runs it, stored as written;
- `host` (the tmux session or pane id while one exists; cleared when the
  pane is gone);
- `resume` (the provider's handle: Claude Code session UUID, Codex rollout
  id, or nothing). The three ids are distinct and never conflated: record
  id for Switchboard, host id for the process, resume id for the agent;
- `autostart` for services, honored only when the record is trusted (see
  "Trust boundary");
- `layout` hints (card position and grouping on the board) so the view
  comes back as it was;
- `scrollback` and transcript backup paths.

Also holds the project's pinned documents.

### Durable store

The store is the product, so its mechanics are fixed here rather than
left to the implementation:

- **Format.** JSON, one file per project record plus a global project
  index, each with a `schema_version` integer. Readers accept older
  versions and migrate forward; the app refuses to write a version it does
  not understand.
- **Location.** Private runtime state lives under the platform data
  directory (`~/Library/Application Support/Switchboard/`), file mode
  `0600`, directories `0700`. Nothing executable or secret is read from a
  project directory in Milestone 1; see "Trust boundary" for the later,
  optional shareable config.
- **Writes.** Every change: serialize to a temp file in the same
  directory, `fsync`, rename over the old file, keep the previous file as
  `.bak`. A crash loses at most the last change in flight.
- **Reads.** Validate on load. A truncated or unparsable file falls back
  to `.bak` with a visible notice, never silently to empty. A project
  whose root has moved is shown as *missing* with a relocate action; its
  record is never deleted automatically.
- **Single writer.** A lock file under the data directory; a second
  instance opens read-only and says so. External edits by hand are
  supported by re-reading on a file watch and reconciling, not by
  assuming the in-memory copy is authoritative.
- **Ids.** UUIDs generated by Switchboard. Renaming, moving, or resuming
  never changes a record id.

### Session kinds

- **Agent sessions** are the reason for the app. Naming is required at
  launch. Switchboard assigns or captures the agent's resume handle and
  runs the agent inside the process host. "Return to it" is idempotent:
  if the host pane is alive (warm), it attaches a terminal to it; if not
  (cold), it starts the agent's resume command inside a new pane in the
  recorded directory, then attaches. Two clicks never make two processes
  for one record. If the agent cannot resume, the session still comes
  back with its scrollback, notes, and directory, and a fresh agent can be
  started with the notes as context.
- **Services** are long-running commands with start/stop, a health line
  (running since, exit code if it died), and a log tail. Marked
  `autostart` or not; a workspace can bring its dev server back with it.
- **Commands** are one-shot, saved per project with a name, exit code
  recorded.
- **Shells** are plain terminals in a directory.

### File browser and preview

- Tree of the root, lazily loaded, with a fuzzy finder across the project
  honoring `.gitignore`.
- Preview pane: rendered Markdown, syntax-highlighted text, images, a
  size-capped fallback. Read-only.
- Actions: open in default app, open in editor, reveal in Finder, copy
  path; for directories, open a shell session there.
- Git decorations on rows: modified, untracked, branch on repo roots.

### Environment

- Env profiles per project, layered: global secrets (macOS Keychain,
  referenced by name) < project `.env` files < profile overrides.
- Sessions record which profile they use; the resolved environment is
  shown with secrets masked.
- Helpers: diff `.env` against `.env.example`; flag variables a saved
  command references but the profile does not define. Secrets are never
  written in plaintext anywhere the user did not already put them.

### Trust boundary

Switchboard executes commands from records, so where records come from
matters. A record that arrived via `git clone` or a branch checkout must
never run anything on its own.

- **Milestone 1: private state only.** Records live under Application
  Support and are written only by Switchboard on this machine. Nothing in
  a project directory is executed or even parsed as configuration.
- **Later, optional shareable config.** A committed
  `<root>/.switchboard/project.json` may carry saved commands, service
  definitions, env profile names (never values), and pinned documents. It
  never carries transcripts, secrets, or live-session state. Its contents
  are shown, not run, until the user approves them; approval records a
  hash, and any change or new entry requires approval again. Autostart
  from shared config is never honored without that approval.
- **Secrets** come from the Keychain and from `.env` files the user
  already owns; Switchboard never writes them elsewhere. Scrollback and
  transcripts can contain secrets that a process printed; they are stored
  privately, capped, rotatable, and deletable, and the UI says so.

## How it looks

**Workspace switcher.** One project is active at a time. Switching is a
single keystroke or click: a strip or palette of projects, most recent
first, with a dot showing whether anything in it is running or waiting.

**The board.** The active workspace is a board of cards, not a tree of
tabs. Two kinds of card:

- **Session cards**, one per shell, agent, command, or service. Each shows
  the name, kind, directory, how long it has been running or since it was
  last seen, and a state derived from what the process is doing:
  *waiting on you* (the agent asked a question or stopped for approval),
  *working*, *idle at a prompt*, *exited* (with the exit code), or *not
  running* (the record exists, click to bring it back). The last line or
  two of output as a caption. Clicking a card opens the session; a
  keystroke gets back to the board.
- **Document cards** for pinned files: the design doc, a to-do list, a
  client brief. Click opens the preview; a modifier opens the default
  app. Any file in the browser can be pinned with one action.

Cards can be dragged into an order and grouped; the arrangement is part
of the workspace record and comes back with it. "Waiting on you" cards
sort or highlight first so the board is also a to-do list of agents that
need attention.

**The switchboard.** Above all workspaces sits the view the app is named
for: every session across every project that is running, waiting, or
recently exited, as one list of cards grouped by project and sorted with
*waiting on you* first. It is the morning start page and the "what did I
leave running" check before shutting the laptop. Clicking a card switches
to that workspace and opens the session. The dock badge counts the
sessions waiting on you.

**The session view.** The terminal for one session, with its metadata in
a sidebar (notes, env profile, resume handle, scrollback link). Never a
dead end: the board is one keystroke away, and the next waiting session
is one more.

**The file side.** The tree, finder, and preview from the concepts above
live in a panel next to the board, so pinning a document or opening a
shell in a directory is a drag or a click away.

## Spike 0: resumability first, rendering second

The spike exists to prove the core idea against the real agents before
any UI is built. Questions, in order:

1. **Can Claude Code sessions be captured and resumed reliably?** Confirm
   the exact mechanism: whether a session id can be assigned at launch or
   must be discovered from `~/.claude/projects/` after the fact, whether
   `--resume` works from a different terminal weeks later, and what state
   (permissions, working directory) it needs. Repeat for any other agent
   in use (Codex, etc.).
2. **What survives an app restart without a reboot, and is it worth
   keeping?** A tmux server keeps processes alive across Switchboard
   restarts. That is a nice warm cache, but it must not become the
   persistence mechanism, since it dies on reboot. Decide whether tmux is
   the process host (also giving reattach for free) or whether Switchboard
   owns PTYs directly and accepts that processes die with it.
3. **How is "waiting on you" detected?** The board depends on knowing a
   session's state without reading the screen. Candidates: Claude Code's
   hooks (a `Notification` or `Stop` hook can write a marker file or hit a
   local socket), its transcript files, the terminal bell, or as a last
   resort an idle-output heuristic. Find the cheapest signal that is
   right nearly always, per agent.
4. **How is a terminal shown?** Only after 1 to 3. Options: hand off to
   Ghostty or Cmux with a jump-to button; render tmux control mode panes
   in egui; or own PTYs with `alacritty_terminal` and an egui view
   (evaluate the `egui_term` crate first). Measure whether an
   egui-rendered terminal handles a full-screen TUI (a coding agent,
   `vim`) acceptably.

Deliverable: `spikes/resume/` with a README recording the mechanisms
that actually work, and a recommendation for the process host and the
terminal view. Likely shape: tmux as the warm cache and process host,
agents resumed through their own commands, terminal shown either in
Ghostty via hand-off or in egui depending on what question 3 finds.

## Spike 0 results (2026-09-05)

Four spikes ran in parallel; see `spikes/README.md` and each directory for
evidence. The decisions:

1. **Claude Code resume: solved.** Switchboard generates a UUID, stores it
   in the workspace record, then launches `claude --session-id <uuid>
   --name <name>` in the project directory. Return-to runs `claude --resume
   <uuid>` in the recorded cwd (resume works from any cwd but costs ~7x
   more elsewhere). Live status is also readable from
   `~/.claude/sessions/<pid>.json`. Codex has `codex resume <uuid>` but no
   launch-time id, so its id is discovered from its session files.
   **Retention risk:** transcripts are pruned after 30 days by default.
   Switchboard checks `cleanupPeriodDays` in `~/.claude/settings.json` and
   shows a warning card when it is unset, and independently backs up
   transcripts continuously: the transcript is append-only JSONL, so a
   file watch (or short timer) copies it into private state while the
   session runs, not only at session end, which would miss crashes and
   reboots. Cold resume restores the backup to the provider's path first.
   The restore path is unproven and is a Milestone 1 gate test. Codex
   retention is unknown and must be checked the same way.
2. **Process host: tmux.** One private tmux server (own socket, own config)
   hosts every session. Sessions outlive the app, a fresh process reattaches
   and reads history, `list-panes -a -F` gives liveness, pid, and exit code
   for the whole board in one ~3 ms call, and a control-mode client streams
   output for the session view. `pipe-pane -o` writes scrollback to disk
   and the raw stream carries escapes tmux strips from its own state.
   tmux 3.2 or newer is a hard requirement in Milestone 1: without it the
   app explains what to install and does not offer persistent launches.
   A `portable-pty` adapter implements the same `ProcessHost` trait for
   tests only; bundling tmux inside the app is the later answer (the spike
   showed it is feasible). Control-mode readers must be byte-oriented
   (chunks split mid-character).
3. **Session state: hooks first.** Claude Code hooks map cleanly onto the
   card states: `PermissionRequest` (also covers questions) = waiting on
   you; `UserPromptSubmit` / `PostToolUse` = working; `Stop` = idle;
   `SessionEnd` = exited. Switchboard ships a `switchboard-hook` binary
   that writes one line to a Unix socket and spools to a file when the app
   is down. Hooks are passed at launch via `--settings`, so nothing is
   written into the project. Correlation never relies on cwd: for Claude
   Code the session UUID is assigned by Switchboard and known before
   spawn; for every host pane Switchboard also injects
   `SWITCHBOARD_RECORD_ID` into the tmux environment so the hook helper
   reports the record id directly, which is how Codex and shells
   correlate. The spool file is rotated by atomic rename before draining,
   events are idempotent (replaying one is harmless), and every state
   derived from events is reconciled against tmux liveness. Hook-free fallback for Claude Code:
   the OSC 777 notify text ("needs your permission") from the pane's raw
   stream; tmux state alone cannot tell idle from waiting. Shells get
   idle/working and exit codes from Ghostty's OSC 133 shell integration,
   which works inside tmux. Transcript tailing is the last resort.
4. **Terminal view: hand off agents, embed the rest.** Agent sessions are
   shown in Ghostty, but Ghostty never runs the agent itself: it runs
   `tmux -L <switchboard socket> attach-session -t <host id>`, launched as
   `open -na Ghostty --args --title=<record id> --working-directory=<cwd>
   -e <that attach command>`, and the window is raised later by title.
   Closing the Ghostty window detaches; the agent keeps running in tmux
   with its liveness, scrollback, hook signals, and reattach intact.
   Shells, commands, and services render inside the app with
   `egui_term` (vendored from git; builds on egui 0.36 unchanged; renders
   `top`, `vim`, and Claude Code's TUI correctly at 1-3 ms per frame).
   Two patches needed: keys are dropped unless the pointer is over the
   widget, and `TERM` must be set explicitly. Cmux was surveyed and is
   not integrated (user decision). libghostty is not usable from Rust yet.

## Architecture

Follows the template layering. Nothing below touches egui.

- `core`: project registry, workspace records, env resolution, and the
  state machine for launching, watching, reattaching, and retiring
  sessions. Actions in, effects out, clock injected. Startup is a
  **reconcile**, not a launch: records plus the host's live pane list go
  in, and out come card states (warm, cold, exited) and launch effects
  only for trusted `autostart` services. Agents are never resumed
  automatically, since a resume costs money. The reconcile is pure and
  fully unit tested.
- `ports`: `Store` (records), `FileSystem` (list, read, watch), `Git`
  (status, branch, repo discovery), `ProcessHost` (spawn with env and
  cwd, attach, signal, stream output; tmux and PTY adapters), `Agent`
  (launch and resume command lines per agent kind), `StateSignals` (hook
  socket, raw-stream parser), `Opener` (default app, editor, Finder,
  Ghostty hand-off and raise), `SecretStore`.
- `adapters`: real implementations, each with a fake. The fake process
  host plays scripted output so UI tests never spawn anything.
- `app`: runs effects, drains worker channels, feeds results back.
- `ui`: workspace switcher, board of cards, session view, file panel.

## Milestones

0. **Resumability spike.** Done; see "Spike 0 results".
1. **Workspace records.** Projects, sessions as records on a board of
   cards, launch and return-to for agents via tmux with Ghostty attached,
   session state from hooks, and the cross-project switchboard view built
   from the same records. **Gate**, demonstrated end to end before any
   Milestone 2 work:
   - two Claude Code sessions in the same cwd map to the correct cards;
   - closing the Ghostty window detaches without killing the agent;
   - repeated "return" never duplicates an agent;
   - killing and restarting Switchboard preserves live processes and
     state;
   - a cold restart resumes from the backed-up transcript after the
     provider's copy is deleted;
   - a Codex session resumes from a discovered id;
   - a corrupt record recovers from `.bak` with a visible notice;
   - hook events produced while the app was down are neither lost nor
     misapplied. Restart the app,
   reboot the machine, everything is still listed and resumable. This is
   the product's reason to exist, so it comes before the file browser.
2. **Projects and files.** Tree, fuzzy finder, preview, open in default
   app and editor, reveal in Finder, pinned document cards.
3. **Commands and services.** Saved commands, services with start/stop,
   autostart, health, scrollback on disk.
4. **Environment.** Profiles, Keychain secrets, masked view,
   `.env.example` diff, missing-variable warnings.
5. **Git awareness and polish.** Decorations, sub-repo discovery, global
   quick-switcher, dock badge with waiting-session count.

## Open questions

- How much scrollback to keep per session? Full history for every
  command may be more disk than value; agents keep their own transcripts.
- When the shareable project config arrives, is a hash-and-approve flow
  enough, or should shared commands run in a visibly sandboxed way?
- Notes editor in scope, or "open in editor"? Leaning open in editor.
- Multiple machines: sync the project list early? Leaning no.
