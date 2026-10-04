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
- **Shareable config (built 2026-09-08).** A committed
  `<root>/.switchboard/project.json` carries command and service
  definitions with the variable names they expect (never values). It
  never carries transcripts, secrets, or live-session state. Its contents
  are shown, not run, until the user approves each entry; approval
  records a hash of the entry on the record, and any change or new entry
  requires approval again. Autostart from shared config is never honored
  without that approval. The file is capped at 64 KiB and refused when
  it is a symlink; the README documents its schema.
- **Secrets** come from the Keychain and from `.env` files the user
  already owns; Switchboard never writes them elsewhere. Scrollback can
  contain secrets that a process printed; it is stored privately, capped,
  rotatable, and deletable, and the UI says so. Agent transcripts stay
  where the provider keeps them; Switchboard reads them and never edits
  one. The one write is cloning a session, which puts a *new* transcript
  (a prefix of the original under a fresh id, owner-readable only)
  beside the original; spike 7 has the mechanism.

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
   - a hostile `<root>/.switchboard/project.json` with an autostart
     service is listed and never run until approved; approving, editing
     the file, and the dropped approval are exercised end to end.

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
  `settings.json`; sessions can be renamed from their header. (Exclusive
  mode was later superseded by workspaces.)

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

- **Message box.** Multi-line: Enter (or Cmd+Enter) sends, Shift+Enter
  adds a line, the box grows to eight rows then scrolls. Drafts are
  kept per session, so leaving and coming back does not lose one. Files
  dropped on the window, rows dragged in from the file side, Shift+click
  on a row, the row menu's "Add path to message", and the preview
  pane's "To message" all add a path to the draft, which is the first step toward
  attachments: an agent given a path to an image or a
  document reads it itself. Copying a pasted image to a file in the
  data directory and dropping that path in is the planned next step.
- **File side next to a session.** The Files toggle in the session
  header (or Cmd+B) opens the project's tree and finder beside the
  terminal or conversation. The side's bottom half previews the
  selected file in place, on boards too; a click no longer leaves the
  view, and Expand (or Preview in the row menu) opens the full document
  view. The toggle is saved in `settings.json`, so it comes back the way
  it was left.

## Milestone 6 progress (2026-09-08)

Items are the ids in `docs/feedback-2026-09.md`.

- **A1 Interrupt.** A Stop button beside Send, and Cmd+. anywhere in a
  session view, send Escape to the pane (`AppAction::Interrupt`,
  `Effect::SendKeys`, the existing raw `ProcessHost::write`). Escape
  itself stays Back: it would also blur the message box.
- **C2 Context size.** The conversation meta line shows the tokens the
  last assistant message carried (input plus cache read and write)
  against an approximate window for the model, as `ctx 84k / 200k (42%)`.
  Only the session on screen reads its transcript, so cards do not show
  it yet.
- **B1 Wrapping.** Activity rows wrap instead of being cut at the pane
  width; tool rows keep one line but show the whole line on hover; pane
  snapshots scroll both ways, since a capture is a grid.
- **B2 Terminal size.** The private server runs with `window-size
  latest` (spike 06): a pane follows whichever client attached last, so
  the embedded terminal and a Ghostty window both see the whole pane.
  The option is re-applied on launch because the server outlives the
  app.
- **B3 Raw pane panel.** The Terminal toggle in the conversation header
  (or Cmd+T) opens the pane snapshot as a resizable panel above the
  message box, with its own Hide button, so closing it never means
  scrolling back to the top.
- **C1 Waiting states with a reason.** A session that waits says why
  next to its state, on the card and in the session header: "waiting on
  you: question" for `AskUserQuestion`, "permission for Bash" for a
  tool, "rate limit" (or another provider code) when the turn ended on
  an API error, "input requested" and "quota" for the matching
  notifications. `StopFailure` is its own event now instead of reading
  as idle; notification kinds are matched exactly; and a Claude Code
  pane reads as *starting* until its first hook, so a session that is
  still loading is not mistaken for one that is working. The reason is
  stored with the activity (`activity_reason`, schema v2, older files
  load with it empty). States that fire no hook are still invisible
  (pane scraping is an open question).
- **D3, D4 Definitions and approval.** `<root>/.switchboard/project.json`
  (schema in the README) is read at startup, when a project is added,
  and every 5 s by mtime. Each entry becomes a record with a `source`
  (name, content hash, env names, autostart request) that cannot run
  until approved; `approved_hash` on the record (schema v3) must equal
  the current hash, so an edit drops the approval and reverting it
  restores it. Entries removed from the file are orphaned, not deleted.
  Parsing is strict per entry and tolerant per file, so one typo yields
  one warning naming the entry. The gate test covers list, approve,
  autostart after a restart, and the dropped approval.
- **D1, D2 Run tab.** The side panel next to a board or session has two
  tabs, Files and Run (Cmd+R shows Run, opening the side beside a
  session if needed; the header's Files toggle is now "Side"). The Run
  tab lists the project's services then commands, each with its state,
  Start/Stop or Run now, Show, the command line and directory, the
  variables it asks for (marked when the project's environment does not
  define them), the approval line (Approve, Revoke, "definition changed
  since approval", "no longer in project.json" with Remove), and the
  pane's last output. A pane that exited now reads its output from
  disk like a gone pane, so a finished command's output is there in the
  session view and the Run tab instead of an empty "Not running".
- **D1, D2 Run bar and board rows.** Under the board strip and the
  session header, one button per command (▶ name) and service (• name,
  colored by state): a click runs the command, starts or stops the
  service, or shows a running command; an unapproved entry opens the Run
  tab instead. On the board, agents and shells stay cards under "Agents
  and shells" while commands and services are rows under "Commands and
  services" with Start/Stop/Run now, Show, and Remove. A running
  command shows a spinner and its latest output line next to its
  button; once it finished, the line and the exit state are on the
  button's hover.
- **Message box keeps a failed send.** The draft is cleared only after
  the pane accepted the text; a dead session or a failed write leaves it
  in the box with the error notice, ready to resend.
- **Documents scroll sideways for tables.** Prose wraps at the visible
  width as before, and a Markdown table or a wide image, which cannot
  wrap, scrolls horizontally instead of being cut off at the edge.
- **Markdown in GitHub's colors.** Documents and final responses use
  GitHub's link blue, code block grey with a hairline border, and inline
  code tint, in light and dark, set on the egui style around the viewer.
- **Not done.** Pane scraping for states that fire no hook (C1); the
  question text and choices of an `AskUserQuestion` in the summary view
  (C3); arrow keys reaching a picker in the embedded terminal (A2);
  restricting an approved command to the variables it lists; per-run
  output history; the switchboard view still shows commands and
  services as cards; the context figure on cards.

## Restyle status (2026-09-11)

The app draws with the "Broadsheet" system from the design hand-off
(`App styling help.zip`, screens 1b, 1d, 1e, 1f, and 1a's file panel):
one serif (Source Serif 4, embedded) at a real type scale, a paper
ground with cyan for interaction and magenta for *waiting on you*,
hierarchy from size and whitespace rather than frames. Everything lives
in `src/ui/theme.rs` (tokens for both themes, fonts, text styles, the
status dot, kicker, and button helpers) and the views draw only through
it.

Built:

- Left project rail replaces the top bar: brand, *All sessions* with the
  waiting count, project rows with the project's most urgent state as a
  dot, *+ Add project*, and *Go to* / *Settings* pinned at the bottom.
  Beside a session the rail lists that project's entries with *← Board*
  and *Files ⌘B* / *Terminal ⌘T*. It is resizable and draws dots and
  initials below about 120 px.
- One card widget for every entry kind, laid out in a grid that adds
  columns as the width allows; the agents grid ends in a dashed *+ New
  session* cell, commands and services have their own grid below.
  Not-running cards are outlined instead of filled. The title is the
  click target that opens the session.
- Board header (title, primary *New session*, secondary *Environment*,
  mono root path with the branch), run bar as ghost buttons with the
  service's status dot, all-sessions view with a summary line and
  *Open board →* per project (sessions only; entries stay on the board).
- Session header without a frame (title, dot and state, actions; kind,
  directory, resume handle; run bar), user turns on a cyan tint with a
  *YOU · time* kicker, activity rows unframed, answers on a surface block,
  reading width capped at 860 px, composer with a primary *Send*.
- Files panel: tab row, *Find a path…* input, tree at 13 px, preview as a
  surface block with the actions under the name. Markdown gets cyan
  links and a dark code block on both themes.
- Dialogs, the Go-to palette, the Environment dialog, and the Run tab
  restyled with the same helpers; notices and the host error are toasts
  at the top centre. Dark theme is the token inversion from the hand-off.
- The window opens maximized, on the screen it showed last: the current
  board or session is kept in `settings.json` (`last_view`, written
  whenever the view moves) and pushed back onto the view stack when the
  store loads, if the project or record still exists. Restoring is only
  a view change: nothing is launched or resumed. A document preview
  remembers its board.

Known gaps:

- The rail width is not persisted across launches (egui keeps it for the
  process); persisting it needs a settings field.
- The rail does not snap to the 56 px dot rail; it draws compact when
  dragged below 120 px.
- Card kickers show the state and age; the model line on agent cards
  only appears once the transcript has been read.
- egui has no letter-spacing per style, so kickers set it at the call
  site (`theme::kicker`); there is no keyboard-focus ring beyond egui's.

## Working sets status (2026-09-14)

A working set is the user's own grid of cards from any project:
sessions of every kind and files. There can be any number; each has an
id (`SetId`) and a name. They live in `views.json` in the data
directory (schema version `VIEWS_SCHEMA_VERSION`, now 2: v1 sets had no
id and get one on load; atomic write with `.bak`; a file from a newer
build is kept and never written over). An item is a target (a record
id, or a project id and a relative path) and a rectangle in grid units;
a set owns nothing, and a card whose session or project is gone is
dropped from every set on the next dispatch. The rail has a "Working
sets" section with a row per set and "+ New working set"; a set's
header has Rename (inline), Clone (a copy named "<name> copy", shown at
once), Delete (confirmed), and Arrange. A target sits on any number of
sets: the "Working sets" menu on a board card, the session header, the
file tree's menu (a submenu), and the document header lists every set
with a check where it holds the item, a click toggling it, and "New
working set with this". Adding never launches or resumes anything. The
last view remembers the set (`SavedView::Set`; the older unit variant
still reads as the first set).

Built so far: the model, store, core transitions (`AddToWorkingSet`,
`RemoveFromWorkingSet`, `PlacePin`), the rail row, and the view, which
draws today's cards at the working-set sizes (a unit is 34 px, so a
board card is 7 units; sessions start at 10 by 8, commands and services
at 7 by 5, files at 10 by 10). Columns come from the window width and a
card past the right edge is reached by scrolling.

Arrange mode: the Arrange button in the header (Done to leave it)
disables the cards and shows the unit grid as dots; dragging a card
moves it and dragging its bottom-right handle resizes it, both in whole
units, with the card drawn where it would land and its outline magenta
where that overlaps another card. Release dispatches `PlacePin`, which
the core refuses on an overlap, so a bad drop snaps back. Minimum size
is 3 by 2 units.

A set can instead be chosen by a rule (2026-10-02): see "Rule sets".

Cards: an agent or shell card shows state and project, the name, the
last prompt on one line ("You: …", hover for the whole prompt), then
the last answer above a one-line send box. An agent's final response
is rendered as Markdown and scrolls, so the whole of it can be read on
the set without opening the session, and "View" in the actions row
opens it in the message dialog, rendered or raw for copying
(2026-09-24); a plain answer (an
agent's activity line while it works, or a shell's pane tail refreshed
with the captions while the set is on screen) is cut at the send box,
shows up to a screenful on hover, and opens the raw message dialog on
click. An agent card has "Terminal" at the bottom
right of its actions row: hovering it shows the pane's last 40 lines
in a code block, so the raw output is a glance away without opening the
session. Enter in the send box dispatches `SendInput`;
the box is off while the session is not running. Commands and services
keep the board card. A file card shows the file inside the card,
scrolling: Markdown rendered with a Raw toggle, raw and plain text with
a Wrap/Sideways toggle, plus Open in app and Take off. Previews for
file cards are loaded per path and reloaded when the file changes.

## Clone session status (2026-09-15)

Right-clicking one of the user's own prompts in a Claude Code session's
conversation offers "Clone session". It dispatches `CloneSession { id,
before, prompt }`; the core answers with `Effect::CloneTranscript` and
nothing else, so no record exists until the copy does. The transcript
reader's `clone_before` writes the records before that prompt (the
turn numbering the reader itself uses; a stale number past the end is
an error, never the whole file) under a fresh session UUID, mode 0600,
beside the original, and returns the new handle. `TranscriptCloned`
then adds a cold record beside the source ("<name> clone", same
project, cwd, kind, and launch, next order on the board, resumable
through the new handle), shows it, and queues the prompt as that
record's draft; the shell moves queued drafts into the UI after each
dispatch (`take_primed`). Nothing is launched: opening the clone is a
Return, which resumes it as any cold agent record, and the primed
message is sent when the user says so. Codex sessions and sessions
without a transcript get a notice instead. Dev aid: `clone-session
<session> <turn>`.

Known gaps: Codex rollouts are not cloned (unverified format); a
subagent directory beside the original is not copied (the resume did
not need it in the spike); the clone's card shows no link to its source.

## Terminal on launch (2026-09-15)

Starting or resuming an agent no longer opens its Ghostty window: the
agent runs in its tmux pane and the conversation view follows the
transcript, so the window is opened only by Open (a Return on a running
session). `Settings.open_terminal_on_launch` (default off, set from the
settings menu, `open-terminal on|off` in scripts) restores the old
behaviour of attaching right after the spawn. Shells, commands, and
services are unchanged: they are embedded and never opened a window.

## Prompt Box status (2026-09-15)

Agent sessions' message box is the Prompt Box editor
(`github.com/msull/promptbox`, a git dependency pinned by revision; spike
8 has the mechanism). One `promptbox::Editor` per agent session lives in
`UiState.prompt_boxes`, made on the first draw of the session with its
own sink, save directory (the project root), and the shared key and
settings; drafts and undo history are per session and in memory only. One
`promptbox::Voice` runtime is bound to at most one session: Start
listening on a session (re)binds it, finishing another session's
utterance into that session first; the rail shows "● Listening · name"
with Stop listening while the runtime is live, whatever view is up, and
the click goes to that session. Send hands the prompt to the session's
pane (`SendInput`) through the editor's sink; a session that is not
running refuses it and the prompt stays. The clipboard is never touched
by Send; Copy still copies.

Nothing is shared with the standalone Prompt Box app: settings are
`Settings.prompt_box` and `Settings.voice` (trigger, model, captions) in
Switchboard's `settings.json`; the `OpenAI` key is a Keychain item under
`VOICE_KEY_ACCOUNT`, read once per run and passed into every editor;
history and drafts use Prompt Box's in-memory store and vanish on exit;
tools have no folder. The one file in common is the whisper model, at
Prompt Box's own path, downloaded once by either app.

`Settings.prompt_box` (default on; the settings menu, `prompt-box on|off`
in scripts) falls back to the plain message box. Known gaps: no Dock
badge while recording (Switchboard's badge is the waiting count); the
level meter and status glyphs rely on the fallback fonts; the standalone
app's project vocabulary is not available to embedded editors.

## Notes in the side (2026-09-16)

A session's notes are a tab of the side panel (Files, Run, Notes) rather
than a row under the session header, so the terminal or conversation
gets the full height. The tab exists only beside a session: Cmd+N, the
rail's Notes item, or the tab shows it, and a second press closes the
side as Files and Run do. A board keeps a Notes choice in settings but
draws Files. The field is the whole side; every edit goes to
`SetSessionNotes` and is saved with the record.

## Agent messages along the way (2026-09-16)

An agent often writes text between tool calls before its final answer
(findings, a plan, a status line). Each such message is its own Agent
block in the conversation, in order, with the tool calls that preceded
it folded under a count between the blocks; the first fold carries the
turn's totals. `Activity::text` holds the whole message (the line stays
the excerpt for cards and lists), and a message written in the same
assistant record as a tool call is placed before that call.

## Discard to a prompt (2026-09-16)

"Discard to here" on one of the user's messages does what Clone session
does, in place: the provider-side copy of the conversation up to that
turn becomes what the record resumes through, the message is primed
again, and a running agent is stopped because it sits on the old
conversation. The record keeps the handle it replaced in `discard`
(`Discarded { previous, before, prompt }`), so "Undo discard" in the
session header swaps it back, across restarts, until a message reaches
the pane: `SendInput` with a live pane clears it, and a prompt typed in
the terminal shows as a turn past the cut and hides the button. Neither
transcript file is ever modified; the unused copy stays on disk.

## Shown folders (2026-09-16)

`show` in `.switchboard/project.json` names folders the file side lists
despite the root's `.gitignore`, for a workspace root whose sub-repos are
ignored. The core copies the list onto the project record (`shown`) when
the file is read, so the side has it without re-reading; the file index
walks each shown folder as its own tree with parent ignore files off, so
the folder's own rules still hold, and lists the folder among its
parent's children whatever the rules say.

## Config editor (2026-09-16)

The board's Config button opens `.switchboard/project.json` as text in
a dialog, with the options listed beside it and the parse result (or the
error) shown as it is typed; Save is disabled while the text does not
parse. Save dispatches `SaveProjectConfig`, the core emits
`WriteProjectConfig`, the adapter writes atomically (temp file and
rename, never through a symlink, under the size cap), and the core reads
the file again on success so entries and shown folders follow. This is
the one write into a project directory, and only on the user's click;
approvals are unchanged, so a saved entry still needs approving.

## Plan review workflow (2026-09-17, core built)

A workflow is a durable record that drives other records. The first one
automates the plan review loop the user runs by hand: a planning session
writes a plan; a fresh reviewer (Codex by default) critiques it into a
feedback file; a clone of the planning session accepts each point by
updating the plan or refutes it in a response file; the reviewer reads
both and either raises another round or declares nothing further; this
repeats until the reviewer is satisfied or a round cap is hit. The user
then reviews the final plan, has Switchboard delete the round files, and
hands the plan back to the *original* planning session, whose context
never saw the review, to enter plan mode and implement.

### Records

- `WorkflowRun` lives in the project's directory of the store beside
  its sessions: `definition`, `source` (the planning session), `plan`
  (absolute path), `planner` and `reviewer` (record ids, created by the
  run), `rounds: Vec<Round>`, `state`, `cap`, and timestamps.
  `Round { n, feedback, response, verdict, snapshot }`: the two file
  paths under the project, the reviewer's verdict (`Changes`, `None`),
  and the snapshot directory under the data dir holding copies of the
  plan, feedback, and response as they were at the end of the round.
- `state` is a small machine: `AwaitingFeedback(n)`,
  `AwaitingResponse(n)`, `Converged`, `AtCap`, `Paused(reason)`,
  `Finalized`, `HandedOff`. Every transition emits `Effect::Save`, so a
  restart mid-round rehydrates and keeps waiting.
- `WorkflowDefinition` is a record in the data directory, never in the
  project: the reviewer's first prompt, its per-round prompt, the
  planner's first prompt, its per-round prompt, the handoff prompt, the
  reviewer's agent kind, the round cap, and the file naming pattern.
  Prompts are templates with `{plan}`, `{feedback}`, `{response}`,
  `{round}`, and `{cap}`. A built-in definition ships with the app;
  editing one in the Definitions dialog writes a copy the user owns.
- Settings gain `workflow_round_cap` (default 4); a definition may
  override it.

The three sessions are ordinary `SessionRecord`s so every existing view
works on them. The original planner is never touched. The planner clone
is made once with the clone path (`clone_before` at the end of the
transcript) and continued each round; the reviewer is launched fresh
and continued each round. Cards show a "review" badge linking to the
run.

### Signals

Codex reports no hook events, so "the agent is done" cannot come from
`Activity`. Each prompt names the exact file the agent must write, and
the run advances when that file exists and has not changed for a settle
period (three ticks). "No feedback" is a file too: its first line is a
fixed sentence the definition states, so the verdict is a string
compare, never a judgement of prose. A missing file after the agent's
pane exits, or a resume failure, moves the run to `Paused` with the
reason; nothing retries on its own.

An approval prompt inside the agent (Codex asking to run `gh`, say) is
invisible the same way, and a round can sit on it for as long as nobody
looks. So a waiting run whose agent's pane has printed nothing for two
minutes (`STALL_AFTER`) is marked stalled: one notice names the agent
and the file, the agent reads as *waiting on you* in the rail and the
Dock count, and the round's word on the review page turns to "quiet,
check it". Output again lifts the mark, so a later stall is noticed
again. It is a guess from silence, so a long quiet think trips it too;
the cost is a glance. Commands the reviewer needs without asking (issue
and PR reads through `gh`) belong in Codex's own rules file as
`prefix_rule` allows, not in Switchboard.

Round files live beside the plan, because Codex's sandbox refuses
writes outside the working directory: `<plan stem>.feedback-<n>.md` and
`<plan stem>.response-<n>.md`. The agents write them, not Switchboard.
At the end of each round Switchboard copies the plan and the two files
into `<data dir>/projects/<project>/workflows/<run>/round-<n>/`, which
is what the review page reads, so the trail survives cleanup.

### Trust and money

- A run is launched by the user and thereby authorizes its own agent
  launches, bounded by the cap. This is the one exception to "agents
  are never resumed automatically", and the startup reconcile still
  never resumes anything: a run that was waiting keeps waiting for its
  file, and the next launch happens only when the file arrives.
- At the cap the run stops in `AtCap`; the page offers "Continue" for
  one more round at a time, or "Raise cap".
- Cleanup deletes only the files the run itself named and recorded on
  its rounds, after a confirmation that lists them. Nothing else in a
  project directory is deleted.
- Step 1 never guesses: the launch dialog lists markdown files the
  source session wrote, taken from its transcript, newest first (the
  newest eight, in a scroll region of fixed height), and the user
  confirms one or types a path.

### Handoff

`Finalize` moves to `Finalized` and shows the handoff panel for the
original planner with three choices: back to the planning session as
is, `/compact` first, or a fresh session in the same directory. In
every case the handoff prompt is primed as that session's message box
draft, never sent: the user reads it, edits it and presses send
(only `/compact` goes straight to the pane). Plan mode is requested in
the prompt text (the agent has a tool for it), never with a keypress.
Choosing marks `HandedOff` and shows the session the draft is in.

### UI

- Launch: "Review plan" on an agent session's header and in the
  message menu; a dialog with the plan path list, the definition, the
  reviewer kind, and the cap.
- Review page (`ui/workflow.rs`): rounds down the left with each
  verdict; the plan at that round in the middle with a diff toggle
  against the previous round; feedback and response side by side on the
  right; a notes box at the bottom that sends the user's own feedback
  as one more round. Header buttons follow the state: Pause, Continue,
  Raise cap, Finalize, Clean up, Hand off.
- Board: runs listed under their project with state; working-set cards
  for the run's sessions carry the badge.

### Actions and effects

`StartWorkflow`, `RoundFileProbed` (the probe found or settled a
file), `PauseWorkflow`, `ContinueWorkflow`, `RaiseWorkflowCap`,
`FinalizeWorkflow`, `CleanUpWorkflow`, `HandOffWorkflow`, plus results.
Effects: `ProbeRoundFile`, `SnapshotRound`, `RemoveRoundFiles`, and the
existing clone, launch, send-input, and save effects. `Tick` probes the
awaited files; the probe is the `RoundFiles` port, with a fake.

### Not a step language

Two roles and a loop are what this workflow needs, so the core gets a
concrete state machine, with prompts, cap, and naming in the
definition. A generic step language waits for the second workflow to
show what is actually shared.

### Build order

1. Model, migration, definition record with the built-in default, core
   state machine and tests with faked file signals. **Built.**
2. Round files port, adapter, fake; snapshot and cleanup effects.
   **Built** (`ports::round_files`, `adapters::round_files`).
3. Launch dialog and the review page, headless UI tests, script lines.
   **Built**: "Review plan" in a Claude Code session's header offers
   the Markdown files its transcript shows it wrote (the newest eight,
   in a scroll region of fixed height, so a long session cannot push
   the buttons off screen; an older file is typed); the page has the
   rounds at the left, the plan (with a line diff against the previous
   round's snapshot) in the middle, feedback beside response at the
   right, the note box for the user's own round, and the controls the
   state allows, with cleanup behind a confirmation that lists the
   files; the board lists the project's reviews. Dev aids:
   `review-plan`, `show-review`, `review-file`, `review-continue`,
   `review-finalize`.
4. Handoff panel: **built** as a menu on the page (as is, compact
   first, fresh session). Still to do: one real loop on a throwaway
   plan, and the definitions editor (the built-in prompts can only be
   changed by editing `settings.json` until then).

### As built (steps 1 and 2)

- The first prompt of a launch rides on the agent's command line
  (`first_prompts`, consumed by `launch_prepared`): `claude --resume
  <id> "<prompt>"` for the planner clone and `codex "<prompt>"` for a
  fresh reviewer. No key is ever sent into a pane that is still
  starting. A later round goes in as `SendInput` when the pane is
  running and rides on a resume when it is not. **Unverified:** `codex
  resume <id> "<prompt>"` for a reviewer whose pane died; a spike is
  due before relying on it.
- The definitions live in `settings.json` (`workflows`), with the
  built-in one (`WorkflowDefinition::default`) always available by name
  and never stored; `workflow_round_cap` is the setting. A user round
  (`UserFeedback`) sends the text in the prompt and keeps it on the
  round, so Switchboard still writes nothing into the project.
- `Continue` from `Paused` re-enters the interrupted wait and re-prompts
  only an agent whose pane is gone; from `AtCap` or `Converged` it opens
  one more reviewer round and raises the cap to match.
- The planner clone is `clone_all` on the transcript port: the whole
  conversation under a fresh id, the same private write as a clone.
- The page reads round files itself through the preview cache, live
  for the round in progress and from the snapshot directory once a
  round was copied, so a cleaned-up run still shows every version.

## A narrowed file side (2026-09-24)

A directory's right-click menu offers "Show as top level": the file
side's tree then starts there and the finder searches only under it,
which is what a workspace directory with several repositories inside
needs. The narrowed side says so above the finder, in the accent, with
a Project root button that puts the whole project back. The choice is
per project (the side belongs to the project, and a session's own
window shares it) and lives in settings (`file_roots`), so it survives
a relaunch; removing the project drops it. Paths stay relative to the
project root underneath, so pins, decorations, and the message box's
paths are unchanged. The preview pane's second button is Open, in the
file's app, since that is what is wanted far more often than the
editor, which the row's menu still offers.

## Session windows (2026-09-23)

A session can have a window of its own, so a working set stays up on
one display as the dashboard while the sessions being worked in sit on
another. The window is an egui viewport drawn from the main frame with
the same page code: header, conversation or terminal, message box, run
bar, and the side panel with its own remembered width. The list of
open windows is data (`Settings.popouts`, with each window's frame once
it has held still), so they come back where they were on the next
launch, and one whose session is gone is dropped on load.

The page of a session is drawn in one place. Cards read the pane's
snapshot and attach nothing, so they keep working everywhere; the full
page attaches a tmux client through the embedded terminal, and two of
those on one pane would fight. So while a session has a window, the
main window's page for it is a note and a button that raises the
window, and showing the session from the rail or switcher raises it
too (`Effect::FocusWindow`). Popping out a session that is the main
window's page steps the main window back to what it showed before.

Entry points: Pop out in the session header (Cmd+Shift+P), Open in
window on a card's right-click menu, and the `pop-out` script line.
Cmd+W in the window, its close button, or Close window in its header
puts the page back. Voice binding is untouched: it is per session, as
before. Boards and working sets are not popped out yet; the window
holds a session only, though nothing in the mechanism is session
specific.

## Windows come back where they were (2026-09-24)

Every window's frame is saved once it has held still, in native screen
points with the name of the display it is on (`WindowFrame.monitor`):
pop-outs on their record, the main window in `Settings.main_window`.
A launch opens the main window at its saved frame, and a pop-out at
its own, only while that display is attached; otherwise the main window
is zoomed to the main screen as before and the pop-out opens where the
system puts it. The main window's frame is read from the settings file
before eframe starts, since that is when a window's first position is
decided, and the launch logs what it loaded and chose. Quitting saves
the frames last seen, so a window moved just before Cmd+Q is not lost
to the settle wait. A Dock launch drags the new window onto the Dock's
display and clamps it to that screen after the window exists, so for
the first moment after launch the main window is asked back to its
saved frame instead of being saved where the system put it. A window zoomed to its screen comes back as that frame rather
than as a zoomed state, which macOS would put on the main screen.

## Zoom per display (2026-09-24)

Cmd+= and Cmd+- zoom the window they are pressed in, and the zoom is
remembered for the display that window is on (`Settings.monitor_zoom`,
by the display's name, native when absent), so a window moved to the
other display takes that display's zoom and a new window opens at it.
A window across two displays follows its centre. egui keeps one zoom
factor for the whole context and applies a new one only at the main
window's pass, so the main window sets it from its display each frame
and a pop-out's pass runs with the factor swapped to its own display's
and back (`ui/zoom.rs`); egui's own zoom keys are off. Pointer events
are converted to points as they arrive, between frames, with the
factor installed then, so the frame ends with the factor of the window
under the pointer installed and the main window's is put back, with
its raw input rescaled, just before its next pass. Each change shows
the new percent and display as a notice in the window it was made in.
Cmd+0 stays "show Switchboard", so there is no reset key: step back to
100.

## Open files (2026-09-23)

A macOS app launched from Finder starts with a soft limit of 256 open
files. An embedded terminal costs a pty, a poller, and two threads, and
the review page opened one per frame because terminals were kept by
the view (a session's own) rather than by what was drawn; the vendored
widget's event thread also spun on forever after its terminal was
dropped. Terminals are now kept by the panes drawn last frame, the
thread ends with its channel, a test attaches and drops twenty and
checks the descriptor count is flat, and the launcher raises the soft
limit to what the system allows.

## Watching a review round (2026-09-22)

A round in progress used to be a word ("reviewing") and an empty pane
until the file arrived. Now the pane that round's file will fill (the
Feedback pane while the reviewer works, the Response pane while the
planner answers) shows that agent's live terminal with an Open session
button, and the word under the round in the list ("reviewing ↗",
"answering ↗") opens the session itself, so the work can be watched and
nudged. A run that is still starting says so in the pane.

## Following /clear (2026-09-18)

`/clear` keeps the Claude Code process and starts a fresh conversation
under a new session id and transcript, so a record bound to the old id
went quiet: the view read the old file and a resume would have brought
the old conversation back. An event that carries the pane's record id
(from `SWITCHBOARD_RECORD_ID`, so it is that pane and no other process
in the cwd) and a Claude Code session id other than the record's now
rebinds the record to the new id and transcript, clears any pending
discard undo, and says so in a notice; the shell drops the cached
conversation when a handle changes under an event. The old transcript
stays on disk.

## Removing with an undo (2026-09-25)

Cards move as states change, so a click meant for one can land on
another's Remove. Remove now takes the session off its board at once
but keeps the record whole in the core, untouched on disk, for ten
seconds (`UNDO_WINDOW`), with a toast "Removed <name> · Undo". Undo
puts the record and its working-set cards back; when the window
closes, the record is dropped as before (the pane is left alone, a
cold pane's scrollback is forgotten, its own window and its place in
the Codex queue go). Quitting inside the window is the safe direction:
the record was never removed from disk, so it is back on the next
launch. Session views open scrolled to the end: each window remembers
which conversation it drew last frame, and a conversation shown afresh
starts at its end instead of at the scroll of the one shown before.

## The preview panel, kept where it was put (2026-09-28)

Prompt Box can open its whole-prompt preview by itself while listening
and lets the panel be dragged and resized. Switchboard passes the
choice through as the voice setting "Preview while listening", and
keeps the panel's place: each editor remembers the panel's frame only
for its own run, so when the panel opens Switchboard puts the saved
frame on the viewport itself (if its display is attached), and once
the panel has held still for a moment its frame is saved in native
screen points, as a window's is. So the panel comes back where it was
left, for every session and after a relaunch.

## A hand controller (2026-09-25)

Clicking cards and the microphone all day hurts. A Wii nunchuk on a
Feather RP2040 (`firmware/nunchuk/code.py`) reports its two buttons
and its stick over USB serial, one short line per change, and the
`Controller` port polls those lines every frame from a thread that owns
the port and reconnects when the cable goes (spike 10). The core keeps
a selected card per working set, drawn with an accent border and
defaulting to the top-left card; a press on a card selects it, and a
flick of the stick steps to the nearest card that way, preferring one
that shares rows or columns and never wrapping. The grid scrolls a new
selection into view. Holding Z opens a radial menu on the selected
session's card, a slice per stick direction: up View (the answer in
the message dialog), right Open, down Stop (Escape to the pane), left
Terminal (the live pane in a dialog that stays until closed). The
stick lights a slice; holding it there for half a second (`DWELL`, a
ring grows round the disc) picks it, as does letting Z go while it is
lit, so it does not matter which of stick and Z is let go of first;
letting go on none cancels. The stick that made a pick is spent until it comes back to
centre, so one still held over cannot move the page the pick brought
up. On a session's page Z opens a menu of its
own mid-window: up Pop out, right Back, down Stop, left Terminal (the
raw pane under the conversation, as Cmd+T). On that page the stick
alone scrolls the conversation, in the main window; a flick left or
right steps a cursor to the previous or next of the user's messages
and scrolls it to the top, starting from the end so the first flick
back lands on the latest one, and past the end is the end again. Two presses of Z within 400 ms are Escape instead: a dialog
closes, a card's text field is left, and no menu opens. View and Terminal are things only the UI can show, so the
core queues them as `UiRequest`s that the shell moves into the UI's
state after the dispatch, the way primed drafts travel. Holding C
holds the selected agent session (or the session being shown) open for
dictation: the core records which, and the Prompt Box pump starts and
stops the one voice runtime to match, through the same path as the
microphone button. Two presses of C within 400 ms latch listening
on past the second release, and the next press turns it off as a
single press would; each press that starts listening counts up, so
the UI hears a fresh press even when the same session is already
held. On a file card C holds the card instead and the stick scrolls
it, at a steady speed while the stick is held over, sideways too. A
shell card under C gets a notice. The waiting count goes back down the
wire for the LED strip, and a ping once a second: the device answers
each one, and a device that has been pinged treats the pings stopping
as Switchboard gone, which the USB side cannot tell it (macOS keeps the
line asserted after a program closes the port). The selection is
transient; nothing here touches the store.

The keyboard drives the same selection when no field has focus: h, j,
k, l or the arrows step, i or Enter puts the cursor in the selected
card's "Send a line" field and Esc gives the keys back, v starts and
stops listening into the selected agent (the microphone's toggle), and
o opens the session. So a round trip is: move, i, type, Enter, Esc,
move on.

## Workspaces (2026-09-25)

The top level. A workspace (`Space` in the code, since `Workspace` is
the older name of a project's record) owns projects and working sets,
each of which is in exactly one, and outside Everywhere (the global
workspace, below) the rail shows one workspace at a time: its working
sets, its projects, its "All sessions". The point is a boundary as much
as a grouping: with a screen shared, nothing on screen names anything
from another workspace unless the selector is opened or Everywhere is
active, which lifts the boundary on purpose. Outside Everywhere, then,
the quick switcher searches the active workspace only, with an "All
workspaces" checkbox that is off each time it opens; Cmd+1..9 count the
active workspace's projects; a working set holds cards from its own
workspace only, except a set made in Everywhere, which holds any
(moving a project out of a workspace drops its cards from that
workspace's sets); a notice about a record in another workspace shows
as "Something in another workspace needs you", with no name; and the
rail's count is the active workspace's, while the Dock badge counts
every workspace, so a waiting agent elsewhere still gets through. The
selector is the active workspace's name at the top of the rail: a menu
of every workspace with its waiting count, then New, Rename, and Delete
(only an empty workspace that is not the last). "Move to" on a
project's board and a working set's header moves it, offered only when
another workspace exists.

Records: `Views.spaces` lists the workspaces (views.json, v5 and later), and
`Project.space` (records v8) and `WorkingSet.space` name each thing's
workspace, defaulting to the fixed id of the default workspace, so
files from before workspaces read as members of it with no step. The
active workspace is `Settings.space` and the next launch opens on it;
the last view is restored only if it is in that workspace. Showing
something in another workspace (a pop-out's Show its window, a hit
with "All workspaces" on) steps into that workspace. Session windows
stay open across a switch: they were opened on purpose, and nothing
lists them outside their workspace. Exclusive mode is superseded: its
setting and action stay for older files, but the checkbox and the
filtering are gone.

The global workspace. A workspace is a boundary, and the global
workspace takes it away when the user asks: it is a view that stands
for every workspace together, not a record. Its id is fixed
(`SpaceId::GLOBAL`, `00000000-0000-0000-0000-000000000002`); it is
never listed in `Views.spaces`, holds no projects, and cannot be
renamed, deleted, or moved into, but it can be the active workspace.
While it is, the rail lists every workspace's projects under their
workspace names, "All sessions" covers them all, the quick switcher
searches everything without its checkbox, and a notice names its
record. Working sets made there belong to it and may hold cards from
any workspace; moving a project between workspaces leaves them alone,
and a global set has no "Move to". It lists only its own sets. A
project added while it is active goes to the workspace picked in the
add dialog (`AppAction::AddProjectTo`). The Dock badge already counted
every workspace and is unchanged; it still differs from the rail's
"All sessions" count by Dispatch's decisions, which keep their own row.
Nothing new is stored: `Settings.space` and `WorkingSet.space` may hold
the fixed id, and views.json went to v5 only so an older build, which
would move a global set into a real workspace and prune its cards,
refuses to read or rewrite the file.

The downgrade is not lossless. A build from before Everywhere reads a
v5 views.json as having no workspaces and sends every project to the
default workspace, saving each record (records are still v8, which it
writes). views.json itself survives, so on returning the workspaces and
their sets are back, but projects outside the default workspace must be
moved back by hand. From this build on, a views file newer than the
build moves no project: the build works in Everywhere instead, so a
later bump does not repeat the loss.

## Side panel position (2026-09-22)

The side panel (Files, Run, Notes) sits on the right of the page by
default; the Settings menu's "Side panel on the left" puts it between
the project rail and the page instead, as `Settings.side_left`. Either
way it is dragged to size at its edge facing the page, and the hairline
marking it off is drawn on that edge.

## Commands as runs (2026-09-21)

A command used to be one pane and one scrollback file, so two runs
blurred together and nothing said when a run happened or how it ended.
Now every launch of a command or service opens a `Run` on the record
(number, start time, log path) and the host poll that sees the pane
exit closes it with the exit code and the end time; a pane found gone
closes it without a code. Each run has its own log,
`scrollback/<host>-r<n>.vt`, and the last twenty runs are kept with
their logs. Cards show the last run's verdict as the kicker (`ok · 1.2 s
· 3m ago`, `exit 2 · …`, `running · 12 s`, `never run`) and keep its
output on the card after it finishes; the command's page lists the runs
on the left with the chosen run's log or live pane in the middle.

Outputs are declared, never guessed: a command's `output` in
`project.json` (or the Output files field of the New session dialog) is
a glob or a list of globs relative to the command's directory. When a
run closes, the files matching those patterns that were modified during
the run become the run's artifacts. They appear as chips on the card
and in a Files section under the output on the page, each the full
width of the page with a dragged split between them, so a page of a
PDF reads; a Markdown file renders in place, a PDF's first page is
rasterized by Quick Look (`qlmanage`) into `renders/` on a thread and
shown at the width it is given on the card and the page, and anything
can be opened in its app or revealed. The artifact list is capped at
fifty per run and the finder is a port so the core never touches the
disk.

One vocabulary everywhere a command appears: its name opens its page
(or the Run tab when it is not yet approved), `▶ Run` or `▶ Start` runs
it, `Stop` stops it. Board card, working-set card, run bar, Run tab,
and the page header all use these three.

Known gaps: only a PDF's first page is shown; a service's run closes
only when its pane exits, so a long-lived service is one run until it
is stopped.

## The control port (2026-09-29)

Another process (Dispatch, `docs/dispatch.md`) drives Switchboard over
`<data dir>/control.sock`: newline-delimited JSON, one request in, one
reply out, the contract in the `switchboard-control` crate rather than
`AppAction`. Every command carries an operation id. The core runs it as
`AppAction::Control` with a quiet flag set: no view changes, no
terminal window, and every record it makes (project, session, space,
working set, run, and the run's reviewer and planner clone) carries the
id in a new `op` field (records v9, views v4). A creation appends a
request line to `operations.log` before its save, and the reply is
appended after, so `find` answers from the log first and a record
removed in the window still reads as made, a repeated id is answered
from its reply line without running again (which is also what keeps a
repeated `session.send` from sending twice), and `op.status` reads a
request line with no reply, or a record still marked `pending_launch`
(set before a control-owned launch, saved, cleared when the launch
reports), as interrupted. A `Stop` hook event now records
`last_stop_at`, and `session.waiting` sets an outside reason on a
record that makes its card read as waiting on you while the pane runs,
so the badge and the rail count Dispatch's pending decisions without
knowing about them. Only the instance holding the store lock listens.
An agent record whose `launch` is `Argv` adds those flags to the
composed command line, ahead of the first prompt (with `--` between,
since Claude Code's multi-value flags would otherwise read the prompt
as one more value), which is how a pipeline operator names a model and
how Dispatch grants Claude Code its artifact directory with an
`Edit(//<dir>/**)` allow rule.

The port runs the other way too. Dispatch serves its own
(`dispatch-control`, `<Dispatch data dir>/dispatch.sock`, while
`dispatch run` is up), and the app is a client of it: a status poll
every two seconds enters the core as `DispatchStatus` (tickets,
decisions and attempts as views, never Dispatch's records), the Dispatch
page lists what waits on the user with each option as a button and one
`DispatchDecide` is one call whose reply is one more action, a ticket
page reads a stage's artifacts through the port, a session Dispatch
made links to its ticket, the rail's Dispatch row and the Dock badge
count pending decisions, and the console is a shell session of the
app's own in a `Dispatch` space and project rooted at Dispatch's data
directory, remembered in the settings, into which a typed line is sent
as the `dispatch` executable beside the app's plus the words (a `!`
line goes to the shell as it is). Without a runner the page shows the
last status it had and the console still works, which is how a runner
is started. The status carries each project's policy limits and where
it stands against them (slots in use, decisions waiting), so a ticket
that is not moving says why it is held and the project's section says
what it allows; the page narrows to one project with a chip row, and a
decision card names its project. The page can leave for a window of
its own (`Settings.dispatch_window`, like a session's pop-out: frame
saved once it settles, Cmd+W closes), with navigation of its own so a
ticket opened there does not touch the main window's stack, which
shows only a note pointing at the window while it is open. A ticket's
page offers Close where the runner says a close would start, and
Remove trees where a closed ticket's trees were kept. Built: the wire
crate, the core path and read models, the log and socket adapters,
`serve` in the app, `tests/control.rs`, and the close: `DispatchClose`
behind a confirmation dialog that lists the paths the runner reports it
would remove (`TicketView::removes`), answered by Dispatch with the
ticket `closing` once the intent is saved, the runner's pass doing the
rest. Gaps: `service.new` and `command.run` are not
on the port yet; nothing marks Dispatch-owned records as such in the
window.

Claude Code asks whether to trust a folder before it runs any hook, so
a launch into a fresh worktree sits at that question with nothing to
report. The shell reads every Claude Code pane that has reported no
hook yet (with the captions, every two seconds) for the question's
text and tells the core with `PromptSeen`; the card reads as waiting on
you with the reason, the count includes it even when Dispatch has
marked the session, and the first hook event clears it. Transient, so
a restart reads the pane again.

The Dispatch page shows a ticket's own agents waiting for themselves
(`waiting_agents_of`: an open attempt whose session counts as waiting,
with the trust question or the hook's reason): a card under "Waiting
on you" with the attempt and an Open session button, the ticket row's
standing, a line on the attempt on the ticket page, and the rail count
beside the decisions. The page can then be the one view watched. The
question is answered from the card or the session page ("Trust this
folder", `TrustFolder`): down arrow and Enter to the pane, sent only
while the pane was last seen showing it. The port reports the question
on the session view (`trust_question`) and answers it with
`session.trust`, so a pipeline whose policy says `trust_folders = true`
has Dispatch answer it for that project's agents.

The `dispatch` crate holds the first slice of `docs/dispatch.md`: the
Switchboard pipeline through `investigate`, the automatic `lanes`
decision, `plan`, and a review run on a copy of the plan, stopping at
the `finalize` decision. A ticket's tree comes first: Dispatch keeps
its own clone of the project's `repo` under `<data>/repos/<project>`,
fetches it before every cut, and cuts the ticket's worktree from
`<remote>/<base>` under `~/.dispatch/worktrees/<ticket>` (the
`dispatch worktrees` setting in `<data>/settings.json`, or the
pipeline's own `worktrees`; a root with a space or a shell-special
character is refused at take, since the repository's tooling gets the
path; `--migrate`, or the page's "Set and migrate", moves idle tickets'
trees with git, repairs the lane clones and re-points the Switchboard
projects through the new `project.root` command), so the user's
checkout is never read or touched and a
ticket starts from what the remote has now. Every stage runs in that
tree, under one Switchboard project per ticket named `#<n> <title>`
and rooted there. A lane with a `repo` of its own (Orchard's workspace
holds three) is Dispatch's clone of that repository too, cut as a
worktree at the lane's path inside the ticket's tree; every lane is
cut before the first stage, the `lanes` decision only chooses which
lanes the stages run in (label hints answer it when the dial says
auto), and a lane's setup runs once before its first agent. A review
stage's reviewer may be Claude Code or Codex; the operator's flags go
over the port with `workflow.start`. What each kind needs is answered
by `OperatorKind` alone (a Claude Code reviewer runs in the ticket's
tree with an allow rule for the attempt directory, Codex in the attempt
directory), so a new kind of agent is one variant and its answers. A project with `root`
instead of `repo` works in place with no branch, for one that is not a
repository. An agent stage's command gate (`implement`) runs after
the agent stops: the context's tree must be clean, its head is
recorded, the fixed argv starts as a child of the runner with the
ticket's values in its environment and its output in the attempt's
`checks.log`, and the exit is bound to that head only if the tree is
still clean at it after; a dirty tree, a nonzero exit or a changed
head is a failed attempt and a rerun decision, and a runner restart
starts the lost check again on the same head. (The design has the
gate run as a Switchboard command record; a child of the runner is
what is built, so the port needs no per-record environment yet.)
A gate-only `pr-checks` stage (`ready`) opens one attempt per
context that launches nothing: it finds the lane's PR by branch
through `gh` (the provider is read from the remote; another host
parks the ticket until an adapter exists), reads its checks once a
minute, and completes bound to the tree's head when they are green at
it, or at once on a merged PR or, with `checks = "none"` on the
stage, on an open PR at the head; no PR, another head, red checks or
missing checks past the young-head window is one `pr` decision with
`recheck` and `park`, never a failed attempt, and a provider that
cannot be read is retried quietly for an hour first. The PR is
recorded on the attempt and shown on the ticket page as a link with
the last reading. A `pr-merged` stage (`merge`) makes the
confirmation decision the design describes (only `park` can be
answered by hand; the session is marked waiting), reads the same PR
once a minute, and on merged completes the attempt and answers the
decision as Dispatch. A human gate-only
stage (`inspect`) asks once per lane with the branch, its head, what
it adds over its base, the tree and the notes (Bitbucket Cloud is a
second provider for both PR gates, read through `curl` with the
account token from the environment or `<data>/env`); `proceed` passes,
`rerun` with a note (typed under the decision on the page) sends
that lane back to the nearest earlier agent stage with the note in
the next prompt. A gate-only stage and a ticket closing past its last
stage hold no slot. A PR GitHub reports as conflicting is rebased by
the policy's `rebaser`, a session the new `session.clone` port command
makes from the lane's implementer's transcript (the record is made
first, so the reply carries its id and recovery finds it by its op;
the copy and the resumed launch follow), capped by `max_rebases` and
never repeated at the same head; a rebaser's or fixer's completion
logs with its role and the head it left. Everything else in that document (external and
human gates on agent stages, gate-only command stages, budgets, the
other pipelines) parks the ticket with a reason. The runner's pass over a project is
one transaction under the data directory's writer lock, and `decide`,
`take` and `queue` take the same lock around their read and write, so
an answer from the terminal is never overwritten by a pass that read
the record before it; a second `dispatch run` on the same directory is
refused by `runner.lock`. Every request keeps its body in the ledger,
so a lost reply to an idempotent one (`workflow.finalize`,
`session.kill`, `set.sync`) is sent again as the same operation; a
lost reply to a non-replayable one (`session.send`,
`workflow.continue`) is a `lost-send` question asked once, since its
ledger entry is marked asked and recovery skips it from then on. An
attempt with no launch on the books fails rather than waits. Parking
and a rerun follow the design's cancellation sequence: the intent is
written (for parking, with every open decision on the ticket
cancelled in the same write: a pending one, an answer not yet acted
on, an acted `rerun` whose replacement has not launched, and a human
gate's send-back note not yet carried by an attempt, so no answer
given before the park launches on the resume or keeps its context
from being asked again, while a `rerun` answer to the resume's
question puts the quoted note back; the session's waiting mark is then cleared
and read back), the review run paused and read back as paused, every
process killed and read back as gone, and only then does the ticket
read as parked or the rerun's answer count as acted (which is what
lets the replacement start); the sequence runs from the saved intent
on every pass, so a restart at any point resumes it whole. A resumed
ticket asks `rerun` again, under a new id and without waiting for a
slot, about each failed or cancelled attempt with no rerun question
in flight, with the attempt's own reason (for a sent-back attempt,
the note) and the options its failure first had; a pending decision
holds its stage only in the context it is about. With every slot
taken, a `park` answer is still acted on, since parking launches
nothing; any other answer waits for a slot. An answer's acted mark
reaches disk only with its action's first write (the parking state,
the ledger entry, the lane record), so a stop leaves an answer either
still unacted or with a durable intent. A record's backup is a hard
link, so its primary is never absent mid-write. The reviewer's
templates get `{worktree}`, `{branch}` and `{project.root}` rendered
in, and a template naming neither is prefixed with the repository's
path, since the reviewer works in the attempt directory.
The runner is patient. A Stop with the card still `working` holds an
agent, reviewer or fix attempt; it fails only after about thirty
seconds idle at its prompt without its artifact, or when the pane
exits. Only `no such run` fails a review run's query or reads a paused
run as stopped; any other failed reply or socket error is asked again
next pass with one warning. A `none` check reading within two minutes
of the head's last move is pending. Answering clears every session a
decision marked, not only the ticket's newest.
Review rounds carry forward. A new attempt of a code review stage
carries the last attempt's settled and open points (`carried_from`,
ids qualified `a<n>/...`) and reads only the change since, unless its
note says "start over"; a send-back note moves onto the attempt when it
starts (dispatch `RECORD_VERSION` 4). A `style:` point, or an untagged
one from the `style` reviewer, holds no round open from `style_rounds`
(default 2, live) and is left to the merge; a `decided:` point contests
the plan's "Decisions" section, which the reviewers are given as
settled, and is found but not done; a reviewer's line that every point
left is wording is a note, not a point. The first review after a
rebase with commits checks it, with the rebaser's notes. A completed
review attempt writes `summary.md`. A code review stage with `commits =
"fold"` or `"one"` rewrites the branch as it completes, with the tree
proven unchanged and the rewrite on the attempt (dispatch
`RECORD_VERSION` 5); a branch the remote already holds is left alone.
A prompt's `{lanes}` is the lanes the ticket chose, in pipeline order
(every lane before the `lanes` decision), and `{lanes.all}` every lane.
A retake after close deletes each kept branch with nothing beyond its
base and asks `reuse | fresh | park` about one with commits, holding
the ticket at the cut; `close` lists the branches it keeps.
`dispatch/tests/first_slice.rs` is the acceptance table, plus a test
per point above; `dispatch/tests/live.rs` runs the first stage against
a real Switchboard and a haiku agent. Known gap: Claude Code treats a
git repository as its own workspace for the folder-trust dialog, so the
first agent in each lane's worktree blocks on that dialog until it is
answered in the pane once (a plain directory under a trusted parent
inherits the trust; a fresh repository does not). Nothing in
Switchboard or Dispatch writes Claude's trust file. One more: `close`
(and `retake`, which closes by it) leaves pending decisions pending.
And: a `working` card left stale by a subagent's last hook, with the
main agent never taking another turn, holds the attempt until it is
parked; the Stop hook's `background_tasks`
(`spikes/03-session-state/README.md:94`) would say it exactly but is
not on the wire. And: the ticket page does not show that an attempt was
carried or how many points a round left to the merge (each would need a
field on the `dispatch-control` views), and a command reviewer cannot
tag a point as style.

## Global workspace status (2026-10-01)

Built: `SpaceId::GLOBAL` and `SpaceId::contains` in the core, the rehydration
that keeps a global active workspace and global sets (and sends a project
hand-edited into global to the first workspace), the refusals (rename,
delete, move into, move a global set out, a project added into it from
the window or the control port), "◇ Everywhere" first in the selector
when more than one workspace or any global set exists, while it is
active, or while a project's workspace is unlisted (views.json from a
newer build, where the core works in Everywhere for the run without
saving that choice), the rail grouped by workspace with unlisted ones
under "Other workspaces" (Cmd+1..9 count it as drawn), the add dialog's
workspace choice, notices named, the switcher without its checkbox, the
`workspace-global` script line, and `set.new` accepting the fixed id on
the control port, whose `spaces` reply lists it last with `view: true`
so a client walking the spaces finds every set it can make. views.json
is v5.

Known gap: a real workspace's sets are not listed in global, so a set
of one workspace is reached by switching to it.

## Rule sets (2026-10-02)

A rule set is a working set whose cards a rule chooses instead of the
user. One rule exists: every session of the set's workspace active in
the last N hours (default 24, 1 to 720), newest first, laid out by the
core left to right at one card size (`grid::flow`, 10 by 8). The rule
is the record (`WorkingSet.rule`, `SetRule::Recent { hours }`, on disk
`{"kind": "recent", "hours": 24}`); membership is a cache
(`AppCore::rule_members`) worked out again after every action, so nothing
is written per card and a tick never saves. Everything that draws or
walks a set reads `AppCore::set_cards`: a hand set's pins, or a rule
set's members laid out. views.json is v6, so an older build leaves a
file with rules alone; a v5 file reads as hand sets.

Activity (`AppCore::last_active`) is the latest of `last_seen` (the
launch, and every hook event), `last_event_at`, `last_stop_at`, and the
pane's last output, alive or exited, while tmux still holds the pane.
A live pane counts through what it prints, not by being alive, or an
idle shell would never leave. When the pane goes (a kill, a restart, a
stop, or the server dying), the core first raises `last_seen` to that
output time, so the session keeps its place until the window passes.
Every kind of session is included, Codex through its pane output.

Dismissing (the "×" on a card, "Dismiss" on a stopped card, "Kill and
dismiss" in a running card's menu) stores the session's activity time
on the set (`WorkingSet.dismissed`), so it stays off while it is quiet
and comes back by itself the moment it does something. A `SessionEnded`
event moves a dismissal in force up to the end, so the end a kill sends
does not bring the card back; Kill and dismiss is one core action
(`KillAndDismiss`) that stamps the dismissal and kills the pane.
Dismissals are pruned when their record is gone, ride along in the undo
window of a removal, and are not pruned by age, so widening the hours
does not bring back what was dismissed.

A rule set takes no pins by any path (`AddToWorkingSet`, `PlacePin`,
`NewWorkingSet { with }`, the control port's `SyncSet`, which answers
"the set is chosen by a rule"), is left out of the "Working sets" menu,
and has no Arrange. Its header shows "Sessions active in the last [24]
h" with the hours as a field. A set's workspace decides which sessions
it may hold through `target_in`, so a rule set made while Everywhere is
active covers every workspace with no code of its own. Clone and Move
to carry the rule and the dismissals. The control port's `set` replies
carry `rule`, with `items` the members laid out at 24 columns. Script
lines: `new-recent-set`, `set-hours`, `dismiss-from-set`; the last two
take a multi-word set name (every word but the last). Hours from any
sender are clamped to 1 to 720. The rail lists rule sets before hand
sets, each in record order (`AppCore::working_sets_in_rail_order`); the
control port's `set` replies keep record order.

Known edges: a pane that redraws forever (`top`, a clock in a prompt)
cannot stay dismissed past its next redraw; Kill and dismiss is the way
out. Rule sets are rule-only (no hand pins beside the rule's cards).
The controller steps through a rule set's list in order (Left and Up
back, Right and Down forward) rather than across the grid, since the
core does not know the view's columns.

## Cleanup pass (2026-10-03)

One pass over the workspace with no change in records or wire
formats, and one change in behaviour: returning to an exited pane now
kills it through the same path as every other kill, so it first raises
`last_seen` to the pane's last output (and saves the record) as a stop
or restart already did. Otherwise: duplicated code merged into helpers (the core's running,
waiting-reason and host-error checks, the store's one atomic write, the
binary lookup, Dispatch's attempt lookups and `git -C` calls), comments
moved to the items they describe or brought up to date, fake fields
nothing read removed (`fakes::services()` builds the default set),
narrower visibility, the unused `fd-lock` dependency dropped, and seven
fewer tests, each removed or merged into another that already covers
it.

Known gap: `HookLog::compact` and `rotate_scrollback` are called only
from tests, so `events.log` grows without bound and the compaction the
hook section above describes does not happen yet.

## Open questions

- Shared project config runs with a hash-and-approve flow and no
  sandbox. Should approved commands be restricted to the variables they
  list instead of receiving the whole project environment?
- Notes editor in scope, or "open in editor"? Leaning open in editor.
- Multiple machines: sync the project list early? Leaning no.
