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
   sessions resume their conversation where the agent supports it and
   for as long as the agent's own retention keeps it. Everything else
   about a month-old workspace, names, notes, directories, scrollback,
   is as usable as this morning's.
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
- All records, the project list and every workspace record, live in
  Switchboard's private data directory (see "Durable store"). Nothing is
  read from, or written to, the project directory in Milestone 1. A
  later, optional, committed `project.json` for shareable commands is
  described under "Trust boundary" and is never executed without
  approval. Records are plain JSON, readable and hand-editable, so a
  broken app never locks the user out of their own workspace.

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
  id, or nothing) plus the provider's `transcript` path as last seen, so
  availability can be checked without searching. The three ids are
  distinct and never conflated: record id for Switchboard, host id for
  the process, resume id for the agent;
- `autostart` for services, honored only when the record is trusted (see
  "Trust boundary");
- `layout` hints (card position and grouping on the board) so the view
  comes back as it was;
- `scrollback` path.

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
  directory, `fsync` it, rename the current file to `.bak`, rename the
  temp file into place, then `fsync` the directory. At every instant at
  least one of the current file and `.bak` is a complete, valid record.
  A crash loses at most the last change in flight.
- **Reads.** Validate on load. A truncated or unparsable file falls back
  to `.bak` with a visible notice, never silently to empty. A project
  whose root has moved is shown as *missing* with a relocate action; its
  record is never deleted automatically.
- **Single writer.** An OS-held advisory lock (`flock`) on a file under
  the data directory, released automatically when the process dies, so a
  crash never leaves a stale lock; a second instance opens read-only and
  says so. External edits by hand are
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
- **Scrollback.** Every session's output is kept on disk so reading what
  happened does not depend on the process still existing. Two files per
  session, both private:
  - the **raw VT stream** from `pipe-pane`, kept for the state fallback
    and for faithful replay of the recent past. Rotated by size: stop
    `pipe-pane`, rename the file, restart `pipe-pane` on a fresh file
    (the old `cat` keeps its descriptor, so a bare rename is not enough).
    Only the newest few chunks are kept. Replay starts from a chunk
    boundary, so a chunk begins with a reset and a snapshot of the grid
    taken at rotation time (`capture-pane -e`), which is what makes a
    truncated prefix safe;
  - the **readable text**, derived incrementally in the app by feeding
    the stream through the terminal parser and appending finished lines,
    the way a scrollback buffer would. This is what the history view and
    search read, and it is not subject to the raw cap; it has its own,
    larger cap and rotation.
  Each session has a delete action that removes both. Default caps are
  an implementation choice, exposed in Settings.

### File browser and preview

- Tree of the root, lazily loaded, with a fuzzy finder across the project
  honoring `.gitignore`.
- Preview pane: rendered Markdown, syntax-highlighted text, images, a
  size-capped fallback. Read-only. The bottom half of the file side
  shows the selected file; Expand opens it full size.
- The file side is always next to a board and can be toggled next to a
  session (Files button, Cmd+B), so a file can be checked without
  leaving the terminal or the conversation.
- Actions: open in default app, open in editor, reveal in Finder, copy
  path; for directories, open a shell session there.
- Git decorations on rows: modified, untracked, branch on repo roots.

### Environment

- Env profiles per project, layered: global secrets (macOS Keychain,
  referenced by name) < project `.env` files < profile overrides.
- Sessions record which profile they use; the resolved environment is
  shown with secrets masked.
- Helpers: diff `.env` against `.env.example`; flag variables a saved
  command references but the profile does not define. Resolved
  environment values are never persisted by Switchboard; process output
  may still expose a secret in the private scrollback (see "Trust
  boundary").

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
  already owns; Switchboard never writes them elsewhere. Scrollback can
  contain secrets that a process printed; it is stored privately, capped,
  rotatable, and deletable, and the UI says so. Agent transcripts stay
  where the provider keeps them; Switchboard reads but never copies them.

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

Deliverable (as planned; the actual spikes landed as four directories
under `spikes/`, see `spikes/README.md`): a README recording the mechanisms
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
   launch-time id, so its id must be discovered from the rollout file it
   creates, and that file records only cwd, not anything Switchboard
   injects. Two Codex launches in the same cwd at the same time are
   therefore indistinguishable by file alone. **Rule:** Codex launches are
   serialized per machine: a launch waits until the previous Codex
   launch's rollout file has appeared and been bound (the spike measured
   this in seconds), with a timeout that marks the record *id unknown*
   rather than guessing. A focused follow-up spike may find a
   deterministic pid-to-rollout mapping (for example via the process's
   open files) and lift the serialization.
   **Retention:** Claude Code prunes transcripts after `cleanupPeriodDays`
   (default 30). Switchboard does not copy or back up agent transcripts;
   the provider's retention is the user's choice and the limit of what
   can be resumed. Before offering a resume, Switchboard checks that the
   recorded transcript path still exists. That is a preflight, not proof:
   a resume can still fail (provider upgrade, corrupt transcript), and a
   failed resume gets the same treatment as a missing one. Either way the
   card says *not resumable*, keeps its name, notes, and scrollback, and
   offers a fresh session in the same directory. A one-time hint names
   the retention setting when the first pruned session is encountered.
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
   correlate. **Event delivery is append-first.** The hook helper always
   appends the event, with a wall-clock timestamp and a per-helper
   sequence, to a durable log under the private data directory; the
   socket is only a wake-up signal and carries no state. The app consumes
   the log from a checkpointed offset, so an event written just before a
   crash is replayed on the next start, and a duplicate replay is
   harmless. Each record stores the timestamp of the last event it
   applied; an older event (a spooled `Stop` arriving after a live
   `UserPromptSubmit`) is ignored, never applied out of order. The log is
   compacted by rotating it with an atomic rename once its offset is
   checkpointed. Liveness from tmux still overrides: a dead pane is
   *exited* whatever the last event said. Hook-free fallback for Claude Code:
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
  cwd, attach, signal, stream output; tmux adapter, plus a PTY adapter
  used only by tests), `Agent`
  (launch and resume command lines per agent kind), `StateSignals` (hook
  socket, raw-stream parser), `Opener` (default app, editor, Finder,
  Ghostty hand-off and raise), `SecretStore`.
- `adapters`: real implementations, each with a fake. The fake process
  host plays scripted output so UI tests never spawn anything.
- `app`: runs effects, drains worker channels, feeds results back.
- `ui`: workspace switcher, board of cards, session view, file panel.

## Milestones

0. **Resumability spike.** Done; see "Spike 0 results".
1. **Workspace records** (done, see status above). Projects, sessions as
   records on a board of cards, launch and return-to for agents via tmux
   with Ghostty attached,
   session state from hooks, and the cross-project switchboard view built
   from the same records. **Gate**, demonstrated end to end before any
   Milestone 2 work:
   - two Claude Code sessions in the same cwd map to the correct cards;
   - closing the Ghostty window detaches without killing the agent;
   - repeated "return" never duplicates an agent;
   - killing and restarting Switchboard preserves live processes and
     state;
   - a record whose provider transcript was deleted shows *not resumable*
     and offers a fresh session instead of failing;
   - a Codex session resumes from a discovered id;
   - a corrupt record recovers from `.bak` with a visible notice;
   - hook events produced while the app was down are neither lost nor
     misapplied;
   - two Codex sessions launched at the same moment in the same cwd each
     resume the correct conversation;
   - a hostile `<root>/.switchboard/` directory containing commands and
     an autostart service is ignored entirely.

   Restart the app, reboot the machine: everything remains listed, and
   agent conversations remain resumable only while the provider retains
   them. This is the product's reason to exist, so it comes before the
   file browser.
2. **Projects and files** (built 2026-09-06, see status below). Tree,
   fuzzy finder, preview, open in default app and editor, reveal in
   Finder, pinned document cards.
3. **Commands and services** (built 2026-09-06, see status below).
   Saved commands, services with start/stop, autostart, health,
   scrollback on disk.
4. **Environment** (built 2026-09-06 after Spike 5, see status below).
   Global and per-project variables, Keychain secrets, opt-in `.env`,
   masked view, `.env.example` diff.
5. **Git awareness and polish** (built 2026-09-06, see progress below).
   Decorations, sub-repo discovery, global quick-switcher, dock badge
   with waiting-session count.

## Milestone 1 status (2026-09-06)

Built overnight from the design, in five parallel work items on the
skeleton's shared types, then integrated and exercised against real
tmux, Claude Code, Codex, and Ghostty. Verified by hand on this machine:

- project and session records persist; the app was killed and restarted
  repeatedly and every session came back warm, with the reconcile
  finding the live tmux panes;
- a Claude Code session launches inside tmux, Ghostty attaches, hooks
  flow through `switchboard-hook` to the board: working, then idle after
  a one-word prompt, then *waiting on you* (orange, "1 waiting" badge,
  red project dot) when the agent asked a question;
- closing the Ghostty window (detaching) leaves the agent running with
  the same pid; "return" raises the existing window when one exists and
  opens a fresh attached one when it does not; never two processes;
- a shell session shows inside the app through the embedded terminal
  (an `egui_term` view running `tmux attach`), with output flowing in;
- a Codex session's rollout id is discovered after its first prompt and
  stored as the resume handle, including after an app restart.

Known gaps, for the next session:

- Codex has no hooks; since Milestone 2 a quiet pane reads as *idle*,
  but the raw stream fallback (OSC 777 parsing) is not implemented.
- Only Claude Code's `--settings` hooks are wired; the "hooks absent"
  transcript-tail fallback is not implemented.
- The Add-project and New-session dialogs were verified headlessly
  (kittest), not by clicking in the live app; the live app was driven
  through `SWITCHBOARD_SCRIPT`.
- Captions can contain glyphs the UI font lacks (shown as boxes).

## Milestone 2 status (2026-09-06)

Built in one unattended session after Milestone 1 was accepted. The
file side lives in a right-hand panel next to the board and the preview
it opens; the preview is its own view (`View::Document`) so Back and Esc
behave as everywhere else.

- **Tree.** Lazily read per directory through the `ignore` crate, so
  `.gitignore` (plus global excludes) and hidden entries stay out, and
  symlinks are neither followed nor listed. Refresh re-reads everything.
- **Finder.** Typing in the Find field switches the panel to fuzzy
  matches over a whole-project index built on a background thread
  (capped at 50k entries, the panel says when it stopped). Scoring favors
  segment starts, contiguous runs, and file names. Enter previews the
  first hit.
- **Preview.** Markdown through the conversation view's renderer, text
  in monospace, PNG/JPEG/GIF/WebP through egui's image loaders, and
  honest notes for binary, oversized (over 2 MB), or unreadable files.
  Reloads when the file changes on disk.
- **Hand-offs.** Open (default app), Open in editor (the `editor`
  setting: a command such as `code` or `zed`, blank for the system text
  editor), Reveal in Finder, Copy path, Pin/Unpin; all on the preview
  header and on the tree's right-click menu. A directory's menu offers
  "New shell here", which creates a shell session record in that
  directory.
- **Pinned cards** preview on click, and carry Open in app and Unpin.
- **Codex cards** no longer sit at *working* forever: an agent without
  hooks whose pane has printed nothing for 20 s reads as *idle*; output
  flips it back to *working*. Claude Code keeps using its hooks.
- **Settings** (theme, exclusive mode, editor) persist in
  `settings.json`; sessions can be renamed from their header.

Verified headlessly (kittest with a real temp directory: ignored
directories absent, folder open on click, preview shows the file's text,
editor and reveal hand-offs recorded, pin round trip, finder match, pinned
card open) and by hand against this repository as the project (tree,
Markdown preview of this document). Text previews use egui's built-in
highlighter (Rust, C-likes, Python, TOML; plain otherwise). Not yet done
from this milestone's list: git decorations on rows, copy-path feedback,
and drag-to-pin from the tree.

## Milestone 3 status (2026-09-06)

Most of this milestone came with Milestone 1's records: commands and
services are session kinds with a saved command line, a recorded exit
code on the card ("exited (1) since 3h"), Kill, and `pipe-pane`
scrollback on disk. Added now:

- **Restart** on the session header of a shell, command, or service:
  kills the pane if there is one and launches the record fresh; idempotent
  while the relaunch is in flight. Agents get no Restart (it would
  discard the conversation); Return covers them.
- **Autostart** checkbox on a service's header; the reconcile on start
  already launched cold autostart services.
- **Scrollback after the pane is gone.** The raw `pipe-pane` stream is
  read back with escape sequences stripped (CSI, OSC, charset escapes;
  `\r` overwrites a line): the open session shows the last 200 lines
  under "last output kept on disk", and cards get their caption from it.
  Verified by killing both the app and the tmux server and reopening the
  session. Removing a cold record deletes its scrollback file; removing a
  running one leaves the process and its file alone, as before.

Not done: the parser-fed readable history with its own cap and search,
raw-stream rotation in the running app, and a per-session "delete
scrollback" action separate from Remove.

## Milestone 4 status (2026-09-06)

Spike 5 (`spikes/05-keychain`) first: generic-password items created by
the bundle are readable by later builds signed with the same identity and
bundle identifier, without a prompt (the item ACL is identifier plus
certificate, not a build hash). Decisions taken with the user: variables
exist at two levels, global and per project; `.env` loading is opt-in
per project.

- **Layers.** Global variables (in `settings.json`), then the project's
  `.env` files when it opted in, then the project's own variables; later
  layers win. Resolution is a pure function in `core::env`; the app feeds
  it parsed files and a secret lookup.
- **Secrets.** A variable marked secret keeps only its name in the
  record; the value is a Keychain item under service
  `com.sadburger.switchboard`, account `global/NAME` or
  `project/<id>/NAME`. The adapter is tested against a throwaway keychain
  file, never the login keychain. Values reach tmux through `-e` flags,
  never a shell command line, and are never logged (names only).
- **Dialog.** Settings > Environment… edits the global layer; the board's
  Environment button edits the project: name/value rows with a Secret
  toggle (secret values are typed into a password field and stored on
  Save; the field never shows them back), Remove, the `.env` opt-in with
  a file list, and a "New sessions get" preview with sources, masked
  secrets behind a Reveal button, "no value stored" for a secret the
  Keychain lacks, and the names `.env.example` declares that nothing
  defines.
- Verified live: a scratch project with `.env` (`FOO`, `BAZ`), a project
  variable overriding `FOO`, a secret `BAR`; the shell session's
  environment held all three with the right precedence, and the dialog's
  preview and `.env.example` line matched.

Not done: per-session overrides (the `env_profile` field is still
unused), flagging `$VAR` references in saved commands, and a masked
environment section on the session view itself.

## Milestone 5 progress (2026-09-06)

- **Git awareness.** The board header shows each repository the project
  holds (the root, or repositories one or two directories down, so a
  clients folder works) with its branch and changed-path count; tree rows
  carry M/?/! for modified, untracked, conflicted files and a dot on
  directories with changes below them. Read through `git status` every
  5 s on a background thread while the board is on screen; nothing is
  written.
- **Quick-switcher.** Cmd+K (or the Go to button) opens a palette that
  fuzzy-matches projects and sessions you may see (exclusive mode
  applies), with each session's state; Enter opens the best hit, Esc
  closes.
- **Dock badge.** The number of sessions waiting on you sits on the Dock
  icon (AppKit `dockTile`, main thread, updated only when the count
  changes; exclusive mode hides other projects' counts too). Verified
  with an injected `PermissionRequest` hook event: card, "1 waiting", red
  project dot, and the badge all agreed.

- **File side next to a session.** The Files toggle in the session
  header (or Cmd+B) opens the project's tree and finder beside the
  terminal or conversation. The side's bottom half previews the
  selected file in place, on boards too; a click no longer leaves the
  view, and Expand (or Preview in the row menu) opens the full document
  view. The toggle is UI state, not a setting, so it starts closed.

## Open questions

- When the shareable project config arrives, is a hash-and-approve flow
  enough, or should shared commands run in a visibly sandboxed way?
- Notes editor in scope, or "open in editor"? Leaning open in editor.
- Multiple machines: sync the project list early? Leaning no.
