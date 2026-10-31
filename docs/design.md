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
- Named environment sets (built 2026-10-06; see "Environment sets"
  below) hold credentials that never enter a pane: a session, its
  project or the Dispatch runner is granted sets, and
  `switchboard-env exec -- <command>` puts their variables in that one
  child's environment, resolved over `env.resolve`.

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
  already owns; Switchboard writes them nowhere else. They leave the app
  only as `new-session -e` written to the tmux client's stdin
  (`start-server ; source-file -`, spike 18), or as the `env.resolve`
  reply on the owner-only `control.sock` to `switchboard-env`, which
  puts them in its child's environment and nowhere else. Neither is a
  command line, so no value reaches the client's argv or the server's,
  which keeps the argv of the client that started it. `env.resolve`
  answers only the holder of a record's launch token
  (`SWITCHBOARD_RECORD_TOKEN`, new at every spawn, its SHA-256 on the
  record), so a session resolves only its own grants; the token is as
  private as the pane's environment. The same token gates
  `session.ask`, which marks only its holder's own record as asking the
  owner something. The one other way a secret's surroundings leave the
  app is as redacted text on the remote port: a paired device reads a
  pane's screen with every value Switchboard holds for the project
  replaced by its name, and Dispatch's ticket text (the list, a
  ticket's summary and body, its events, and every text file of an
  attempt, the checks log included) with every value of the runner's,
  each attempt session's and every project's grants replaced the same
  way, since no ticket names the project it runs in; that over-masks
  rather than leaks. The match is literal: a value in an encoded form,
  or one a `set -x` trace writes differently (a value holding `'`, `\`
  or a newline), can pass, and a value the app never held is not
  found. The port's key and tokens never reach a record or a log.
  The writes (`env.set.upsert`, `env.secret.store`, `env.grant`) are
  accepted only inside a five-minute setup window the owner opens from
  the app's settings menu, so an agent cannot grant itself a set. What a
  child prints reaches the agent's tool result and the provider's
  transcript: the owner accepts that by granting a set. Scrollback can
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
   `egui_term` (vendored from git; builds on egui 0.37 unchanged; renders
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
  `project/<id>/NAME`, or `set/<name>/NAME` for an environment set. The
  adapter is tested against a throwaway keychain file, never the login
  keychain. Values reach tmux as `new-session -e` on the tmux client's
  stdin, never on any command line (spike 18), and are never logged
  (names only). A set's values reach only
  the `env.resolve` reply to `switchboard-env` and its child's
  environment.
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
  (or Cmd+T) opens the pane snapshot as a resizable panel under the
  message box, with no header row; Hide floats over its top-right
  corner, so closing it never means scrolling back to the top.
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
- The window opens filling the primary display's visible frame, by an
  explicit size rather than the system's zoomed state (which the app
  cannot bound to the GPU's surface limit), on the screen it showed
  last. The current board or session is kept in `settings.json`
  (`last_view`, written whenever the view moves) and pushed back onto
  the view stack when the store loads, if the project or record still
  exists. Restoring is only a view change: nothing is launched or
  resumed. A document preview remembers its board.

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
still reads as the first set). A session made over the control port
with `replaces` takes over the old session's pins on every hand-placed
set of its space, in place.

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
session. Enter in the send box dispatches `SendMessage`, which
clears the line only once the pane has it; the box is off while the
session is not running. Commands and services keep the board card. A file card shows the file inside the card,
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
pane (`SendMessage`) through the editor's sink; a session that is not
running refuses it and the prompt stays. A write that fails gives the
prompt back to the editor, which still shows "Prompt sent" and lists
the prompt in its recent list. The clipboard is never touched
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
app's project vocabulary is not available to embedded editors; a
failed write still reads as "Prompt sent" in the editor and its recent
list, because promptbox settles a delivery when the text is queued, not
when it is written.

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
the pane: a message from the owner's boxes or any other writer
(`SendMessage` or `SendInput`) with a live pane clears it, and a prompt
typed in the terminal shows as a turn past the cut and hides the
button. Neither
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
  `Failed(reason)`, `Finalized`, `HandedOff`. Every transition emits
  `Effect::Save`, so a restart mid-round rehydrates and keeps waiting.
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
pane exits, or after a Claude Code agent stopped since it was asked
(each round keeps `feedback_asked` and `response_asked`) and its card
has read idle for `STOP_GRACE` (thirty seconds, Dispatch's
`STOP_IDLE_POLLS` wait), moves the run to `Failed` with the reason; a
resume failure moves it to `Paused`. Nothing retries on its own. Codex
has no Stop hook, so a Codex reviewer that sits idle without its file
still only gets the stall notice below; the planner is always a Claude
Code clone, so its half is always covered. A round saved before the
asked times were kept never fails on a stop (the stop may be the
previous round's). A Claude Code agent whose hooks have reported since its
pane was spawned (see Known gaps), that has
started no turn since it was asked (no typed `UserPromptSubmit` after
the ask: `SessionRecord::last_prompt_at`; an injected notification does
not show the ask was taken), whose pane runs, whose card is not on an
open turn or at a permission dialog, and that holds nothing for the
owner (the trust question, an outside wait) fails the run the same way
once `LOST_PROMPT_AFTER` (three minutes) has passed since the ask: a
dialog in the pane swallowed the prompt, and no Stop will ever come.
Codex has no hooks, so a Codex reviewer whose prompt is lost still only
gets the stall notice.

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
  right. The note box sends the user's own feedback as one more round.
  It and its "Send my feedback" button are one block pinned under the
  header, above the columns, so a short window never pushes the button
  off the page; the box scrolls rather than grows, and Cmd+Enter in it
  sends (a blank box sends nothing). Header buttons follow the state:
  Pause, Continue, Raise cap, Finalize, Clean up, Hand off.
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
- `Continue` from `Paused` or `Failed` re-enters the interrupted wait
  and re-prompts an agent whose pane is gone or that stopped since it
  was asked (for a round saved without an asked time, any recorded stop
  with the card idle), setting the asked time anew; a running agent
  that has not stopped is left to finish. A round with no asked time
  never fails on a stop, so Pause it first to free it; Continue does
  nothing while a run waits. From `AtCap` or `Converged` it opens one
  more reviewer round and raises the cap to match.
- The owner's objection arrives over the control port as
  `workflow.object {run, round, text}` (`ObjectWorkflow`), sent by
  Dispatch's `revise` answer to `finalize`. It is refused unless the run
  is `Converged` or `AtCap`, has a planner, the text is not blank, and
  `round` is the run's round count plus one, so the feedback file the
  sender wrote beside the plan and the round opened here carry the same
  number. The round keeps the text in `user_feedback` with `objection`
  set (schema v13), and the planner gets the `respond_to_user` prompt,
  as for a notes-box round. Once its response settles the reviewer is
  prompted for the next round whatever the cap, which is raised to
  match; a notes-box round still ends the review at `Converged`. The
  rounds list reads it as "owner's objection".
- The ticket page's Plan tab titles a round Dispatch lists with
  `PlanRoundView.by` as "owner", or "owner, via the supervisor" when
  `by` is `supervisor`, so a reader tells the owner's round from the
  reviewer's and sees when the supervisor typed it on the owner's
  behalf.
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

Every restore (the main window at launch, a pop-out, the Dispatch
window, the preview panel, and the launch hold) fits the saved frame
to its display's visible frame and to 8192 device pixels a side
(`core::window_fit::fit`) after the attached check, and the launch log
records the fitted frame next to the saved one. With nothing saved, the
main window opens at the primary display's visible frame.

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
raw pane under the message box, as Cmd+T). On that page the stick
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
knowing about them. `session.ask`, sent by `switchboard-ask` with the
pane's launch token, sets the session's own question beside it in a
field Dispatch's `session.waiting` never touches. A run's `paused`
state carries `failed: true` when its agent let the round down (the
core's `Failed`) rather than a user's Pause; the flag is omitted when
false and read with a default, so either side may be older. Dispatch
fails the review attempt on it. Only the instance holding the store
lock listens.
An agent record whose `launch` is `Argv` adds those flags to the
composed command line, ahead of the first prompt (with `--` between,
since Claude Code's multi-value flags would otherwise read the prompt
as one more value), which is how a pipeline operator names a model and
how Dispatch grants Claude Code its artifact directory with an
`Edit(//<dir>/**)` allow rule.
`session.new` takes an optional `replaces`, a record id: the new
session takes that record's place on every hand-placed working set
whose space it belongs to, at the same rect (`AppCore::take_over_pins`;
a set that already pins the new session just drops the old card). It
is not written when absent; an id that does not parse refuses the
request, and one that matches no pin moves nothing.

The port runs the other way too. Dispatch serves its own
(`dispatch-control`, `<Dispatch data dir>/dispatch.sock`, while
`dispatch run` is up), and the app is a client of it: a status poll
every two seconds enters the core as `DispatchStatus` (tickets,
decisions and attempts as views, never Dispatch's records), the Dispatch
page lists what waits on the user with each option as a button and one
`DispatchDecide` is one call whose reply is one more action, a ticket
page reads a stage's artifacts, the ticket's events and the ticket in
full through the port (see "Dispatch ticket page"), a session Dispatch
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
Remove trees where a closed ticket's trees were kept. Resume, on the
board's row and the ticket's page, is `dispatch resume`: it reruns what
the park cancelled mid-run, which spends agent runs on one click, and
says so on its hover text; `--no-rerun` is only on the command line.
Built: the wire crate, the core path and read models, the log and socket
adapters, `serve` in the app, `tests/control.rs`, and the close:
`DispatchClose` behind a confirmation dialog that lists the paths the
runner reports it would remove (`TicketView::removes`), answered by
Dispatch with the ticket `closing` once the intent is saved, the
runner's pass doing the rest. Dispatch's services are `session.new`
service records with an argv launch that carries `PORT` through `env`,
killed and then removed with `session.remove`, and a service's `before`
runs as a child of the runner. Gaps: `service.new` and `command.run`
stay unbuilt; nothing marks Dispatch-owned records as such in the
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
starts the lost check again on the same head, except a gate-only
command, which is asked about and waited for, never started again or
signalled. (The design has the gate run as a Switchboard command
record; a child of the runner is what is built, so the port needs no
per-record environment yet.)
A gate-only `pr-checks` stage (`ready`) opens one attempt per
context that launches nothing: it finds the lane's PR by branch
through `gh` (the provider is read from the remote; another host
parks the ticket until an adapter exists), reads its checks once a
minute, and completes bound to the tree's head when they are green at
it, or at once on a merged PR or, with `checks = "none"` on the
stage, on an open PR at the head; no PR, another head (unless Dispatch
pushed the tree's head under two minutes ago), red checks or
missing checks past the young-head window is one `pr` decision with
`recheck` and `park`, never a failed attempt, and a provider that
cannot be read is retried quietly for an hour first. The PR is
recorded on the attempt and shown on the ticket page as a link with
the last reading. A `pr-merged` stage (`merge`) makes the
confirmation decision the design describes (`recheck` and `park` can
be answered by hand; the session is marked waiting), reads the same PR
once a minute, and on merged completes the attempt and answers the
decision as Dispatch. A base that moved into a conflict under it (a
local `git merge-tree` probe, or the provider's `conflicting`, which
Bitbucket's `diffstat` gives) sends the ticket back through the
`pr-checks` stage before it to be rebased, pushed and checked again,
as does `recheck`; a tree that is not clean, a stage between that is
not gate-only, or a trip back that left the head where it is asks
instead. A lane may name lanes whose pull requests merge first
(`merge_after`, with `merge_after_deploy` for the base pipeline on that
merge's commit, or one step of it): its merge question is held, and
the hold is shown on the ticket page and logged as a `waits` event,
until they merged and their run finished, then asked plainly, or with a
"but" naming a failed, late or unread run, a missing merge commit, or a
closed pull request. The base pipeline wait is refused for Bitbucket
until spike 16's reads are run there. The order is static per pipeline
rather than per ticket, and the supervisor's seed is told to merge in
it; one that merges outside `gh` merges by hand in the same order. A
human gate-only stage (`inspect`) asks once per lane with the branch,
its head, what it adds over its base, the tree and the notes (Bitbucket
Cloud is a second provider for both PR gates, read through `curl` with
the account token from the environment or `<data>/env`); `proceed`
passes, `rerun` with a note (typed under the decision on the page) sends
that lane back to the nearest earlier agent stage with the note and
the previous attempt's notes path in the next prompt, restarting that
stage's services with a fresh `before`. The question shows each
notes file's first line. A gate's own `question` follows these lines
after a blank line rather than replacing them, and a branch or diff
line whose git read fails is left out (and logged) rather than failing
the gate. A `confirm` gate (`tried`) inside its agent
stage's hold run offers `rerun` beside `done` and `park`, so a try
that could not test runs again on the same deploy while the stack
stays held. The note box under a decision is multi-line, and an
option the decision lists in `DecisionView.needs_note` (`revise` on
`finalize`) stays disabled until the note has text: Dispatch refuses
that answer without one and the decision stays pending, so the page
gates it rather than send a click that changes nothing. That is on the
Dispatch page; the ticket page takes a plan review's note in the Plan
tab's block. A gate-only stage and a ticket closing past its last
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
`session.kill`) is sent again as the same operation; a
lost reply to a non-replayable one (`session.send`,
`workflow.continue`, `workflow.object`, and an objection the port
answered "the app did not answer in time") is a `lost-send` question asked once, since its
ledger entry is marked asked and recovery skips it from then on. A
ticket's ledger holds only that ticket's requests: the queue set's
`set.new` and `set.sync` live on the project record's `view_op`,
resolved before any new sync. A lost `set.new` there is found as any
creation is; a lost `set.sync` is not replayed but superseded by a
redraw of the current queue, since the layout it carried may be stale. An
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
process killed and read back as gone, the attempt's running checks
killed with their process group and read back as gone (or, past a
two-minute limit, sent SIGKILL and named in the cancellation reason),
and only then does the ticket read as parked or the rerun's answer
count as acted (which is what lets the replacement start); the
sequence runs from the saved intent on every pass, so a restart at any
point resumes it whole. A close does the same before it writes an
open attempt cancelled: its checks stopped under the same limit, then
its command reviewers killed. `dispatch park` writes the same intent
by command, and the runner's next pass finishes it. A resume is the
`rerun` answer, recorded as answered by `resume`, for each latest
attempt its park cancelled mid-run (its end logged after the park's
`parking` event) with no question about it withdrawn (`--no-rerun`
answers none). A resumed ticket asks `rerun` again, under a new id and
without waiting for a slot, about each other failed or cancelled
attempt with no rerun question in flight, with the attempt's own
reason (for a sent-back attempt, the note) and the options its failure
first had; a pending decision holds its stage only in the context it
is about. With every slot
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
exits. A plan review run the app reports failed (its reviewer or
planner exited or stopped without its round file) fails the attempt
the same way, with the ordinary rerun/park question and `max_reruns`;
a rerun kills the old run's agents and starts a fresh run, and a
`paused` question left from a pause continued in the app is cancelled.
Only `no such run` fails a review run's query or reads a paused run as
stopped; any other failed reply or socket error is asked again next
pass with one warning. A `none` check reading within two minutes
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
A rebase that conflicts after the last code review stage, with no PR
to push to, gets one resolution review of the range-diff between the
reviewed and the resolved branch before the stage runs, with the
conflict and its commits on the lane and its bring-up (dispatch
`RECORD_VERSION` 9).
A refresh holds a lane whose worktree is mid-rebase or off its branch
with a `refresh` question instead of reading it, and a failed
rebaser's `rerun` question holds the stage and reads the lane again
(no record change).
The `refreshed` event and `show` say who brought a lane up (git, the
rebaser, or a hand rebase adopted by a `recheck` or `rerun` answer) and
how many commits conflicted, derived from the ticket's attempts and
answers and the time the conflict was recorded (record v11), so a
rebaser from before the conflict is never credited; a bring-up after a
rebaser that failed or was cancelled, with no answer since, reads
`stopped`, since the record cannot tell the rebaser's work from a hand
rebase made before a park.
A ticket's tree with no chosen lane on its branch is brought up to the
project's base at every stage but a gate-only one, the joined tester
among them, recorded as the ticket's `tree_refreshed` with a copy at
each stage entry that a ranged restart restores (record v20), and
logged as a `refreshed` event naming `root`.
`dispatch park` writes the parking intent by command, and a resume
reruns the attempts that park cancelled mid-run, recorded as `rerun`
answers by `resume` (no record change).
A prompt's `{lanes}` is the lanes the ticket chose, in pipeline order
(every lane before the `lanes` decision), and `{lanes.all}` every lane.
A retake after close deletes each kept branch with nothing beyond its
base and asks `reuse | fresh | park` about one with commits, holding
the ticket at the cut; `close` lists the branches it keeps.
A fold or squash that cannot rewrite the branch asks `rerun | keep |
park`, and `keep` completes the review at the reviewed head with the
history as it is, once the head, the checks and a clean tree say it may.
A `one` fold takes the message and author of the last commit no fix
round made that is not a `fixup!` or `squash!` commit, so a spike's
commit no longer leads; this project's pipeline folds with `one`. The
agent stage right after a review that folds to `one`, or folds with a
stage declared `keeps_commit`, counts the branch's commits in the lane
before its agent starts and fails with `rerun | park`, listing each,
when there are more than one plus the kept stages (no record change).
A `lane:<name>` deploy with `without_lane = "base"` deploys the lane's
base for a ticket that did not choose the lane, instead of skipping:
it holds the stage's resource, runs as `<name>@base` in the project's
shared base tree (a detached worktree of the project's clone with the
lane's clone detached inside, moved to the bases and set up right
before each start, waiting while another ticket's base deploy runs
there), and its commit is what the tester and `tried` read. The tree's
path is recorded on the project (record v21) and `worktrees --migrate`
moves it when nothing runs in it.
`dispatch/tests/first_slice.rs` is the acceptance table, plus a test
per point above; `dispatch/tests/live.rs` runs the first stage against
a real Switchboard and a haiku agent. Known gap: Claude Code treats a
git repository as its own workspace for the folder-trust dialog, so the
first agent in each lane's worktree blocks on that dialog until it is
answered in the pane once (a plain directory under a trusted parent
inherits the trust; a fresh repository does not). Nothing in
Switchboard or Dispatch writes Claude's trust file. One more: `close`
(and `retake`, which closes by it) leaves pending decisions pending.
And: a ticket's tree with commits of its own whose rebase onto the
moved base conflicts is left stale with only a log line, and a ticket
with a chosen lane on the tree's branch has its tree brought up only by
that lane, so it is stale at a stage the lane skip rule skips, the
joined tester among them.
And: a `working` card left stale by a subagent's last hook, with the
main agent never taking another turn, holds the attempt until it is
parked; the Stop hook's `background_tasks`
(`spikes/03-session-state/README.md:94`) would say it exactly but is
not on the wire. And: the ticket page does not show that an attempt was
carried or how many points a round left to the merge (each would need a
field on the `dispatch-control` views), and a command reviewer cannot
tag a point as style. And: a check or command reviewer runs in its own
process group, so it outlives a runner killed by Ctrl-C or a closed
pane; the restarted runner stops it by its recorded group before
anything of it runs again, but a leaderless group with no fresh kill of
its own, and the one-`fsync` window before the group is saved, are
left running. And: the `refresh` question is asked per stage, not
per lane, so one `recheck` covers every held lane; a lane whose rebaser
finished and that is brought up in the pass right after another lane's
`recheck` reads `rebased by hand (adopted)`. And: a base that moves
cleanly under a ticket waiting at `merge` is not brought up unless the
owner answers `recheck`, so a provider that requires branches to be up
to date waits on that answer.

A supervising agent has commands of its own, all read-only:
`dispatch health` (the runner's `runner.json`, written after every
pass, plus one query on each socket, with repeated failures folded
into one line per ticket and error and `--verbose` to list each),
`show` (a ticket with its lanes' base, head and pushed head, its
rounds' files and a `paths` view of the files to read next), `events` and `wait` over `events.jsonl`, `report`
(stage time, plan and code review points counted by id, fix passes,
rebases, the PR's range from git), and `tail`, which reads a running
agent's pane through the new `session.screen` query with the project's
secret values replaced by their names. The event log is diffed at the
one ticket write path and synced before the record's rename, so a
crash repeats a transition and never loses one, and a failed write is
withdrawn with a `void`. Usage errors exit 64; `wait` exits 2 on a
timeout and 3 when the ticket parks or closes. `wait` reads the record
first, so a parked or closed ticket and a pending decision return at
once, and `--since <seq>` replays the log from a cursor. A supervisor's
watch re-arms from the seq it last printed (the same one after a
timeout), so a decision raised between two watches is returned and one
left pending is not returned again. Any event
the record does not bear out yet, live or replayed, is held until the
record's `updated_ms` passes its write. The agent guide is the
contract, and a test holds it to the usage. Gaps: `events.jsonl` is
never rotated; cost and turns are not recorded, so `report` says so;
tickets from before this build have no events until their next
transition; and a `void` whose own append fails leaves its events
standing, which only `wait`'s check against the record covers. A
replayed event of a kind with no record check is covered only by the
`updated_ms` hold. A ticket's `updated_ms` moves only on a write that
changes more than poll bookkeeping (`store::write_ticket_stamped`), and
every write that logs an event moves it. When the event's write and its
`void` both failed, three cases remain. An event the record bears
out, held or replayed, is returned only if no later line logs the same
transition (a decision by its id, a pull request by its url and
attempt; no other kind, since every other kind can recur for real); if
one does, the event gives way to that line, so a decision asked again
returns the write that landed, not the phantom. A replayed event with
no check of its own is let through once any later write that changes
the ticket lands; a write of poll bookkeeping alone does not move
`updated_ms` and lets nothing through. And the hold lets an event
through at once when the clock did not move forward between two writes
(the same millisecond, or a step backwards) and the later write's
rename failed.

A close's tree removal runs with the writer lock let go: `git worktree
remove` of a tree with a large build directory can take minutes, and
`take`, `decide` and `close` do not wait behind it. One finisher at a
time holds the ticket's `closing.lock` (a pass reaching a ticket a hand
close is finishing leaves it alone, and a second retry is refused),
the ticket and the project are read again once the lock is back, and a
lock that cannot be taken back refuses every later write in that
transaction. A retry of kept trees keeps its `trees_kept` mark until
the removal reads back, so a retry killed mid-removal stays retryable.
A command that waits more than two seconds for the writer lock logs a
line naming the holder, which writes `pid <n>: <command>` into the
lock file. Gap: fetches, lane setup, rebases, port calls and PR reads
on active tickets still run under a step's lock.

`dispatch restart <ticket> [<stage>] [--note <text> | --file <path>]` (`src/restart.rs`) gives a
ticket a fixed pipeline: it puts the ticket at its stage, or an earlier
one, under a fresh copy of the live file (`pipeline.<n>.toml`, the
first copy never rewritten), with stages mapped by name. The target
may be a stage only the live file has, when every live stage before it
was run; the work from the stage after it on is discarded. The intent
rides on the park sequence, so nothing moves until every process is
read back as gone. A ranged restart cancels the later completed work as
discarded and resets each branch with `git reset --keep` to the head
recorded as the ticket last advanced into that stage (`entered`, record
version 12), restoring the lane's base and bring-up record. Without a
recorded head, a branch stays where it is when nothing after the target
(an agent, workflow or review attempt in the range, or a rebaser after
the entry) can have moved it, or when it is at its base with a clean
tree; a gate-only command is assumed not to commit. Each reset
is saved as it lands, and a refusal holds the restart parked with its
intent, which `resume` will not drop. A lane whose `setup` changed runs
it again, before a `check` answer's checks too, failing the checks
rather than parking. No agent of the stage launches until its `rerun`
question is answered, unless the restart gave `--note`: the note is the
answer, so the agent launches without asking and `check` is not
offered. Without a note, attempts whose checks may be run again under
the new copy are offered `check`. A note (record version 23, on the
intent and the restart) is put on `rework` for each context the target
ran in, with where that context's latest attempt left its notes, and
the discarded attempt's reason quotes it so a `rerun` answer after a
park puts it back. A ticket from pull requests takes a ranged restart
only when every branch already stands at its target head, so nothing
is reset; a moved branch refuses it, and a reset is never run on one.
An active ticket keeps its holds through the restart except one a
deleted secret was made under, so it deploys again as a parked one
does, and the note waits for the tester. Gaps: no port body or
Switchboard button yet (the CLI only); a ranged restart asks `rerun`
again at each later agent stage it reaches, since those attempts are
discarded too; a ticket without entries can be restarted earlier only
where no branch moved after the target; a note on a never-run stage is
refused rather than held for its first attempt; a code review stage
takes no restart note; and a park before the agent launches keeps the
note only through a completed attempt's discard reason, so the note on
a failed or park-cancelled attempt at the current stage is lost to that
park.

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
set's members laid out. views.json is v7, so an older build leaves a
file with rules alone; a v5 file reads as hand sets.

Two settings sit beside the rule. `WorkingSet.running_only` leaves out
members whose pane is not running (`AppCore::is_running`). It is applied
where membership is worked out (`members_at`), so every reader agrees:
the cards, the keys and the controller, the rail's count, and the
control port's `items`. Before the first `HostListed` no pane is known
to be running, so such a set is empty for up to a second at startup; a
pane that dies leaves it within a second. `WorkingSet.card_scale` is
the cards' size in percent, 90 to 200 in steps of 10 (`RULE_SCALE`),
default 100; `grid::rule_card` turns it into units, with the width
growing 1.4 times as fast as the height (90 is 9 by 7, 100 is 10 by 8,
200 is 24 by 16), and clamps again for a hand-edited file. Both were
added in views v7; a v6 file reads with the toggle off and the scale
at 100. Clone and Move to carry them. The header shows them as a
"Running only" checkbox and a "Card size" slider after the hours.

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
lines: `new-recent-set`, `set-hours`, `running-only <set> on|off`,
`card-scale <set> <percent>`, `dismiss-from-set`; all but the first
take a multi-word set name (every word but the last). Hours from any
sender are clamped to 1 to 720. The rail lists rule sets before hand
sets, each in record order (`AppCore::working_sets_in_rail_order`); the
control port's `set` replies keep record order.

Known edges: a pane that redraws forever (`top`, a clock in a prompt)
cannot stay dismissed past its next redraw; Kill and dismiss is the way
out. Rule sets are rule-only (no hand pins beside the rule's cards).
The keys and the controller step across a rule set's grid as drawn:
each frame the view reports its width (`AppAction::ViewColumns`, a
transient measurement that saves nothing), and the core lays the set
out at it before `grid::neighbour` picks the next card. Right from the
last card of a row does not wrap; Down from above the empty end of a
short last row goes to the nearest card below by centre line, as on a
hand set. Only the control port, which has no view, lays a set out at
24 columns (`RULE_COLUMNS`), as the core does before the first frame.

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

## Test waits and times (2026-10-04)

A test that waits on a real process (a tmux pane, a child check, the
file index's thread) polls to a deadline through its file's helper and
never sleeps a fixed time before an assert, so a loaded machine makes a
test slower, not red. Dispatch's dirty-tree wait after an implementer's
response moved from a count of passes to the clock: five minutes from
the round's `dirty_since_ms`, which the record now carries (record
version 6), so a restart keeps the clock and the acceptance table steps
over it with `Env::wait`. Dispatch's integration tests skip the device
flush in `store::atomic_write`, which was most of their time on macOS.
`scripts/test-times.sh` measures every test serially against a budget
of 5 s a test, and the README records the slowest.
A test that waits on a child's setup waits on a marker the child
writes after that setup, because a deadline after the action cannot
order the action behind it: the check kill test waits for the inner
shell's `ready` after its `trap "" TERM`, and the long typing test waits
for the pane's `ready` after `stty -icanon` before it writes.

Known gap: tmux 3.4 on the Linux runner can leave a pane dead without
a status. The exit-code test accepts that on a tmux older than 3.5,
and CI (`scripts/ci-test.sh`) retries a failed test once and warns
`flaky:` when the retry passes. A test binary that crashes without
naming its failed tests fails CI with no retry.

## Frames while hidden (2026-10-04)

eframe 0.36 runs no egui pass while the root window is occluded or
minimized; it calls only `App::logic`, once per honored repaint
request. The tick, `pump` (the control port, the hook wake, the host
and hook-log polls), the tick's `request_repaint_after` re-arm and the
Dock badge therefore live in `SwitchboardApp::logic`, and so does
raising a session window the core asked for; `ui` only draws. Work
that must keep running while the window is hidden belongs in `logic`.
While hidden, eframe throttles repaints to one per 100 ms, so a
control request is answered within about 100 ms. Spike 12 has the
measurements.

The window counts as hidden when `ui` has not run since the previous
`logic` call, which is the condition eframe uses to skip it; a visible
popout keeps the root visible, by eframe's rule. Visible, every `logic`
call ticks and the host and hook log are polled once a second.
Hidden, `src/cadence.rs` backs the tick off to 5 s, and with it
everything that rides the tick, each up to 5 s late: every
`AppCore::tick` timer (notices, the update check, pairing and env-setup
expiry, trash undo, `workflow_tick`, the radial-menu dwell), captions,
the trust check, definition files, the Dispatch poll, the Dock badge,
pane death and remote change events. The radial dwell in particular is
not honoured promptly while hidden: a held stick sends no event, and a
controller event does not force a tick. The control port, controller
events and Dispatch answers are served on every `logic` call, so they
stay within about 100 ms.

A hook wake reads the event log at once, so a session that starts
waiting moves the badge within about 100 ms, but lists the host at most
once per second: `PostToolUse` fires per tool call, and a burst of
wakes gets one `list-panes` and one trailing list when the second ends.
On uncover the first `logic` still reads as hidden; `ui` sees that and
asks for one more pass, which lists the host, so the first visible
frame is at most one frame stale.

The saving is arithmetic, not a measurement: visible, about
`1 + N/2 + M/2` forks a second (one `list-panes`; a caption capture
every 2 s for each of the view's `N` running sessions; a trust capture
every 2 s for each of `M` running Claude Code agents whose state is
still unknown); hidden, about a fifth of that.

Known gap: the sleep and wake cycle and the Dock badge while hidden
were not measured; spike 12 lists the steps. A window merely covered by
others, rather than minimized or hidden, was not measured to be
reported occluded, so it may keep the visible cadence.

## Dispatch ticket page (2026-10-04)

A ticket's page is a header (title, actions, the meta row with the
lanes and the latest attempt's pull request as chips), the ticket's
pending decisions pinned under it on every tab, and a strip of tabs.
The chosen tab per ticket is `UiState.dispatch_ticket_tabs`, transient;
a tab click dispatches nothing. The meta row wraps by whole items: a
chip or the PR link that does not fit moves to the next row rather than
breaking inside, and the header and pinned decisions sit outside the
tab body's scroll. A long question folds past its first paragraph (see
"Long decisions fold").

- **Timeline**: `AppCore::ticket_timeline` groups rows newest first,
  a new group each time the stage changes. The rows are the runner's
  event log for the ticket, after the record's rows from before its
  first event less those the log has too, since a ticket in progress
  when the log began has its earlier history only on the record. The
  core marks the row each attempt's details (state, checks, PR,
  rewrite, rounds, session and artifact buttons, the chosen artifact
  as markdown) go under (`TimelineRow.details`): its end row, or its
  start row while it runs; attempts no row carries are listed last
  (`Timeline.earlier`). A decision row
  shows the decision's full question and answer, not the log's capped
  line.
- **Issue**: the title linked out, labels, the body as markdown.
- **Plan**: the plan as it stands, an open review's copy labelled
  with its round ("Reviewed copy, round 2: review open"), else the
  newest complete plan labelled with the stage that wrote it; an
  agent's prompt is still handed only complete files (`Ticket::shown`
  against `Ticket::input`). Then the plan review's rounds folded, each
  read when opened. While a plan review's `finalize` is pending
  (`dispatch::revisable`, which skips a workflow stage reviewing
  another subject), the tab pins the owner's note box and "Send my
  feedback" above the plan with the round count. Sending answers
  `revise` with the note, so Dispatch writes the round's feedback file
  and the tab lists it when it lands; Cmd+Enter sends, and an empty box
  sends nothing. On this page the pending card shows "Revise…" in
  place of `revise`, and no note box of its own; "Revise…" switches to
  the Plan tab and focuses the box, and the card's `finalize` and
  `park` send no note.
- **Notes**: every attempt's `notes` artifact, newest first, the
  newest open and the rest folded.
- **Review**: each code review attempt's rounds, their findings and
  responses folded, its summary, and what it did to the commits.
- **Changes**: per lane, the branch's commits over its base and the
  files it changes with their line counts, read through the
  `BranchChanges` port (`git log base..head`, `git diff --numstat -z
  -M base...head`, so a rename names both paths) on a thread when the
  ticket's `updated_ms` or the
  lane's range moves. The base and head are the status's lane, current
  every poll; the full ticket only adds the clone. The tree is the lane's worktree, or Dispatch's clone of the lane
  (`LaneView.clone`) once the worktree is removed. Open and Reveal are
  offered while the worktree stands. A click on a file's path opens its
  diff under the row (below).

Reads. The events read is `Body::Events { ticket, since }`; the runner
answers with the ticket's events after `since` from `events.jsonl`, the
highest seq it read as the next cursor, and the seqs this ticket's
`void` lines in the batch withdraw. The core caches them on
`DispatchState.events` with the cursor, and the ticket in full
(`Body::Ticket`, which alone carries `paths`, the plan review's round
files and each lane's clone) on `DispatchState.details`; both are
transient and fill on the page's first draw. The page asks only when
`AppCore::events_read_due` and `ticket_read_due` say so: connected, not in flight, and
not yet asked at the ticket's `updated_ms` from the status poll, so an
idle ticket costs nothing. `updated_ms` moves only when the ticket
changes in more than poll bookkeeping (settle counts, idle-poll
counters, a pull request's `checked_ms`), so a ticket waiting on a
decision keeps its place in the list and its "updated" time. A runner
from before the events read answers it `Failed`; the page then builds the timeline from the record
(attempts started and ended, decisions made and answered, restarts)
without a notice, and asks again when a runner reconnects. The single
ticket view is kept while the runner is away, so the plan and changes
still read. An artifact is read the same way (`artifact_read_due`,
`DispatchState.artifact_reads`): a failed read is said under its name
("Not written yet." for a missing file, since an agent writes its notes
after the runner hands out the path) without a notice, and asked again
once the ticket's `updated_ms` moves or its button is clicked. An
artifact with text is read again once the ticket's `updated_ms` moves
past the one it was read at (`DispatchState.artifacts_at`), since a
review edits its copy in place under one path; the old text stays
drawn until the new text arrives, and a failed re-read keeps it.
During a review `updated_ms` moves on each `revise` answer, revision
and `finalize` ask, so the copy is current whenever `finalize` is
asked. Times
read "3 min ago" with the local time on hover
(`cards::ago_ms`), the ticket table's too.

Known gaps:

- `DispatchReadArtifact` reads a path once, so a plan or notes file
  rewritten on an active ticket stays stale until the app restarts.
- `read_since` reads the whole event log on every events read; the
  log is never rotated.
- A failed artifact read is retried only on a change to the ticket
  other than poll bookkeeping, or on a click, so "Not written yet." can
  stay while an artifact settles.
- The Changes tab reads trees on this machine, so it cannot show a
  runner's on another machine.

## Supervisor sessions (2026-10-04)

A project whose live pipeline has a `[supervisor]` table gets one
long-lived Claude Code session that watches its tickets
(`dispatch/src/supervisor.rs`; the spec is "Supervisor" in
`docs/dispatch.md`). Built:

- The table (`guidance`, `read`, `setup` as one argv or several,
  `model`, `decides`), validated against the decisions Dispatch asks
  (`scheduler::DECISIONS`, held to the ask sites by a test) and the ones
  the file's gates ask; `stuck` is in the list but refused in `decides`,
  since its answer is the owner's. A template's `{decisions.<name>}` is
  checked against the same names only when a ticket takes the file,
  never on a running ticket's copy.
- `ProjectState.supervisor` (record v13): the workspace, the current
  session by Switchboard record id with its seed hash, the ones it
  replaced, an intent from the port, the one request in flight (made
  and recovered as a ticket's creations are), and the last error.
- `dispatch supervisor <project> [--fresh [--setup] | --resume |
  --kill [--reason]]`, `dispatch brief <project>`, and `dispatch
  events --follow --timeout <secs>` (exit 0 once something printed, 2
  when nothing came) for a supervisor's watch loop, beside `dispatch
  wait <ticket> --for move --since <seq> --timeout 540` as its watch on
  one ticket.
- The actor: `SWITCHBOARD_RECORD_ID` naming a project's current or
  past supervisor makes a command that supervisor's. It may read
  anything, `take` and `queue` on its own project, and `decide`,
  `park`, `resume --no-rerun` and `close` on its own tickets; a plain
  `resume` needs `rerun` in `decides`. Refused with exit 1: `restart`
  (unless `may` lists it, and even with it while the ticket runs a
  deploy), `run`, `worktrees <path>` and `--migrate`, `supervisor --fresh`,
  `--resume` and `--kill`, any verb not on the list
  (`supervisor::SUPERVISOR_VERBS`, held to `USAGE` by a test), and any
  other project. A `decide` outside `decides` saves a `Refusal` on the
  decision and logs a `refused` event, and its message names the
  owner's two routes (`supervisor::OWNER_ROUTES`). The answer's `by`, a
  ticket's `state_by`, its source's `taken_by` and an event's `actor`
  say `supervisor`.
- The control port's `session.resume` (non-replayable): resumes an
  agent's conversation with no terminal, leaves a running pane alone,
  and refuses a session that cannot resume rather than launching it
  fresh (`ControlAction::Resume`).
- The control port's `session.prompt` (non-replayable): `session.send`
  only when `AppCore::ready_for_prompt` allows it, checked and typed in
  one dispatch (`ControlAction::Prompt`), else a `Failed` naming the
  reason, which `SessionView.prompt_refusal` also reports. A key into a
  session's embedded terminal dispatches `AppAction::InputTyped`, a
  transient mark a submitted prompt clears. Dispatch's subscriptions
  deliver through it. A delivery leaves the message box's draft and
  the Prompt Box editor alone. Only the box's own Send
  (`AppAction::SendMessage`) settles its draft: it is cleared once the
  pane has the message, kept on a failed write, and given back to the
  editor on a failed editor write. A draft in a box does not hold
  deliveries off. Only keys in the embedded terminal do.
- The runner subscribes the supervisor to every active ticket of the
  project: `take` subscribes the new ticket (`by` `runner`, from the
  log's tail, with or without a supervisor seated), and each pass,
  before delivering, subscribes every Active ticket that has none, from
  one before its oldest `decision` line still waiting on the user when
  there is one, so a gate already open is delivered once. Parked
  tickets wait until they resume and closing or closed ones are
  skipped; an existing cursor is never moved. The seed no longer tells
  the supervisor to subscribe, which with `--since` would move the
  runner's cursor; `dispatch subscribe` stays for `--since`.
- The runner types each settled burst into its pane through
  `session.prompt`, which the core refuses unless
  `AppCore::ready_for_prompt` holds: a running Claude Code agent
  between turns by the same `between_turns` an owner's queued answer
  waits for, no trust question, no answer waiting, and no key typed
  into its embedded terminal within `TYPED_HOLD` (60 s) or since
  before its last submitted prompt, nor a `session.prompt` typed and
  not yet submitted (refused as "a prompt is being submitted", so the
  wait is not blamed on the owner). The check and the typing are one
  dispatch, and `SessionView.prompt_refusal` reports the same verdict
  so Dispatch asks with a query before it writes. A lost reply is sent
  again under the same op and answered from the operations log. The
  app keeps no queue: Dispatch's cursor is the queue, offered once a
  pass, so a delivery lands on the first pass after the `Stop`.
- A Fresh's `session.new` names the last session in `replaces`, so the
  new supervisor's card takes the old one's place on the owner's
  working sets, and each session is named `Supervisor · <project> ·
  <date> <time>` from its start in the runner's local time, while the
  Switchboard project keeps `Supervisor · <project>`.
- Dispatch's port: `supervisor-fresh` writes the intent the runner's
  next pass carries out; `ProjectView.supervisor` is the chip's data.
- The Dispatch page: a chip after each project's limits, `Supervisor ·
  working | waiting on you | idle | cold | asks to trust its folder |
  none`, with `seed changed`, `starting` and the error; Open, Resume
  (only for a record that can resume, since a return on any other
  launches it fresh with its first prompt), Trust, and Fresh behind a
  confirmation (`UiState.confirm_supervisor_fresh`). A decision card
  and the timeline say who answered.
- A rotation keeps one `## From the session of <date>` heading over the
  session's own text, replacing the ones earlier rotations left on top.
- `wait --for move` returns on what a supervisor acts on, and `--for
  any` / `--for move` return a burst: every match within two seconds
  of the last (ten at most), by the clock and by the lines' own times
  so a replay from an old cursor comes a burst at a time, ended at once
  by a decision and after the lines before it by a park or close,
  followed from the seq of the last event line.

Known gaps:

- The seed's subscribe lines changed, so a supervisor started before
  the runner followed every ticket reads "seed changed" until a Fresh,
  and until then its seed may still run `subscribe --since` and move a
  cursor the runner set.
- A ticket the supervisor unsubscribes is subscribed again on the next
  pass: it follows every active ticket.
- Keys typed into an outside terminal attached to the supervisor's
  tmux session are invisible to the typed mark.
- A draft left in the embedded terminal past `TYPED_HOLD` is still in
  Claude Code's input box: the delivery is pasted after it and both go
  in as one prompt, which arrives as the owner's `PromptSubmitted`, so
  the half-typed text reaches the agent and clears an ask as if the
  owner had sent it.
- After an Esc interrupt no `Stop` comes, so the pane reads busy and a
  delivery waits for the owner's next prompt and its `Stop`; after a
  `StopFailed` (a rate limit) it reads at a prompt until the next turn.
- A Fresh while a delivery is in flight drops it unsent and gives the
  new supervisor the same lines, which the old one may also have seen.
- An app that dies after typing a delivery and before writing its
  reply line has no record of the op, so the resend types it again;
  each line carries its seq.
- `deliver_answer` does not read the typed mark: an owner's answer
  given between a delivery being typed and its `PromptSubmitted` can be
  typed behind it, as with any `session.send`.
- Setup runs unconfined and under the writer lock, as a lane's setup
  does.
- With a non-default `DISPATCH_DATA_DIR`, the supervisor's commands
  need the variable, which its allow rule does not cover.
- The actor check is a guard against mistakes, not a boundary: an
  agent can unset the variable.
- A refused `resume`, `restart`, `runner`, `worktrees` or `supervisor`
  is an exit and a message, not an event, since no ticket carries it;
  a refused `runner stop`, `start` or `restart`, and a refused
  `restart <ticket>`, are kept on the project's record.
- Project refusals are not in `dispatch-control`'s `SupervisorView`,
  so the app does not show them; only `dispatch supervisor <project>`
  does.
- `Restart` has no `by`: the park a supervisor's restart makes is
  stamped `supervisor`, but once the restart applies only the
  supervisor's report says who restarted the ticket.
- The flags pass the allow rules on the command line; whether a second
  `--settings` would have replaced Switchboard's hooks was not tried.
- An older app drops `replaces`, so the old card stays pinned and the
  owner re-pins the new supervisor by hand.
- Two Fresh runs within one minute make two sessions with the same
  name.

## The rail scrolls (2026-10-05)

The rail's head (the workspace menu, All sessions, Dispatch) is fixed
and its bottom (the listening indicator, Settings, Go to, and beside a
session Terminal, Files and Notes) is pinned; the middle, the working
sets and then the project list or a session's neighbours, scrolls
between them. Before, nothing bounded that middle, so a long list
painted over the bottom items. The project list and each project's
neighbours keep their own offset, so a visit to a session leaves the
project list where it was. A change of view scrolls the selected row
into sight once (`UiState::rail_followed`), the rule the working set
grid follows, so a manual scroll is left alone. The scroll bar takes
egui's defaults, as the file tree does: it shows only when the list
overflows and floats over the rows while hovered.

Known gap: the offset is not persisted, as the rail's width is not;
after a restart the rail starts at the top and follows the restored
selection.

## The Dispatch runner as a service (2026-10-05)

The Dispatch overview's standing line is a toggle. Start makes one
`Service` record named `runner` in the `Dispatch` project beside the
console (remembered as `Settings.dispatch_runner`, made again if
removed) and launches it; Stop kills it. The record's launch is
`Launch::Argv(["/usr/bin/env", "RUST_LOG=info",
"DISPATCH_DATA_DIR=<Dispatch data dir>", "SWITCHBOARD_DATA_DIR=<this
app's data dir>", "<dispatch>", "run"])` with Dispatch's data directory
as its cwd: no shell, and the pane's pid is the runner's. Both data
directories are explicit because the tmux server keeps the environment
it was started with, not this app's, so without them a runner started
from a dev instance could work on another Dispatch directory or make
its sessions in the production app.

The record's launch is otherwise frozen, so `DispatchConfigured` (sent
by `start()` before the first host listing) and every Start rewrite it
from the executable the app found. In the bundle that is
`Contents/MacOS/dispatch`, so a rebundle plus Stop and Start runs the
new build. Outside the bundle a bare `dispatch` (no sibling binary) is
refused with a notice, since nothing says which one would run, and is
never written over the record, so the reconcile relaunches the last
absolute `dispatch` a Start wrote rather than whichever one is on the
tmux server's `PATH`.

`autostart` is the toggle's state: Start sets it, Stop clears it. It is
written by the app on a click, never read from a project directory, so
the trust boundary is unchanged, and the startup reconcile relaunches
the runner exactly when the last toggle was Start. **This is the one
deliberate exception to "agents are never resumed automatically".** The
runner resumes nothing itself, but it reads an attempt lost with the
old runner and starts it again for an active ticket, so after an app
launch or a reboot agents start and spend money without a click. It is
accepted because Start is a standing, visible instruction to run
Dispatch, as typing `dispatch run` was; Stop withdraws it durably;
Dispatch's own slots and parks still bound the work; and nothing else
Switchboard launches at start changes. The Start button's hover says
so.

Stop clears `autostart`, closes the open run as killed and kills the
pane. A killed runner can hold `runner.lock` for a moment, and a runner
launched then exits 1. So the core keeps a transient
`DispatchState::runner_stop`: a Stop that killed a pane is `Stopping`
until a status arrives `RUNNER_LET_GO` (`DISPATCH_POLL`, the app's
two-second status poll, plus a second) after it; a Start in between
queues as `StartWhenStopped`. A silent answer then launches it. A
runner still answering with the app's pane gone may be the killed one,
slow to exit, or one the app did not start; the port cannot tell them
apart, so the queued Start is dropped with `RUNNER_STILL_ANSWERS`, which
says to Start again once it is gone, and its `autostart` cleared. A
Start while a runner the app did not start answers the port is refused
with `RUNNER_OUTSIDE`, which the disabled Start's hover repeats. The
row's state is `AppCore::runner_standing`, and the refusal is
`runner_refusal`, which both the click and the button read. Board-card
Start and Stop on the same record stay plain service actions and leave
`autostart` alone.

The runner's pane carries its record id and launch token, and its
children inherit the id. A command gate of a stage with `env` runs
under `switchboard-env exec --` with the token, so it resolves the
runner's grants (`switchboard-env grant --runner <set>`); every other
child of the runner has the token removed. Every child is built
through `dispatch/src/child.rs`, and a test fails when a
`Command::new` appears anywhere else in the crate's code.

The row reads `runner up · pid N · K ticket(s)` with Stop, `runner
starting…` while the pane runs but has not answered, `runner
stopping…` or `starting after the old runner stops…` during a Stop,
`runner up outside the app`, `runner gone; showing its last status`, or
`runner stopped`; stopped, it adds the last run's kicker (`exit 1 · …`,
`killed · …`). A Runner section above the console shows the live pane
while it runs and the tail of the last run's log when it does not.

Known gaps:

- Stop is `kill-session` mid-pass. A check or command reviewer runs in
  its own process group, so a Stop leaves it running orphaned; the next
  runner stops it by its recorded group before the checks start again,
  the lost reviewer fails its round, or the attempt is cancelled. Left
  running: a leaderless group with no `orphans_killed` entry younger
  than the stop limit, and a group whose spawn the Stop cut off before
  the save that records it. Stopping the groups on Stop, or a graceful
  stop, needs a signal handler or a port call in Dispatch.
- A Stop during a deploy leaves the deploy running. The next runner
  waits for its group rather than killing it: a park, a close or a
  `rerun` holds until the group empties or the owner answers `stuck`
  with `released`.
- No restart on a crash while the app is up; the runner stays down
  until Start or the next app launch, like every service.
- A rebundle or an update with the runner up no longer waits for
  `dispatch runner restart`: the app restarts a runner on an older build
  itself once no attempt is open, and otherwise offers Restart on the
  row (below). `dispatch runner restart` (the owner's, or a supervisor's
  whose table's `may` lists `runner`) still asks the app over the
  control port to kill the pane without turning autostart off and
  start it again once the old runner lets go, refused while a ticket
  runs a deploy. A supervisor that runs the bundle script is still
  stopped by its permission prompt.
- The runner's children inherit `SWITCHBOARD_RECORD_ID` and `TMUX`.
- `runner stopping…` ends on the first status `RUNNER_LET_GO` after
  the Stop, answered or not. A kill that fails leaves the pane running,
  and the next host listing shows it `Up` again. A killed runner that
  takes longer than that to exit still answers, so the row reads
  `runner up outside the app` until it exits, and a queued Start is
  dropped with a notice to Start again.

### The runner's build (2026-10-27)

A runner keeps the binary it started with when the file is replaced, so
after an update (the self-updating app included) or a rebundle the old
runner went on serving with the old code: a pipeline the new CLI
validated, with a gate's `question` naming `{decisions.*}`, reached the
agent with the fields as written. (The issue named the CLI 0.5.0; a gate's
`question` and `validate_decision_vars` are in v0.6.0, so the validating
binary was 0.6.0 or later.)

- **What a runner records.** `dispatch run` stats its own executable
  once, before its first pass, since after an update the path names the
  new file: `BuildId { version, mtime_ms, len }`, the version stamped by
  `dispatch/build.rs` from the app's `Cargo.toml` (the crate's own
  `0.1.0` never moves). It goes into `runner.json` and the `Status`
  reply's `runner`; a runner from before the field sends it blank.
- **Stale.** `dispatch_control::stale` compares the recorded build with
  the version wanted and the file on disk now: blank is `Unrecorded`, a
  different version is `Version`, and a different mtime or length (both
  known on both sides) is `Build`. Inequality only, never "newer": an
  install may keep the archive's times. A rebuild with the same mtime
  and length is undetectable until a content hash is recorded.
- **The app.** Before each status answer the app reads the `dispatch`
  beside it (`DispatchPort::build_on_disk`, on the port's thread) and
  sends `DispatchBuildOnDisk`; `AppCore::runner_stale` is the rule above
  for a runner that answers, the app's or one started outside it. Each status poll
  then tries `restart_stale_runner`. There is no pass boundary to
  restart at (the runner has no pause or handshake), so the safe moment
  is "no attempt is open on any ticket" (`runner_is_busy`), with one
  exception: a `gate-only` attempt whose ticket has a pending decision
  at its stage only waits on the owner, whose question survives on the
  record. The view's decision carries its stage, not its attempt, so
  the match is by stage; a deploy has no pending decision of its own
  while it runs, so it stays busy. Busy, one notice says the runner is
  older and that Restart on the row restarts it now. The automatic
  restart happens at most once per build on disk (`StaleGuard`): the
  file can itself be older than the app (`cargo run` after building only
  the app), and then one notice says the restarted runner still runs an
  older build. A new file on disk is a new key and one more restart.
  A runner started outside the app is named and labelled but never
  restarted by it (the app restarts only its own); one notice says so.
  Known gap: "no attempt open" covers agent panes, reviews, resolution
  passes and gates, but not a lane bring-up's rebase, a lane `setup`
  (`cargo fetch`) or a `gh` read, which are no attempt, and the restart
  is a tmux hangup the runner does not catch, so an automatic restart
  can end one of those mid-way. That is the exposure every by-hand
  `runner restart` has had; the recheck path recovers it.
- **Restart on the row.** While the runner is stale the row reads
  `… · older build` with a Restart button (`DispatchRunnerRestart`,
  `SWITCHBOARD_SCRIPT` line `dispatch-runner-restart`). It is refused
  while the runner is not the app's own, not up or a `gate-only` attempt is at work
  (the CLI's deploy refusal, `mid_apply`); an open agent attempt does
  not refuse it, since the owner who clicks accepts restarting it.
- **The CLI.** `dispatch health` adds a line comparing the runner's
  build with its own, which is the app's too since the bundle ships both,
  by version only: the CLI may be another copy of the binary than the
  runner's (a development build beside the bundled one), and only the app
  knows which file a restart runs. It never changes `ok` or the exit code. `dispatch brief` prints the
  same line.
- **At run time.** `Vars::render_checked` refuses a pipeline template
  that names a `{decisions.X}` or `{decisions.X.note}` with no value:
  every decision a gate asks is set, empty while unanswered, so a
  missing one means a runner older than the pipeline file. A stage
  prompt or its guidance fails the attempt with `rerun`/`park` offered;
  a lane refresh's or a PR remedy's guidance fails that attempt the same
  way; a reviewer's or a fix round's prompt fails the reviewer or the
  round, and a rewriter's fails the message step; a gate's own
  `question` is asked without that block and the error logged, since no
  attempt exists to fail; a plan review's definition is rendered before
  the owner file or the attempt directory is written and parks the
  ticket. The check reads the template, not the output, so a
  note holding `{decisions.x}` cannot trip it. Other families
  (`lane.*`, `inputs.*`, `services.*`) keep `render`'s rule, since their
  keys are set only where they apply.

The thread's disk read travels on its own channel, ahead of the status
answer it goes with, rather than inside the answer: `DispatchDone` is
the port's one reply type for every call, and widening it for one body
would touch each caller for a pairing the channel's order already holds.

Not built: a pass-boundary handshake with the runner; the attempt key on
`DecisionView`, so the owner-wait match need not be by stage; a content
hash in `BuildId`.

## Bracketed paste in the embedded terminal (2026-10-05)

A multi-line paste into a Claude Code session in the embedded terminal
arrived as one Enter per line: the vendored widget wrote the clipboard
raw, so Claude Code submitted the first line and the rest landed as
separate input. The widget now frames a paste in `ESC[200~` ..
`ESC[201~` whenever the terminal has bracketed-paste mode (DECSET 2004),
with line breaks as LF and ESC dropped inside the bracket, and each line
break as CR when the mode is off (patch 4 in
`vendor/egui_term/SWITCHBOARD-PATCHES.md`).

The widget always sees the mode on, because its pty runs a tmux client
and tmux enables 2004 at the client on every attach. tmux passes the
markers through to a pane that asked for 2004 and strips them for one
that did not, so a plain shell pane still runs pasted lines one by one.

Known gap: off macOS a plain paste still sends ^V (an upstream hotfix);
only Cmd+Shift pastes the text.

## Secret artifacts (2026-10-05)

A gate-only command stage writes artifacts: its command gets
`DISPATCH_WRITES_<NAME>` for each, and its exit 0 fails the attempt
when one is missing. An artifact may be `secret`: the attempt records
which names are, the port refuses to read one, the ticket page draws
it as a label, and it is deleted (`Attempt.forgotten`, a `forgotten`
event) when the ticket leaves its stage's hold range, when its attempt
is replaced, failed or cancelled, and when the ticket parks or closes.
Agent sessions get each input their prompt names as
`DISPATCH_INPUT_<NAME>`, carried on the Switchboard record
(`SessionRecord.env`) so a resume keeps it. `{inputs.<stage>.<name>}`
names one stage's artifact and has its own variable,
`DISPATCH_INPUT_<STAGE>_<NAME>`. Both forms read the reader's own
lane from a stage that runs per lane, and a root or joined stage that
names a per-lane file is refused when the pipeline loads. Dispatch's
own reads of `plan` and `notes` follow the same lane rule: the refresh
and remedy agents' plan, a lane's human gate's notes line, and the
review workflow's plan (the reviewer's and the fixer's `{plan}`, the
decisions section, the message rewriter's input). A joined or root
code review lists every lane's plan in all three when the newest plan
writer runs per lane: `{plan}` is each lane's path with its lane in
parentheses, the decisions come one block per lane, and the rewriter's
input has one line per lane. A root or joined
human gate and `report` list one file per lane when the newest writer
runs per lane, and the one file otherwise, and read complete files
only; a lane's refresh notes stand in for that lane only. `show` and
the port's `PathsView.plan_files` and `notes_files` follow the same
per-lane rule, but `plan_files` shows an open review's copy for its
lane (`scheduler::shown_lane_files`), since they show the plan as it
stands rather than hand it to an agent.

Known gaps:

- The `checks` log holds whatever the command prints, and is readable
  on the Mac's ticket page and on the disk unmasked; the remote port
  masks it (Remote client, "Ticket text masked").
- The file is chmodded 0600 after the command exits, not created so;
  the 0700 attempt directory covers the window.
- No secret on agent stages: an agent that wrote one would hold it in
  its transcript.
- The consuming agent's transcript: a tester that reads the file holds
  its contents in its Claude Code transcript, which Dispatch cannot
  forget.
- Switchboard's scrollback (`<data dir>/scrollback/*.vt`) and run
  logs, if the agent prints the file. Only the path reaches a
  Switchboard record (`SessionRecord.env`); the contents reach these
  files only through the agent's own output.
- A root pull-request remedy over per-lane plans gets no plan clause
  (`plan_clause` is one path).
- An agent launched before the lane rule, or before a joined review
  listed every lane's plan, keeps the plan its prompt was given, which
  may be another lane's or none.
- A session launched before `{inputs.<stage>.<name>}` had its own
  `DISPATCH_INPUT_<STAGE>_<NAME>` keeps its old environment on resume:
  one `DISPATCH_INPUT_<NAME>`, which may hold the stage-qualified file.

## Environment sets (2026-10-06)

Credentials for one child, from named sets, without a pane's
environment holding them. `Settings.env_sets` holds each set's
variables (secrets by name only, values under `set/<name>/<VAR>`) and
its AWS method (`vault`, `sso` or `static`). `Project.env_sets` and
`SessionRecord.env_sets` are the grants; a session resolves its
project's sets, then its own, later variables winning, and two sets that
both give an AWS method are an error naming both. Schema v12.

- **Resolution** is pull-only. `switchboard-env exec -- <command>`
  sends `env.resolve { session, token }`; the app checks the token
  against `SessionRecord.token_hash`, resolves with
  `core::env::resolve_sets` and the Keychain at that moment (values are
  never cached), and answers the pairs. The binary removes the token and
  any `AWS_VAULT` from the child, and when a set gives an AWS method
  every ambient AWS key, token and profile variable too (the CLI prefers
  ambient keys to `AWS_PROFILE`); then it adds the pairs, and for
  `vault` runs the command under `aws-vault exec <profile> --`, for
  `sso` checks `aws sts get-caller-identity --profile <p>` first and
  sets `AWS_PROFILE`, and for `static` refuses unless the sets supply
  `AWS_ACCESS_KEY_ID` and `AWS_SECRET_ACCESS_KEY`. `env.resolve` is a query, so it never reaches the
  operations log; `session.screen` redacts a set's secrets too.
- **Tokens.** The app makes a token (two v4 uuids) at every
  `Effect::Spawn`, dispatches `RecordTokenIssued` so the hash is saved
  before the pane exists, and sets `SWITCHBOARD_RECORD_TOKEN`. A pane
  that outlives the app still resolves; a relaunch replaces the hash.
  The port's clone takes the sets Dispatch names, the planner clone gets
  none, and the user's own clone keeps the source's (it continues the
  same conversation, as it keeps `env`); all three start with no hash.
- **Setup** is `switchboard-env set create|var|secret|aws`, `grant` and
  `revoke`, accepted only within `ENV_SETUP_WINDOW` of the settings
  menu's Unlock environment setup (transient, closed by `Tick` and by a
  restart). A secret's value comes from the terminal or a pipe, never
  argv, and never travels in an upsert.
- **Dispatch.** `env` on an operator and a stage; an agent launch
  carries the union on `session.new` / `session.clone` and one prompt
  sentence naming the absolute `switchboard-env` beside `dispatch`.
  Validation refuses `env` on a non-claude operator, and on a stage
  whose agent, implementer, or the policy's rebaser or fixer is one.
- **Off the argv.** `spawn` writes its `new-session -e …` line to the
  tmux client's stdin after `start-server` (spike 18), so no value,
  launch token or pane command shows in any process listing.
- **Spike 14.** A pane's `-e` variables reach a grandchild. Check (e),
  whether Claude Code with Dispatch's launch flags runs
  `<abs>/switchboard-env exec -- true` without a permission prompt, and
  check (d), the `aws-vault exec` form against a cached session, were
  not run; the spike README has the commands. Until (e) is run, no
  allow rule is passed, and the fallback (`OperatorKind::env_flags`,
  `SessionClone.args`) is unbuilt.

Known gaps:

- No GUI editor for sets and grants; `switchboard-env` is the editor.
- The global and project layers still live in the pane's environment
  (over stdin, not argv).
- A `switchboard` server started by an older build keeps that spawn's
  values in its argv until it exits; killing it ends every session on
  it, and the values that appeared there should be rotated.
- An expired SSO session is reported, not refreshed.
- Codex operators are refused `env`: their sandbox may not reach
  `control.sock`.
- `env.resolve` writes no audit line.
- A pane launched before tokens cannot resolve until it is relaunched.
- A runner started outside Switchboard fails every `env` gate.
- The runner's grants serve every `env` gate it runs; a stage's `env`
  names on a gate are not consulted.
- An older app drops `env_sets` from `session.new` silently.
- An aws-vault prompt cannot be answered from an agent's Bash tool.
- Scrollback holds whatever a child prints.

## Asking the owner (2026-10-07)

A session can say it waits on the owner without a hook to say so: an
agent that ends its turn on a question, or a supervisor with a call to
make. `switchboard-ask "<one line>"` sends `session.ask { session,
token, message }`; `switchboard-ask --clear` sends it with no message.
Schema v14.

- **The record.** `SessionRecord.asking: Option<Ask>` (the message, cut
  to its first line and `ASK_MAX_CHARS`, and when the app took it),
  separate from `waiting_on`, which Dispatch sets and clears after every
  answer, so neither clears the other. The token is checked in the core
  against `token_hash`, as `env.resolve` checks it; a blank message is
  refused rather than read as a clear.
- **Showing it.** While the pane runs, an ask makes the card read as
  waiting on you and counts toward the Dock badge, the rail and the hand
  controller even after a Dispatch status has arrived (it is not
  Dispatch's decision). The message is the waiting reason on the board
  card, the working-set card (which now reads `waiting_reason` like the
  board), a lane agent's card on the Dispatch page, and the port's
  `SessionView.waiting_reason`. `AppCore::standing_ask` gives the whole
  message while the pane runs, and the surfaces read it so the ask is
  never hidden behind a reason that blocks the pane on top of it. The
  session page shows it whole on a wrapped line under the header with
  "Dismiss question", and the supervisor's ask has a wrapped line of
  its own under its project's row (`SupervisorChip.ask`; the chip's
  `reason` is `None` when it is the ask). The board card, the
  working-set card and the lane agent's card show it under a waiting
  reason that differs ("permission for Bash"), through
  `AppCore::ask_beside_reason`, which is `None` when the reason already
  is the ask; the board card clamps it to two lines and the working-set
  card to its body, so the whole question is on the session page.
- **Clearing it.** The owner's next typed prompt, sent while no turn is
  open and later than the ask (a prompt from the turn that asked does
  not count), the cards' "Dismiss question" (`DismissAsk`, the session
  is not told), the session's own `--clear`, or any new spawn of the
  record. What does not clear it:
  - A background task's notification or a harness reminder. Where
    Claude Code runs `UserPromptSubmit` for these, the tagged text is the
    `prompt`. The helper writes `"injected": true` on the line when the
    prompt, less leading whitespace, starts with `<task-notification>`
    or `<system-reminder>` (spike 17: the payload has no field naming
    the origin), and the adapter reads that as `PromptInjected`, which
    sets the session working and leaves the ask. The prompt itself is
    never written.
  - A prompt the helper reads as typed while a turn is open (the
    record's activity before the event is `Working`): the turn guard. A
    reminder the helper could not flag arrives inside the turn it
    interrupts, so it cannot pass for the owner's reply, even after an
    earlier injected turn has stopped. A `SessionStart` sets `Working`
    without opening a turn: Claude Code runs it on `/clear`, `/compact`
    and `/resume` as well as at launch, with no `Stop` after, so the
    record is marked in a transient `started` set that the next hook
    event setting an activity drops, and the guard ignores a `Working`
    it set. A
    `SessionStart` inside an open turn (an automatic compaction) keeps
    the turn open.
  - A prompt sent with `session.send` over `control.sock`, such as
    Dispatch's dirty-tree nudge. The send marks an asking session in a
    transient `relayed` set, only while the pane runs, since nothing
    reaches one that is gone; the next typed prompt event consumes the
    mark and leaves the ask, and a new spawn drops it. The app's own
    message box and Prompt Box (`AppAction::SendMessage`) set no mark, so
    the owner's sends clear. So does a send from a paired device, by
    `/send` or the draft's Send: the remote port serves it as the owner's
    and it reaches the core as `ControlAction::OwnerSend`, which sets no
    mark. The origin is the channel the app read the request from, set in
    the app and never a wire field, since any socket client could set
    one: everything on `control.sock` is a tool's, `session.prompt`
    included, even from a pane holding its launch token.
  - A prompt a workflow sends on its own account: a reviewer's
    `review_first` or `review_round`, the planner's `respond` after a
    Changes verdict, a Continue re-prompt, and the `/compact` at a
    Compact hand-off. Each marks an asking session the same way
    (`AppCore::mark_relayed`, shared with `session.send`). The owner's
    own words through a workflow (`respond_to_user` for a note,
    `workflow.object`'s objection, the `owner_round` a rerun opens with)
    set no mark and clear the ask as a typed send does. A send that
    becomes a launch's first prompt sets no mark either: the spawn
    clears the ask.
- **The supervisor.** Its seed gains one line telling it to run
  `<abs>/switchboard-ask` before it stops on something the owner must
  answer, and its launch flags an allow rule
  `Bash(<abs>/switchboard-ask:*)`, both only when the binary sits
  beside `dispatch`. The line is in `seed_hash`, so a running supervisor
  reads "seed changed" until a Fresh.

Known gaps:

- Spike 14 check (f), whether the supervisor's allow rule runs
  `switchboard-ask` without a permission prompt, has not been run; the
  spike README has the command. A prompt would be answered once per
  supervisor. When `<abs>` holds a space the seed names the binary
  single-quoted while the rule names it bare, so if Claude Code matches
  the rule against the command as typed, that path may prompt even
  after (f) passes; (f) runs an unspaced path only.
- A supervisor started before this asks nothing until a Fresh picks up
  the new seed.
- Lane agents are not told about `switchboard-ask`; their prompts come
  from the pipeline.
- A reminder injected while the pane is blocked at a mid-turn
  permission or question prompt (`WaitingOnYou` with a turn open) reads
  as typed if it carries neither tag, and clears the ask.
- A reply typed after an Esc interrupt: Claude Code runs no `Stop` hook
  on an interrupt, so the activity stays `Working` and the guard refuses
  the reply. The ask stands until it is dismissed or a prompt follows a
  later `Stop`. Spike 17 did not measure whether an interrupt fires any
  other hook.
- The `relayed` mark is transient. A send to a running pane that never
  becomes a prompt (Claude Code drops it) leaves the mark until the
  owner's next prompt, which then leaves the ask; a restart between a
  send and its prompt loses the mark, and the send clears the ask. The
  hand-off `/compact` is such a send if the hook does not report it as
  a prompt (not measured). A mark pushed for a Codex reviewer is never
  consumed, since Codex reports no hooks; it goes with the record.
- The `started` mark is transient too, while the record's `Working`
  persists. A restart between a `SessionStart` outside a turn (a
  `/clear` after the ask) and the owner's reply loses the mark, the
  guard reads the reply as typed inside a turn, and the ask stands
  until it is dismissed or a prompt follows a later `Stop`.
- The owner's reply sent from the message box while an injected turn
  runs reads as typed inside an open turn, and the guard refuses it.
  The ask stands until it is dismissed or a prompt follows that turn's
  `Stop`.

## Structured asks (2026-10-07)

A Claude Code session may ask for an answer of a shape, and the owner
answers on the card rather than in the pane. `switchboard-ask
"<question>"` takes `--confirm` (Yes or No), `--choice "<option>"` once
per option, or `--text` (one line of the owner's own); the wire's
`session.ask` gains `ask_kind` and `choices` (not `kind`, the request's
tag). Schema v15.

- **The record.** `Ask` gains `kind: AskKind` (`Note`, `Confirm`,
  `Choice(options)`, `Text`; older files read as `Note`, the plain ask
  above) and `answer: Option<String>`, the owner's answer while it
  waits to be sent. A choice takes 2 to `ASK_CHOICES_MAX` (6) options,
  each cut to its first line and `ASK_CHOICE_MAX_CHARS` (60), none blank
  or repeated. Any kind but `Note` is refused from a record that is not
  a Claude Code agent, since only Claude Code's `Stop` hook says when
  the answer can go in.
- **Answering.** The board card, the session page, the supervisor's
  line on the Dispatch page and a lane agent's card draw Yes and No, a
  button per option, or a line and "Send answer" under the question
  (`AppCore::answerable_ask`). A click dispatches `AnswerAsk { id, at,
  answer }`, where `at` is the ask's, so a click on an ask that was
  answered by a typed prompt, replaced by a newer ask, dismissed or
  cleared is refused with an info line; one on a pane that stopped
  since is refused as not running, and the ask stays. An answer that
  does not fit the kind is refused. A text answer's draft is kept with
  the `at` of the ask it was typed for and emptied when a newer ask
  shows. The working-set card shows the question but takes no
  answer; its body is clipped.
- **Between turns.** The answer goes into the pane as one typed line,
  `Owner answered "<question>": <answer>`, only when the pane runs and
  sits at its prompt: activity `Idle`, or `Working` from a
  `SessionStart` outside a turn (the `started` mark). An MCP dialog's
  `elicitation_complete` or `elicitation_response` notification sets
  `Working`, not `Idle`: the turn it interrupted goes on. A permission
  prompt, an `AskUserQuestion` or a failed stop (`WaitingOnYou`) is not
  between turns: keys there would answer that dialog. Otherwise the
  answer is kept on the record, and a second answer replaces it.
- **Delivery, at most once.** `deliver_answer` runs at every `Stop`
  and for every record holding an answer on every host poll. The
  second covers a restart: the app polls the hook log before tmux, so a
  replayed `Stop` lands while no pane reads as running; it still sets
  the record `Idle`, and the first host list sends. The ask clears in
  the same step that emits the send, and the save precedes the send in
  the effects, so a crash or a failed write loses the answer rather
  than sending it twice (a duplicate prompt is a paid turn). The send
  is `Effect::SendAnswer`, which the app writes like `SendInput`, and
  like it settles no draft: only the box's own `SendMessage` does. It
  ends a pending discard's undo like any send, and sets no `relayed`
  mark; the prompt it starts comes back between turns with no ask left
  to clear.
- **The typed reply wins.** A prompt typed between turns clears the ask
  and any answer on it, as before. When that drops a stored answer
  (after a failed stop), an info line names it: sending it after the
  owner's own reply would be a second, stale one.
- **The supervisor.** Its seed line on `switchboard-ask` names the
  three flags and the shape of the reply. It is in `seed_hash`.
- **Measured** (spike 19, Claude Code 2.1.293): an answer typed between
  turns and one held through a turn each arrive as the next user
  message, submitted once.

Known gaps:

- Codex panes and shells take plain asks only.
- A Codex agent's ask is never cleared by a prompt, the owner's or a
  workflow's: Codex reports no hooks and no turn signal, so only
  Dismiss question or a new spawn clears it. A Codex turn signal is a
  separate piece of work.
- A pane that dies with an answer pending loses it: a new spawn clears
  the ask.
- A failed write loses the answer; the error toast ("send input to
  <host>") is what remains, and the owner retypes it in the pane.
- A new `switchboard-ask` in the same turn replaces the ask and a
  pending answer with it.
- A running supervisor learns the flags only after a Fresh.
- The card keeps reading "sent when the turn ends" for an answer that
  will not go out until the owner dismisses it:
  - an Esc interrupt runs no `Stop` hook, so the activity stays
    `Working` and an answer stored in that turn waits for a `Stop` that
    does not come;
  - the `started` mark is transient, so after a restart an answer
    clicked at the prompt that follows a `/clear` reads the pane as mid
    turn and is stored, and no `Stop` follows to send it.
- By design: after a failed stop the owner's typed prompt drops a
  stored answer rather than send it after the prompt; an info line
  names the dropped answer.

## Multi-line sends paste (2026-10-07)

A message with a blank line, sent from the message box or Prompt Box,
reached Claude Code cut short: `write_line` typed the text with
`send-keys`, so each LF arrived as typed input and a blank line
submitted the part before it. `write_line` now sends text that holds a
line break as one paste, `load-buffer -` then `paste-buffer -p -r -d`
(spike 15): tmux brackets it in `ESC[200~` .. `ESC[201~` only for a
pane that asked for bracketed paste, so a shell or service never sees
the markers; `-r` keeps LF, and `-d` leaves no copy in the server's
buffer list. The text goes over stdin, never a command line. Line
breaks go as LF and ESC is removed first, the same rule as the embedded
terminal's paste. Single-line text is still typed. Enter follows as a
separate write after the paste returns, so it lands after the closing
marker and submits once. Dispatch's `session.send` and the handoff
prompts go through the same call. Claude Code records such a message
inside `<pasted_content>` tags, as it does any paste.

## File links in the conversation (2026-10-08)

A path an agent writes in its prose is a link that opens the file in
the full document view. It links in the agent's messages and in each
turn's final answer, never in the user's prompt or the tool rows. A
token links when it is path-shaped (it starts with `/`, `~/`, `./` or
`../`, holds a `/`, or is a bare `name.ext`; it has a letter and no
`://`), whether in prose or as a whole code span, outside code blocks,
links, images and headings, and when it names an existing regular file.
A `:line`, `:line:col` or `:line-line` suffix gives the line.
Punctuation a sentence hangs on the end is dropped. A token right
after `!` is not linked, since `![..](..)` would draw as an image.

An absolute path is taken as it is and `~/` is under home. A relative
path is tried under the session's project root, then under the
session's working directory. The file opens under the project whose
root holds it most closely, so Pin works for a file of any project, and
under the session's own project when no root does
(`AppAction::ShowFileRef`). The line rides beside the view in the core
(`document_line`, with a request number so a repeat click scrolls
again) and is cleared by every other show, the same file's from the
file tree too, and by Back; nothing about it is saved.

A text file is drawn unwrapped in the full view, so each source line is
one row: it scrolls to the line with three lines above it and marks the
line with a faint band, and the header shows `path:line`. The previews
in working sets and workflows still wrap. A Markdown file opens at the
top, since rendered Markdown has no line map.

The finder and the rewrite are pure, in `core::file_refs`; whether a
path exists is `adapters::files::existing_file`, a `stat` per token on
the UI thread. It is made the first time a message text is drawn and
cached per record and text hash in
`UiState.file_links` for as long as that text is in the conversation.
A re-read of the transcript keeps the entries of texts still there, so
only new messages are scanned. A file created after its message was
first drawn links only after a relaunch.

## A file's diff on the Changes tab (2026-10-08)

A click on a changed file's path on the ticket page's Changes tab shows
its `base...head` diff under the row; a second click closes it. One
file is open per lane, which lives in `UiState.ticket_diff_open`
beside the expanded folds, so a relaunch shows none.

The diff is read through `BranchChanges::diff` with pinned flags:
`git --literal-pathspecs -c core.quotepath=off diff --no-color
--no-ext-diff --no-textconv -M --unified=5000 base...head -- path
[old_path]`. The path comes from numstat, never from a diff header, and
goes after `--` as a literal path, so `[id]` or a leading `:` in a name
is never a glob or pathspec magic. Output over
1 MiB is not parsed, and a diff body over 5000 lines (`DIFF_LINE_CAP`)
is `TooLarge`; both say "Too large to show here".

The context equals the cap, so a file under it comes back as one hunk
holding the whole file, and folding is a pure view over its context
runs (`core::diff::runs`): a run between two changes keeps three lines
each side, a leading or trailing run keeps the three nearest the
change, and a fold that would hide fewer than four lines is not made.
Expanding a fold is view state and never reads git again; it is keyed
by the new-side number of the fold's first line, so it stays with those
lines when the head moves and the diff is read again.

`adapters::diff::parse` is a hand-written reader of that one format:
the extended header (`new file mode`, `deleted file mode`, `rename
from`, `Binary files`), then `@@` hunks numbered on both sides, `\ No
newline at end of file` flagging the line before it, and an empty line
read as blank context (`diff.suppressBlankEmpty`). The crates looked at
either have no notion of git's extended headers or are unmaintained.
Changed words are marked on the read thread: each run of removed lines
is paired with the added run right after it, line by line, and each
pair goes through `similar`'s word diff with its default config (no
deadline, so the marks are deterministic). A pair with a line over 500
bytes, or with fewer than half its words equal, is a rewrite and gets
no marks.

The core holds each diff in `DispatchState.diffs` by ticket, lane and
path, with the `DiffRange` it answered: the lane's tree, base and head,
and the ticket's `updated_ms`, since the head may be a branch name that
never moves. `ReadFileDiff` is sent only when nothing is on its way and
nothing good is held for that range; the page asks
`AppCore::file_diff_due` each frame, so a moved head re-reads while the
held diff stays drawn. A failure is not due again at the same range;
Retry dispatches the read, which the core then sends. The read is an
`Effect::ReadFileDiff` that the app runs on a thread of its own and
drains in `logic` as `FileDiffRead`, repainting every 50 ms while one
is out. Nothing is saved.

The diff draws each run as one highlighted block, unwrapped and
scrolling sideways per hunk, beside a gutter of old and new line
numbers in the same font. Added and removed rows are tinted full width
(`diff_add`, `diff_del`) and their changed words take `diff_add_word`
or `diff_del_word`. While the lane's worktree stands, a new-side number
opens the tree's file at that line (`AppAction::ShowFileAt`), under the
project whose root holds it most closely, else the ticket's
`root_project`, else the active one. The `ticket-diff <id> <lane>
<path>` script line opens a file's diff.

Known gaps:

- The gutter jump opens the worktree's file at head's line number, so
  uncommitted edits in the tree can shift it.
- A removed lane with no clone shows nothing, as the list does.
- Only egui's built-in languages are coloured; everything else is
  plain.
- `DispatchState.diffs` is never evicted while the app runs.
- No side-by-side view yet.

## Held stops (2026-10-08)

A Claude Code `Stop` ends a turn, not always the work: an agent can end
its turn with a background agent or shell still running, or with a
`ScheduleWakeup` set, and write its file in a later turn. The `Stop`
payload says so in `background_tasks` and `session_crons`
(`spikes/20-stop-pending`). The helper keeps each task's `type`,
`status` and `agent_type` and each cron's `schedule` and `recurring`,
never a description, command or prompt, and writes them as `tasks` and
`crons` on the `Stop` line. The hook adapter turns them into a
`StopPending` (task kinds, wakeups with a fire time read from a one-shot
`M H * * *` schedule in local time) on `EventKind::Stopped`.

The record keeps the last snapshot as `SessionRecord::pending_at_stop`:
replaced on every `Stop`, cleared on `SessionStart`, `StopFailure`,
`SessionEnd` and a new spawn. `StopPending::held_until(stop)` is the
rule, in one place, read through `SessionRecord::held_until`: a one-shot
wakeup holds until its time plus `STOP_GRACE`, but never past
`WAKEUP_HOLD` (65 minutes, `ScheduleWakeup`'s longest delay and its
rounding) after the `Stop`, so a schedule whose minute had passed by the
`Stop` and rolled to the next day cannot hold for a day; background
work, a recurring cron or an unreadable one holds for `BACKGROUND_HOLD`
(30 minutes) after the `Stop`; the later of the two wins. The card still reads idle, with "idle: N pending" as
its state text, so a held agent never counts as waiting on you and a
`session.prompt` is not refused as busy.

The app's plan review (`fail_if_stopped`) fails a round only once the
grace is past and the agent is not held (its pane runs and the hold is
in the future); the stall notice and mark are skipped while held. The
control port's session view carries `pending`, `wakeup_at_ms` and
`held_until_ms`. Dispatch's `held(view, now)` resets the idle count on
the missing-artifact path only (stage agent, reviewer, implementer,
rewriter): a present artifact completes as ever, and a `working` card is
still `busy`. While held, the attempt records `held` per session, which
`dispatch show` prints; a failure after the hold ends names what the
last `Stop` still listed.

A subagent's hand-back and a fired wakeup start a turn the owner did
not type, so the helper writes `injected` for both and an open ask and
a waiting card answer survive them. The hand-back is recognised by its
`<agent-message` tag. A fired one-shot wakeup carries no tag: its prompt
is the cron's `prompt` verbatim (spike 20), so on every `Stop` the
helper writes an FNV-1a hash of the session id and each listed cron's
trimmed prompt, never the text, to `crons/<session_id>` (0600, temp file
and rename), and a `UserPromptSubmit` whose hash is in that file is
injected. Each helper call is its own process, so the file is how the
`Stop` reaches the later prompt; the log keeps only the verdict. The
file is replaced on every `Stop` (removed when the list is empty or
unreadable), removed on `SessionStart` and `SessionEnd`, since crons
belong to one `claude` process, and any file older than 24 hours is
swept on each `Stop`, which covers a pane killed without a
`SessionEnd`.

Known gaps: there is no "the session is alive: wait" answer, so a pane
still holding when the cap runs out fails into `rerun | park`. Only a
one-shot wakeup is claimed: a recurring `CronCreate` and a `/loop` are
unmeasured, so whether they fire their listed prompt verbatim is
unknown. A prompt the owner types byte for byte like a listed cron's
reads as injected. A wakeup that fires after a `/compact` and before
the next `Stop` reads as typed, because the `SessionStart` dropped the
file.

## Evidence directories (2026-10-09)

A stage may keep the files it produced beside its notes, mainly a
tester's screenshots, in one evidence directory:
`writes = ["notes", { name = "evidence", dir = true }]`. It is not a
file artifact: `Stage::write_names()` leaves it out, so nothing that
assumes a file (the missing-artifact check, the artifact buttons, the
port's inline read) sees it, and no other stage reads it. The attempt
records it as `Attempt.evidence`: made under the attempt directory,
mode 0700, before the launch; `{evidence}` in the prompt and
`DISPATCH_WRITES_EVIDENCE` in the session's env (kept on the session
record, so a resume keeps it), the same variable for a gate-only
command. While the writer settles, the directory is one settle entry
(total size, newest time); when it finishes, one walk that never
follows a symlink lists its regular files, with anything over
`evidence_file_mb` or past `evidence_attempt_mb` kept but not listed.
The listing is recorded once and never re-read, so the ticket page,
`show`, `report` and `dispatch evidence` agree after a sweep. The
`tried` question names the count.

A closed ticket's directories are removed `evidence_keep_days` (30)
after `close.closed_ms`, by a sweep the runner runs once a day, or at
close with `dispatch close --drop-evidence`; the attempt keeps
`evidence.swept` and the event log an `evidence-swept` line. The
ticket page lists each file with its size and age, Open only for
documents and images (`evidence_opens`, since an agent's file has no
quarantine flag), Reveal for every file, and View for `.md` and `.txt`
through the port, which reads an evidence file only up to 768 KiB,
the same bound as every ticket file.

Spike 21 measured which writes reach the directory unasked: the Write
tool does under the attempt directory's existing allow rule; Bash `cp`,
`mv` and `mkdir` ask even with `--add-dir`, so no flag was added; a
Playwright MCP screenshot reaches it only when the operator's server
runs with `--allow-unrestricted-file-access`, the owner's choice.

Known gaps: a runner on another machine records paths that do not open
here. Codex is refused, since it writes only inside its cwd. Images are
not drawn inline. Keep days are read from the ticket's copy, so
shortening them does not reach older tickets. Bash copies into the
directory ask for permission, and a screenshot needs the Playwright
flag above.

## Long decisions fold (2026-10-10)

A decision card draws its question's first paragraph (up to the first
blank line) always. The rest is drawn whole when it is 8 lines or
fewer, counted by newlines so the default does not depend on the
window's width; past 8 it folds behind a "… N more lines" toggle that
reads "Show less" when open. The toggled state is
`UiState.dispatch_question_folds`, by ticket and decision id (decision
ids repeat across tickets): transient, kept for the run
across the overview and the ticket page, and dropped when the decision
is answered from a card. The overview and the ticket page draw the same
card, so both fold.

Dispatch's `inspect` question carries at most 6 `diff --stat` file
rows, then "… and N more files" and git's totals line; the ticket
page's Changes tab lists every file. The scheduler finds the rows by
counting back from the totals line rather than matching `" | "`, which
a commit subject can contain, and leaves a summary with no totals line
as it is. `GitCli::summary` runs `diff --stat` with `LC_ALL=C` so the
totals line is English under any locale.

Known gaps: several pending decisions can still push the ticket page's
tabs down on a short window. Decisions already pending keep their long
question until answered, and `report` and `show` print it whole.

## Dispatch definitions pruned (2026-10-11)

Dispatch installs one reviewer definition per operator and content
hash, named with `DISPATCH_WORKFLOW_PREFIX` (`"Dispatch: "`), and never
deletes any. Switchboard prunes them: a Dispatch install, and every
store load, drops each `Dispatch: ` definition that no unfinished run
in any workspace names, keeping the one just installed. Unfinished is
every state but `Finalized` and `HandedOff`: a `Failed` or `Paused` run
can be continued, `Converged` and `AtCap` take more rounds, and each
reads its prompts from its definition, where a missing one would fall
back to the built-in's, which do not name the ticket's worktree. The
prefix is reserved, so an owner's definition named this way is pruned
too; an owner install prunes nothing. The prune at load runs after the
store's notices, so a locked load prunes memory only and writes no
settings. The pruned list reaches disk through the usual
`SaveSettings`; there is no record change.

The Review plan dialog never lists `Dispatch: ` definitions, since
Dispatch starts its own reviews. Past six definitions it draws a
"Definition" menu instead of radios, so its height no longer depends on
how many there are.

Known gaps: a finished Dispatch run whose definition was pruned
refuses an owner's note and a handoff with an error, rather than run
them on the built-in prompts; the owner starts a fresh review instead.
An owner-named run whose definition was deleted still falls back to the
built-in. Dispatch's install and start are two requests, so a second
`dispatch` process installing between them can prune the first
definition; the start then fails with "no workflow definition called …"
and Dispatch fails the attempt, which a retry recovers.

## Remote client (2026-10-12)

A native visionOS and iPadOS client that browses the sessions and
working sets of several running Switchboard installations and sends
prompts to them is specified in `docs/remote-client.md`: a port over
the control port's wire contract, pairing with a pinned host key, one
revision counter and `changed` events, and content work off the frame
thread. The plan is Spike 22 and then Milestone A as five commits.

Built so far:

- **A0, the conversation cache in the app.** Parsed transcripts moved
  from `UiState` to `SwitchboardApp::conversations`
  (`content::ConversationCache`): each entry keeps the handle and file
  time it was read through, `is_current` answers the refresh without a
  read, and the views reach the cache through `DrawCtx::conversations`.
  A seeded conversation (tests, the script) has no handle and no file
  time and is never read over. Behaviour is unchanged; the UI tests
  seed the new place.
- **Spike 22** (`spikes/22-remote-port`). A pinned-TLS HTTP and
  WebSocket port on std threads with the frame as the executor works
  as specified: the frame's share is under 0.25 ms per pass, a
  request's latency is one frame interval (a client that sends on the
  reply synchronizes with the frame), the connection pools, the
  handshake timeout and the subscriber overflow hold under a stall and
  an unread socket, a wrong pin fails at the handshake, and `mdns-sd`
  advertises a service `dns-sd` sees. The largest transcript on this
  machine (146 MB) reads and parses in 290 ms at 575 MB peak, which
  settles that content work runs off the frame and argues for one
  content worker rather than one per connection. ctime and inode see
  the same-size rewrite, the rename and the regrown log that mtime,
  size and length miss. Not measured: the visionOS pinning delegate
  and the read-model diff.
- **A1, identity and an authenticated read-only host.** `identity.json`
  holds the server id, the display name, the self-signed certificate
  (made with `rcgen` on first start) and its SPKI hash, and where the
  port binds (loopback until the owner allows the local network, port
  43187 or `SWITCHBOARD_REMOTE_PORT`); the private key is a Keychain
  item under `remote-key:<server id>`. A certificate whose key is gone
  is remade over a new key under the same id, `devices.json` is cleared
  and the dialog says so; a Keychain error keeps the identity and the
  port does not listen. `adapters/remote.rs` is the TLS listener in the
  control socket's shape, with the pools, deadlines, body cap and rate
  limits of the spec's §4.1, handing at most eight requests per pass
  to the frame; `app.rs` authenticates the bearer, routes the
  allowlist (`/info`, `/pair`, projects, spaces, sets, one set,
  sessions, one session, waiting, screen) onto the control port's
  `answer`, bounds every string in a reply (4 KiB, notes 64 KiB alone,
  each cut named in `truncated`), caps the body at 1 MiB and wraps it
  in `{epoch, revision, body}`; everything else is refused by name,
  before translation. Pairing: the Remote devices dialog (settings
  menu) opens a five-minute window with an eight-character Crockford
  code shown as a QR carrying the server id, key hash, code and
  address; `/pair` must carry the host's own key hash and the code.
  Failures are counted per source address: the third from one address
  locks that address out for the rest of the window (403
  `pairing_locked`, listed in the dialog and named in a notice) while
  the window stays open for everyone else, and reopening clears the
  lockouts, and the window closes once failures from all addresses
  reach `PAIRING_FAILURES_MAX` (three for each of 32 tracked sources),
  so rotating addresses cannot guess until it ends; a body with no key
  is refused 400 without counting.
  `/pair` and `/info` are rate-limited per source address, over a map
  of at most 256 addresses (past it the stalest address's window is
  dropped, so rotating addresses resets the limit; the total of
  failures above is the bound there). The first
  success spends the window, and the device gets a 43-character bearer token
  whose SHA-256 is what `devices.json` keeps. The dialog lists devices
  with scope (read or control, changed there and nowhere else), last
  seen and Revoke. Not in A1: events, the content worker (the screen
  is answered on the frame as the control port answers it, until the
  screen cache of A4), Bonjour (A2), and `server_name` lives in
  `identity.json` rather than `Settings`, which the spec now says.
- **A2, two hosts and the first client.** The host advertises
  `_switchboard._tcp` through `mdns-sd` (`adapters/bonjour.rs`, a stub
  off macOS) while the port is on the local network, under the display
  name with a short id suffix and TXT `v`, `name` and `id`; a rename
  re-advertises, loopback withdraws. The Swift package under `client/`
  is the protocol (the wire views as `Codable`, decoded against
  fixtures the Rust port writes and checks), the transport (`URLSession`
  with a delegate that accepts only a certificate over the pinned
  P-256 key, and `Network.framework` browsing), the core (known servers
  apart from discovery, credentials in the Keychain for the apps and an
  owner-only file for the command-line browser, one `ServerConnection`
  actor per host with the §6.2 states, the per-key revision rule, the
  epoch check, a key change dropping the token and keeping the entry,
  and the unified store with source labels and offline marks) and a
  SwiftUI browser. Measured on this Mac with two throwaway instances:
  discovery finds both, the command-line browser pairs with each and
  shows both hosts' sessions under distinct labels, a killed host's
  items stay visible marked offline while the other keeps refreshing,
  and the host back on another port is reconnected under the same pin
  within one refresh, because a host that is not connected is resolved
  again from its Bonjour record on every tick (a killed process sends
  no goodbye). Not in A2: the visionOS and iPadOS app targets (an
  Xcode project; the package builds for both), events (A3), and the
  Swift tests are not in the pre-commit hook or CI.
- **A3, synchronization.** The revision moves from a read-model diff:
  `core::revision::Fingerprints` hashes every exported view (sessions
  with `quiet_secs` held still, sets, projects, spaces, and each
  collection's membership) and the app compares two passes, once per
  pass that dispatched something while the port is bound, before
  requests are served and again at the end of the frame; one `changed`
  or `removed` per resource that moved, the counter bumped once. The
  diff is the gate rather than the effect list, so a set rename, a
  trust question and a quiet Codex pane all surface with no save, and
  ten passes over a running pane emit nothing. Measured: 0.4 ms per
  pass over three hundred sessions in a release build (3.3 ms debug),
  against the spec's 1 ms; the first cut hashed each view's JSON with
  SHA-256 and took 1.1 ms, so the wire views derive `Hash` and are
  hashed directly. `GET /api/v1/events` with an upgrade is the events
  socket: `hello` with the revision goes into the subscriber's queue
  on the frame before anything later can move the counter, so a
  snapshot taken after it reconciles against everything that follows;
  the connection thread runs the socket non-blocking (an earlier cut
  blocked up to 100 ms in a read between events, and any burst past
  the queue inside that window dropped every subscriber), writes with
  the 10 s deadline, drops a subscriber that fell 256 events behind
  with `resync_required` and the close, and closes on Revoke. The
  control port's `serve` logs a non-replayable command's request line
  before it runs and refuses to append nothing; an op with a request
  line and no reply is answered `interrupted` and never run again;
  `/api/v1/ops/{op}` reads the status. `SessionView` gains the
  additive `last_activity_ms`. The Swift client opens the socket
  through the same session and pin, waits for `hello`, buffers frames
  while it fetches the collections, applies the buffer against each
  resource's held revision (a member falls back to its collection's),
  refetches only past the held revision, drops on `removed`, starts
  over on a new epoch, and reconnects on `resync_required` or a closed
  socket under its backoff; the timer refresh is gone, the manager's
  loop retries only hosts that are not connected. Not in A3: watched
  content (A4) and the `runs` collection, which has no endpoint yet.
- **A4, watched content.** A screen read is the spec's two-step
  handoff: the frame checks the record and resolves the redaction
  policy (the project's secret values and its granted sets', with a
  fingerprint), one content worker thread (`content::ContentWorker`,
  over the same `Arc<dyn ProcessHost>`) snapshots the pane's whole
  window, hashes it as the source version, masks and trims it, and
  reports back; the frame assigns the key a new revision and emits
  `changed {screen, id}` only when the source or the policy moved, and
  stamps the reply with the key's revision, so two reads of one text
  carry one revision however many lines each asked for, and a secret
  added to the project moves a watched screen though the pane did not.
  A policy that fails to resolve refuses the read; nothing is served
  unmasked. The worker polls the watch set every two seconds: every key
  a subscriber sent `watch` for (answered `watching`, never refused)
  plus every key read in the last minute, capped at 128 with the least
  recently read evicted and re-entered by its next read; `unwatch`, a
  closed socket and a revocation each drop their watches. The policy
  is resolved and cached on the frame, cleared by any save or secret
  change, which is the one Keychain read left there until Milestone C.
  The control port's `session.screen` shares the policy and refuses
  the same way. The Swift client counts holders per screen, sends
  `watch` at the first and `unwatch` at the last, reads the screen on
  `changed` past its held revision, resends every watch after `hello`
  on a reconnect before it snapshots, and the command-line browser's
  `screen <session id>` shows a pane live. Measured with the real
  adapter: five tests in `tests/remote.rs`, including four stalled
  strangers filling only their pool and a fifth closed at once while a
  paired subscriber keeps receiving. Not in A4: the conversation, file
  and run-output content kinds (Milestone C), which take the same
  worker and cache.
- **B1, the app shell.** The visionOS app is an xcodegen project
  (`client/project.yml`, the `.xcodeproj` git-ignored) with one thin
  target over the package: the entry declares the main window and
  where the registry lives, and `RootView` in `SwitchboardUI` draws
  the sessions and hosts tabs, so the iPad shell will reuse it.
  Bonjour on visionOS needs the service and the local-network reason
  in `Info.plist`. `scripts/sim.sh` builds, installs, launches and
  screenshots the app on the simulator from the terminal, and a
  launch-time `SWITCHBOARD_INVITE` pairs it with a host without
  typing. Measured: the simulator paired with a throwaway host on this
  Mac and listed its sessions under the source label.
  The Mac host's bundle (`scripts/bundle.sh`) carries the same pair, so
  the first launch of a signed bundle after that change may show the
  Local Network prompt once, with that text: at launch if the LAN
  listen setting is saved on, otherwise at the first tick of "Allow
  devices on the local network". The keys are necessary but not shown
  to be sufficient for a Developer ID build that sends raw multicast.
- **B2, card windows.** A session in the browser opens as a window of
  its own through `WindowGroup(for: CardWindowRequest.self)`: a
  request is the qualified session plus a window id, so two requests
  on one session are two windows, while opening a request again brings
  its window forward: a click on a listed session goes to the window
  it already has, and "Open another window" in its context menu makes
  a second. The window offers the modes its
  kind can take, draws `compact` and `screen`, and greys the rest with
  the note that they arrive with Milestone C; the screen mode holds a
  watch for as long as it is on screen (the model counts holders, so
  two windows on one pane send one `watch`) and the pane's text
  follows the host's `changed` events. The browser model now mirrors
  the manager's store when the manager says it changed rather than on
  a timer. A host that is not connected leaves the window on what it
  last showed with the mode row disabled; a session a connected host
  no longer lists is marked removed; closing a window releases the
  watch and nothing else. Each window's mode is kept in
  `presentation.json` in the app's container by window id, because
  visionOS restores an app's windows on relaunch and a restored card
  came up in the default mode before this. A launch-time
  `SWITCHBOARD_OPEN` opens card windows for the simulator run. Seen
  on the simulator: a line typed into the watched pane on the host
  showed in the window within the poll interval; the system restored
  the previous run's card windows on one relaunch and not on another
  forced termination, which is its call.
- **B3, browsing and persistence.** The main window lays the sessions
  out as a list, a grid, or one session in focus with previous and
  next (the cursor is kept by ref, so a session moving in the order
  stays focused), with the §8 filters as toggles over hosts, kinds and
  the states in view (an empty set admits everything, so turning one
  member off selects the rest first); working sets are grouped by
  source. The layout, the filters, the focused session, each card
  window's mode and the card windows open at the last save live in
  `presentation.json`; a launch waits a moment for the system's own
  window restoration and then reopens the saved windows it did not
  bring back, under their ids, so each keeps its mode. Seen on the
  simulator with two throwaway hosts: both hosts' sessions under
  labels told apart by the short id, a window from each host open at
  once, a screen window surviving the host's restart (the offline
  banner with the mode row disabled, then the pane refilled live once
  the watch was resent after `hello`), and a screen window reopened
  with its mode and content after the app was terminated. On the
  headset the first run showed a gap the simulator could not: a run
  that ends with only card windows is restored without the browser,
  and nothing started the connections either, since the boot lived in
  the main window's view. Every scene now boots the model (the first
  call does the work), the first card window to find the main window
  missing after the restoration grace opens it, and the card header
  carries a Sessions button that brings the main window forward (it
  is a value-keyed window group, so this never makes a second one).
  From the first hours with it: the card header is small (a short
  host pill, the Sessions button an icon) so a tall narrow card, the
  shape the conversation and document modes will want, holds
  together, and the mode pills are mini and become icons when the
  titles would wrap; the text of every mode but the card (the screen,
  the run output, the last response and the conversation, through an
  environment value the prose views size their fonts from) scales
  through a readout at the end of the mode row that opens a slider
  and snap sizes (50 to 200 percent), kept per window while it is open
  and starting over in
  a new one, since the comfortable size depends on where the window
  sits in the room. A pinch on the pane was tried first and did
  nothing on the headset, and a slider along the window's bottom edge
  was hard to take hold of next to the system's own bar, so the
  control moved up; the pinch may come back later.
  Milestone B runs on the headset, installed from the terminal with
  `client/scripts/device.sh` once the one-time Xcode setup was done
  (Developer Mode, the device paired in Xcode's Devices window, one
  run from Xcode with a development team on the target). Not in B: the
  conversation, last-response and run-output modes (Milestone C), the
  QR scanner (the invite is pasted or passed at launch), and the
  Xcode project is generated, not checked in.
- **C1 and C2, content on the wire and the conversation.** The port
  serves `session.conversation` (a page of turns from a number,
  stopping at `limit`, 100 at most, or the page's byte cap, with
  `next`; or anchored at its end with `from=last` or `before=n`,
  filling backward so the anchor is always in it, since a page of
  large turns walked forward from a count stopped short of the end and
  showed a stale tail; a page carries the first 512 bytes of each
  tool call's input and result, marked `more`, and
  `session.turn` reads one turn whole for a client that opened its
  activity, which cut a page of this project's own transcript from
  half a megabyte for six turns to a page of twenty; a client follows
  a change page after page to the end; a turn over 256 KiB loses its tool detail and
  notes first, then its prompt is shortened, then its answer is cut
  from the front, and says `truncated`), `session.last_response` (the
  last turn's answer, a projection of the conversation key at its
  revision), `session.runs` (from the record, artifacts by name) and
  `session.run_output` (the log's tail, 500 lines at most). The
  content worker reads them all: a transcript is parsed through the
  reader's new `read_masked`, which runs the mask over every text
  before the parser's caps, and the parsed, masked conversation stays
  on the worker by handle, file version and policy, so a poll is a
  stat and a page is served from memory; a run's log is read through
  `scrollback::tail_masked`, which starts the longest secret's length
  before the tail window so a value on the window's edge is masked
  whole, and is versioned by inode, change time and size. The key for
  a run is `record/n`, and the diff pass fingerprints a record's runs
  as the `runs` collection. Only a Claude Code session has a
  conversation; the rest are refused `no_conversation`. The client
  holds a conversation as pages merged by turn number (`ConversationState`):
  a `changed {conversation}` refetches from the last turn held, which
  replaces that turn and everything after, a page reaching the end
  drops anything after it, and another `resume_id` drops it all; the
  card windows draw Last response (the answer with its inline
  Markdown), Conversation (each turn's prompt, its activity folded
  under a line with the tool calls' input and result one level deeper,
  its answer; "Earlier turns" loads the page ending before the first held;
  the end follows the host) and Run output (a run picker with its
  status and the tail, scaled like the screen). Seen on the simulator
  against a throwaway host: a real transcript's last page scrolled to
  its end, and a service's run with its exit code and log. Not yet in
  C: `file.read` (no view asks for a file before Milestone D) and the
  policy reads off the frame.
- **C3, the control scope.** `POST sessions/{id}/send|resume|trust`
  for a device whose scope is `control` (a read device is refused
  `scope`, by name, with nothing run). The body carries the device's
  own operation id, `op`, and for a send the text (16 KiB at most).
  The request becomes the wire request the control socket would carry
  and goes through `serve_from` as the owner's, which is what lets a
  send answer a standing ask (Asking the owner, above); nothing new
  runs or logs: a send or a resume is written to `operations.log`
  before it runs, a repeat of the op is answered from the log without
  typing or launching again, and an op that was started and never
  answered is refused `interrupted` (409) rather than run twice. `/info` lists
  `control` among the capabilities and the send and op caps among the
  limits. The client makes a UUID per command and follows the spec's
  rule on a timeout: a status query, never a blind resend; `done`
  takes the logged reply (its failure reason included), `interrupted`
  is shown as started with the outcome unknown, `unknown` resends once
  with the same op. A `scope` refusal names the reason and keeps the
  pairing (only an unauthorized or revoked token drops it), and a
  timeout on a command does not count against the connection. The
  card window of a Claude session ends in a control bar: a message
  field with a microphone button that starts dictation at once
  through the Speech framework, on-device where the device has it,
  the words revised in place after what was typed (the keyboard's own
  mic still works; "look to dictate" is a system affordance of search
  fields only, and an app gets no gaze event, so the button is a tap),
  Send, "Trust this folder" when the session view
  carries the question, and Resume for a not-running session with a
  handle, behind "Resume spends a turn". The bar is disabled unless
  the session is live, and a read device sees why instead of the
  controls. Tests cover a read device refused, a send typed once for
  one op, a resume launching once, a running one launching nothing,
  trust answering the question once, the bad bodies, and the scope
  lowered again; on the client the op per command, the three status
  outcomes after a timeout, the one resend and the give-up, and the
  refusals.
- **C3, the draft.** An agent session's prompt as a device reads and
  works on it is its Prompt Box editor on the host, made on demand as
  the card's microphone makes one (`prompt_box::ensure_editor`).
  `GET sessions/{id}/draft` answers the editor's rendered text, the
  live utterance's range, its status and what undo, redo and the AI
  pass can do (`DraftView`); `POST` with `action` applies replace,
  undo, redo, clear, clean_up, delete_sentence or delete_paragraph,
  or `send` with the device's `op`, which takes the draft to the pane
  as the owner's, as the plain send does, and clears the editor (a
  repeat of a send that went is answered from the log; an empty draft
  is refused); `POST sessions/{id}/dictation` feeds a device's speech
  events (`started`, `voice_started`, `partial`, `final`,
  `voice_ended`, `stopped`, with the device's own recognizer session
  number) to the editor as `SpeechEventReceived`, so the device
  recognizes and the editor keeps the document. The drafts are
  fingerprinted every 150 ms while a subscriber is connected and an
  editor that moved emits `changed {draft, id}`; the device holding it
  re-reads. `/info` lists the `draft` capability. The client's
  control bar on such a host is a view of the editor: typing replaces
  the text once it pauses, dictation goes as speech events in order
  with a recognizer session per run, the tools sit above the field,
  Send goes through the draft under an op, and `changed {draft}`
  re-reads what a bar holds. The prompt window on the headset
  (`PromptWindowView`, one per app) shows the whole draft of the
  session whose input is active, the live utterance in amber, with a
  microphone, the tools and Send, the desktop preview's role for the
  Working Set. Known gaps: the AI
  pass runs in the editor's worker, pumped by the host's frame, so a
  host whose window is occluded finishes it on its next frame; the
  Mac's own voice runtime and a device dictate into the same editor
  under different session numbers, and nothing stops both at once.
- **C3, restoration.** A card window keeps its place in a
  conversation (`ConversationPlace`): the turn at the top of the view,
  or the end it follows, and the folds open per turn and per activity,
  keyed by the transcript's `resume_id`. The scroll view is positioned
  by turn number, so a page merging under the view keeps the turn at
  the top where it is, and "Earlier turns" pins the turn that was
  first; the end is followed only while the end marker is on screen,
  so reading an old turn while the agent works no longer jumps. The
  folds live in the window rather than the rows, so a turn replaced by
  a refetch, or the whole conversation dropped and re-read on a
  reconnect, comes back with the same folds open. The place is saved
  beside the window's mode in `presentation.json` (an older file reads
  with no place), so a relaunch restores it; a turn no longer held
  after a reconnect, or another transcript, falls back to the end.
- **C4, the policy off the frame and `file.read`.** The frame no
  longer resolves a redaction policy: a content job carries a
  `PolicySpec` (the project's environment, the sets it and the session
  may use, and `policy_epoch`, moved by every save and secret change)
  and the content worker reads the dotenv files and the secret store,
  caching the policy per spec for a minute, so a slow Keychain call
  costs the worker, never a frame; `Services.secrets` is an `Arc` for
  that. A store that fails refuses with `policy` (503) rather than
  answering unmasked. `GET projects/{id}/files/{path}` reads a file
  some working set pins (`PinTarget::File`), through
  `adapters::files::open_pinned`: the path checked before the disk,
  the root opened as a directory descriptor and every component
  opened from the previous one with `NOFOLLOW` (no symlink is followed,
  parent or leaf; macOS says `ENOTDIR` where Linux says `ELOOP`), the
  leaf non-blocking so a FIFO does not park the thread and checked to
  be a regular file, dotenv names refused whatever is pinned, the read
  one byte past the 768 KiB limit so growth cannot pass it, the version
  the inode, change time and size. The worker caches the masked file
  per key and re-reads only when the version or the policy moved; a
  watched file emits `changed {file, project/path}` on the poll.
  Refusals: `not_pinned` (404), `too_large` (413, with the size),
  `failed` (400) for a dotenv file, a symlink, a FIFO or a non-text
  file. The client holds files as content (`FileBody`, `watch(file:)`,
  re-read on `changed`); no window shows one yet, which the working set
  view brings. Milestone C is complete.
- **Scope on the listings.** A listing over the remote port leaves
  out the records Dispatch made for its own use (a ticket's project
  and its sessions, a worker session anywhere), told by the
  `<ticket>-<nonce>` op they were made under
  (`switchboard_control::is_dispatch_worker_op`); a supervisor's
  (`sup-…`) and the owner's own stay. `?scope=all` on `projects`,
  `sessions`, `waiting` and the `events` subscription lists and
  announces them, and a worker record is still read by its id, for
  the ticket view to come. Frames for a worker record go to `scope=all`
  subscribers alone. The control socket is untouched, so Dispatch sees
  everything as before.
- **On the headset (2026-10-14).** Scene restoration is ruled out: a
  launch opens the Sessions window and nothing else (the card and
  prompt window groups have `restorationBehavior(.disabled)`), after a
  restored window kept the process alive out of view so that an icon
  tap presented nothing. A launch log in the app's container
  (`launch.log`: launches, window appearances, scene phases) stands in
  for a device syslog the Mac cannot read. The conversation's end
  marker lives outside the lazy stack, so the follow button and
  opening at the end hold on the device. Following the end was then
  made a mode (2026-10-16): the view is held at the end with scrolling
  off, and anything growing under it pulls the end back on screen; a
  button at the right edge releases it to read, the ellipsis at the
  foot returns it. Before, following meant "the marker is on screen",
  and a new turn's row growing after the scroll landed pushed the
  marker off and turned following off by itself; the turn at the top
  was also a position bound to the scroll view, which re-applied it as
  the content grew and sent the view up. Markdown tables in an answer
  are laid out as grids. A Bonjour resolve that gets no answer is
  given up after 5 s, and each offline host resolves on its own task,
  after the retry loop hung on one resolve left waiting by the system.
- **Tickets on the headset (2026-10-17).** The port serves Dispatch's
  tickets from the app's own status poll (`src/core/tickets.rs`, spec
  §4.7): a listing by project with each ticket's standing worded as the
  Mac's page words it and its decisions; one ticket with its body,
  attempts and lanes; `decide` and `resume` under `control`, answered
  when Dispatch answers the call the Mac's action became
  (`TicketReply`, the content worker's deferred channel reused). The
  tickets are one fingerprinted collection, so a device hears one
  `changed tickets` frame per status that moved. The client lists them
  on a Tickets tab, oldest first within a project, urgent ones marked,
  with a detail view that answers decisions (the recommended option
  first, a note field, several options for a `multiple` one), resumes
  a parked ticket, and opens the project's supervisor session.
  The next day (2026-10-18) the attempts' files came: each attempt
  lists its artifacts, checks log, review rounds' files and evidence
  by a key that stands for the path, the port reads one as text
  through the Mac's own artifact read (from its cache while the ticket
  stands) or as an image from the disk, serves the ticket's events
  from the Mac's cache, and takes `close`; `info` lists `evidence` for
  it. The client's list is ordered last-moved-first with a menu for
  the order (waiting first, newest, by project), the hosts, projects
  and states (closed hidden unless asked), and a "waiting on me"
  switch; the detail shows each attempt's checks and rounds with a
  button per readable file, a Notes shortcut to the newest notes, a
  timeline, and Close. Left out for now: take, the console, the plan
  review's feedback flow; a worker session is named but not opened,
  since the plain listing hides worker records.
- **A session opened from the headset (2026-10-19).** The port takes
  `POST projects/{id}/sessions` under `control` (spec §4.5): the
  body's `op` the key, through the control socket's `session.new`,
  a Claude Code, Codex or shell session named by the device, in the
  project's root or a directory under it (`project_dir` in `app.rs`
  refuses a path that steps out or is not a directory), an agent's
  `prompt` its first turn on the command line. `info` lists `create`,
  and `/ops/{op}` names what a done creation made, so a lost reply
  still finds its record. The client offers a plus button on the
  Sessions tab and "New session in this project" on a listed session
  for a host that is connected, paired under `control` and serves
  `create`; the sheet (`NewSessionView.swift`) picks the host, the
  project by workspace, the kind, a name (defaulting to the kind's
  word and a count), a directory and the first message, and Start
  opens the session's window on the reply. Not from a device yet:
  commands and services, environment sets on the new session, a new
  project or workspace.
- **Pairing by the typed code (2026-10-14).** visionOS gives an app
  no camera, so the QR code is never scanned on the headset. A
  discovered host that is not paired offers Pair on the Servers screen:
  the client resolves its record, reads `/info` under no trust and
  shows the key the host presented in the dialog's own short form
  (`KeyFingerprint.short`, the first sixteen hex digits in fours), the
  owner compares it with the Key line on the Mac and types the
  eight-character code, and the pairing goes out pinned to that key
  (`ConnectionManager.lookup` and `pair(_:code:)`; the resolver is
  injectable for the tests). Pasting the invite's text stays as the
  fallback.
- **Known gaps at the first merge.** Milestone D, the iPad shell, is
  not started. The Codex transcript parser: a Codex session's
  conversation is refused with `400 no_conversation`, since only a
  Claude Code session with a resume handle is served. The Dispatch
  ticket view; tickets and a ticket's worker session (read by id) are
  not gated on `scope`, which filters only the projects, sessions and
  waiting lists. Removing dead sessions
  from a device. The Recent sessions set counts worker sessions. The
  screen and run-output modes follow the end without the
  conversation's end marker. The draft path makes the editor on the
  frame (`prompt_box::ensure_editor` reads the voice key there), so an
  unsigned host freezes on the Keychain prompt while a device asks for
  a draft; the signed bundle never prompts. Remote access off the LAN
  is §9 of the spec, deferred.
- **A web page on the port (2026-10-23).** The port answers `GET /`,
  `/app.js` and `/app.css` to anyone from `web/`, built in by
  `adapters/web.rs` and served on the connection thread with a strict
  content security policy, `no-store`, `nosniff`, no referrer and
  `Connection: close`, under a window of 120 a minute; nothing else
  outside `/api/v1/` changed. The invite the QR and "Copy invite" carry
  is the page's URL, `https://<address>:<port>/#code=…&key=…&id=…`
  (`core::remote::invite_url`), the code in the fragment so no request
  carries it; the address is the loopback when the port is Mac-only,
  and the dialog says so. The Swift client's `PairingInvite.parse`
  takes the URL as well as the JSON. The page reads `/info`, compares
  the key and id with the fragment's, and pairs with
  `transport: "cookie"`: the token comes back as an `HttpOnly`,
  `Secure`, `SameSite=Strict` cookie named `__Host-sb-<server_id>`,
  absent from the body, and the host accepts it beside the bearer. A
  cookie-only write or upgrade must carry `Origin: https://<Host>`
  (403 `origin`). `GET /api/v1/me` tells the page its device and
  scope. The page lists sessions waiting first, reads a Claude Code
  session's last response and any other's screen, replies under
  `control` with an op kept across a retry (a failed or unsure send
  leaves the box as it is, and the page's promise that a retry is not
  typed twice holds while the box is unchanged), polls `/waiting` every
  3 s, and treats the events socket as a hastener; a 401 is told
  apart through `/info` (same key and id: removed on the host;
  otherwise: identity changed). Spike 23 says what was measured.
  Known gaps: headless Chromium keeps the cookie from the self-signed
  IP-literal host and connects `wss://` past the `Origin` gate (the
  browser test below), but Safari is a hand check, and whether a
  browser a person drives offers a way past the certificate warning
  (today's certificate has no subject alternative names, and Chrome
  matches only those) is untested; the trusted-certificate path and a
  token-in-page fallback are the owner's decisions. The card's one-line
  excerpt is not on the wire, so the page's rows have none.
- **The conversation on the web page (2026-10-24).** A Claude Code
  session opens as its conversation rather than its last response:
  `conversation?from=last&limit=20`, then `before=<lowest held
  turn>` for "Earlier turns", merged by turn number in memory only
  (`mergeTurns`; a page that starts past the turn after the last held
  one, or ends short of the turn before the first, replaces them, and
  turns past `total` or under another `resume_id` are dropped).
  "Earlier turns" reads its lowest held turn when its turn in the
  queue comes, so a refresh queued ahead of it that replaced the turns
  leaves no gap, and a session switch hides the turns and the button
  and clears the earlier-turns error. Each turn shows its prompt, its
  steps' one-line summaries folded under "N steps" (errors marked), and
  its answer. Any other kind keeps the screen, and `400 no_conversation`
  says the host's message in the block. The page's requests go one at
  a time through one queue, the list's, `/waiting`'s and the
  conversation's alike. The open conversation is refetched when the
  session's own list entry moved (anything on it but `quiet_secs`,
  which the host's revision leaves out too, compared by `entryMark`
  against the entry the drawn read was made against) and skipped when
  only another session's did; a failed or discarded read leaves no
  mark, so the next refresh reads. A turn's boundary (the card or
  `last_stop_at_ms` changed) is read once more 3 s later, since the
  order of the transcript line and the hook is Claude Code's and the
  entry goes quiet after a Stop; a line later than that shows on the
  entry's next move or a reopen. The shell's screen is still refetched
  on every list refresh. No host code changed;
  the route-table test now fails on a path placeholder it does not
  fill. Known gaps: Safari was not driven (headless Chromium is, by
  the browser test below). A step's tool input and
  result are not shown (a page carries 512 bytes of each; reading a
  turn whole through `conversation/{n}` is a later increment). The
  next features, each its own issue, in order: Dispatch tickets and
  decisions (needs the socket frame's kind read before refetching,
  since the revision moves for tickets, a ticket fill rule in the
  route-table test, and a "no runner connected" state); a session's
  runs and their output (artifacts by name only, having no content
  route); the draft as a text area with `replace` and `send` (after
  the owner decides whether the draft read is masked or refused to
  `read` scope, since it is sent as typed today); pinned files
  (through `spaces/{space}/sets` and `sets/{id}`, as no route lists
  pins); and dictation last, behind `isSecureContext` and the
  recognizer check on each device.
- **The web page's sessions list (2026-10-25).** The list opens on
  the live sessions (liveness `running`, whatever the card says) as one
  flat list, waiting first and then by recency, each row naming its
  project. A search box and a "Filters" button sit in one row; behind
  the button are the state (live, waiting on you, ended, everything),
  the kind (the wire's five words), the projects (checkboxes, none
  meaning all) and the sort (waiting first, recent, by project, by
  name), the words following the native client's filter vocabulary.
  The button counts the choices that differ from the defaults, a count
  line says "N of M shown", and a line says how many waiting sessions
  the filters hide. An empty default list says "No live sessions; N
  not running" with a "Show everything" button, so the host's first
  moments, when every session reads not running, are never a blank
  page. Everything runs in the browser over the last list loaded
  (`arrangeSessions`), so a control never refetches. The choices,
  but not the search text, are kept in `localStorage` under
  `switchboard-list`, read through `readView`, which turns anything
  odd into the defaults. `SessionView` gains an additive
  `last_seen_ms` (the record's `last_seen`, for every liveness), so
  an ended session has an age; recency is the latest of it,
  `last_activity_ms` and `last_stop_at_ms`. Known gaps: the layout is
  checked at 390 px in headless Chromium only (the browser test
  below), not at 375 px or in Safari; the Swift client does not read
  `last_seen_ms`; the search text is not remembered.
- **The web page in a browser (2026-10-25).** `tests/remote.rs`'s
  `the_page_works_in_a_real_browser` drives the page in headless
  Chromium (Playwright, pinned under `tests/browser/`) against the test
  host, which the test pumps while `page.mjs` asks for each step: a
  browser that refuses the cookie is told so and remembers no host;
  pairing from the invite clears the fragment, keeps the host and a
  `Secure`, `HttpOnly`, `SameSite=Strict` cookie script cannot read,
  and draws the list with no horizontal scroll at 390 px; a chosen
  sort survives a reload; a conversation opens; a poll that redraws
  the list and the open session leaves the reply box's text alone;
  "Earlier turns" clicked while a list refresh is held behind a grown
  transcript ends with consecutive turns up to the last; switching to
  a shell hides the turns and the button while its screen loads, and
  an earlier-turns error does not survive a switch; another session's
  rename reads no conversation while the open one's reads it once; and
  a transcript that changes after the read a Stop caused is drawn by
  the confirming re-read;
  five tab switches leave one polling chain; a send is typed once and
  says "Sent."; and a 401 says "Identity changed" or "removed" as
  `/info` decides. It is `#[ignore]`d, so the hook does not run it;
  CI's `browser` job does, and the doc comment has the commands.
  Layout is checked at 390 px only, and Safari stays a hand check.
  The page's node tests now fail
  on CI without node, which CI installs, and print `SKIPPED:`
  elsewhere. Text typed into the reply box while a send is in flight
  stays: on success the sent text is removed as a prefix of the box
  (leading whitespace ignored), only whitespace left clears it, and a
  box edited away from the sent text is kept whole; the run checks
  that ` extra` typed mid-flight remains. `socket:` and `polls in
  9 s:` are printed as measurements rather than checks.
- **A session's runs on the web page (2026-10-27).** A command or
  service session lists its runs beneath the text block, newest first,
  each a row in the Mac's run-list words (`Run 3`, when it started, how
  long it took and `ok`, `exit 2`, `running` or `killed`) with the
  files it produced by name only. Once the pane has stopped the block
  shows the chosen run's output (`runs/{n}/output`, its last 200
  lines, masked), the newest by default and any other a tap away; a
  choice the host no longer keeps (it keeps 20 runs) falls back to the
  newest. While the pane runs the block keeps its screen and the rows
  are not buttons. The page calls `sessions/{id}/runs` and
  `sessions/{id}/runs/{n}/output`, both served before; no host code or
  wire field changed. A log that grows does not move the list's
  revision (the `Runs` fingerprint is the count, the last end and the
  last exit), so on each poll tick with the list unchanged an open
  command or service reads its runs listing again, and an output again
  while its run is open or once after it closed (`needsOutput`); the
  requests go through the page's one queue. The browser test opens a
  finished command, checks both runs' verdicts, the file name and the
  output of each, and a running command keeping its screen across a
  tick. Known gaps: two files of one name in different folders look
  alike; the list's row carries no run verdict (that needs a
  `SessionView` field); a live pane's screen is still read only when
  the list moves; a pinned artifact is served whole by the files route,
  which the page does not use; Safari at 390 px with a long output is
  unchecked.
- **A bounded request head (2026-10-25).** The request line and each
  header are read through `read_head_line`, which takes at most what
  is left of the 8 KiB head budget plus one byte, so a stranger who
  never sends a newline is refused at the bound (one `BufReader` fill
  past it at most) instead of growing a `String` until the read
  timeout; a head of exactly 8 KiB still parses. A subscriber's
  inbound WebSocket frame and message are capped at 16 KiB, past which
  the subscription ends; what the port sends is not capped. Unit tests
  count the bytes a refused head pulls from its reader, and
  `tests/remote.rs` checks a stranger streaming 64 KiB with no newline
  is closed within 2 s with no reply and a 20 KiB frame ends a paired
  device's subscription. Known gaps: a paired headset or iPad has not
  been checked against the frame cap.
- **The unauthenticated pool per source (2026-10-25).** One address
  holds at most 3 of the 4 unauthenticated slots
  (`MAX_UNAUTHENTICATED_PER_SOURCE`), counted by `Admission` on the
  peer's canonical address (an IPv4-mapped address is its IPv4 form);
  past its share or past the total a stranger is closed at accept with
  a debug log line. The slot is a drop guard (`Slot`) moved into the
  connection's thread, so it is given back when the connection ends,
  when the thread panics, and when the spawn fails. Before a
  connection is paired it runs on absolute deadlines, checked on every
  socket read beneath rustls (`Timed`): the handshake within 5 s of the
  accept, the head within 5 s of the handshake, the body within 10 s
  of the head; on promotion the deadline is dropped and the 30 s idle
  timeout set. Every reply on a connection that is not yet paired says
  `Connection: close` and the host closes after it, `/info` and
  `/pair` included, so a stranger holds a slot for one request at
  most; a paired device's own 401 or 429 keeps its connection. Unit
  tests drive `Admission` with made-up addresses (one source refused
  past its share, another admitted up to the total) and the guard
  through a drop, a promotion and a panic, and run a listener with
  300 ms deadlines against a handshake, a head and a body trickled a
  byte every 100 ms below TLS, each closed in time and its slot freed;
  `tests/remote.rs` checks the share from one address, a paired call
  beside two stalled strangers, and the close after `/info`, a 403, a
  401 and a 429 while a paired connection answers twice. Known gaps:
  strangers on several addresses can still fill the 4 (3 from one, 1
  from another), and whether a paired LAN device is shut out that way
  in practice is unchecked; a stranger's frames still queue to the
  frame before `authenticate`; a paired connection's head and body
  have no deadline; how many connections Safari and Chrome open per
  page load is unmeasured, so the share of 3 is unconfirmed on a
  device. Refusals the frame makes are logged at debug, with one info
  line a minute per cause (the refusal code, or `rate_limited` for the
  listener's 429s), and a lockout once at info. What pairing still
  lacks: the code has no check digit and no O/I/L input mapping; the
  page's rate window is per listener, not per source; a source is one
  address, never an IPv6 `/64`; and the per-source limits (6 `/pair`
  and 60 `/info` a minute) are unconfirmed against a browser on a
  device.
- **Ticket text masked (2026-10-25).** Every ticket text the port
  sends (the list, a ticket read alone, its events, a text file of an
  attempt, and the `{decision}` or `{ticket}` a command answers) goes
  to the content worker as a `ContentKey::TicketText` job with a
  `Source::Json` value, which masks every string in it before it is
  serialized, so a value holding a quote or a backslash is found as it
  is. The policy is the union of one spec per project and per stored
  set, each set resolved alone so two sets' values of one name both
  mask (every stored set, not only the ones granted now, so text a
  child printed under a grant stays masked after the grant is taken
  back or its session is gone), carried in `ContentJob.policies`; any
  that fails to resolve refuses the read `policy` (503), so a set
  whose secret the store cannot read refuses ticket text until it is
  fixed. The worker caches the masked value per key under the hash of
  its input and the policy's fingerprint. A text file over 768 KiB is
  refused `too_large` rather than masked and cut (a file between 256
  KiB and that is still cut after masking). `resolve_policy` now
  fails on a store error for a set's secret too, not only a project
  variable's, which tightens the screen read as well. A gate that ran
  under `switchboard-env exec` is marked (`GateRun.env`, sent as
  `ChecksView.env`; a runner without it, and an attempt recorded
  before it, read as `false`), and an attempt that ran so, or whose
  session holds sets now, says so to a device as `TicketAttempt.env`,
  which labels its images, since no mask reads pixels. The images
  are served: 0.7.3 withheld them, which kept every tester screenshot
  off the headset, since a try stage runs with a set, and the owner
  accepts that a grant may show in a tool result or a picture of the
  app alike. The checks log, which the runner lists among the
  artifacts too, is listed once, as `checks` or the artifact's name.
  `tests/remote.rs` checks the body, the checks log, an evidence text
  and the events with a set value named, a grant taken back, a failing
  store, a file over the cap, and an env attempt's image served with
  the attempt marked. Known gaps: the
  Mac's own ticket page and the files on disk are not masked; a value
  in an encoded or `set -x`-quoted form can pass; a gate still resolves
  the runner's grants and ignores the stage's `env` names; and whether a
  real `switchboard-env exec` gate's log carries a set value to the
  port was not run, only the fixture.
- Ticket text bounds (2026-10-26): the port cut every ticket string
  at 4 KiB, so a file or a body past it lost the rest; it now cuts per
  read: a file's `text` at 256 KiB, a ticket alone's `notes` and
  `body` at 64 KiB, an event's `text` and the list's strings at 4 KiB.
  Dispatch reads every ticket file with `take(FILE_TEXT_MAX + 1)` and
  refuses one over 768 KiB before reading it, with the size in a
  stable reason prefix; the port maps that refusal to `too_large`
  (413, naming source bytes), and the Mac's page drops the text it
  held and says `Not read:` with the size.
- Evidence image whole (2026-10-26): the image route named no long
  field, so an evidence image was cut at 4 KiB of base64 (3 KiB of
  bytes); its `data` now takes the body cap, an image up to 700 KiB is
  sent whole, and the length is checked again after the read so a file
  that grew past the limit gets the documented 413 `too_large`.
  `tests/remote.rs` reads a 200 KiB image and one at the limit whole
  and refuses one a byte over. Known gaps: the evidence path is read as
  Dispatch's status names it, with no containment check in the core; a
  real runner's image and a paired device were not run.

## Versioning and updates (2026-10-15)

The version in `Cargo.toml` is the one source, by semantic versioning,
and reaches the binary, the bundle's `Info.plist` and the host's
`/info` reply; each user-visible change is a fragment under
`changelog.d/` until `scripts/bump.sh` folds the fragments into
`CHANGELOG.md` under the next version, dated, and
`scripts/release.sh` publishes that version as a GitHub release tagged
`v<version>` with the signed bundle, its SHA-256 and the changelog
section as notes. The app updates itself from there: `core::updates`
checks thirty seconds after launch and once a day through the
`UpdateSource` port (`adapters::updates::GitHubReleases`, the fake in
tests), offers a newer release as a notice and a rail item, downloads
it on a click to a staging directory, checks the zip against its
published SHA-256 and the bundle with `codesign` against the running
app's Team ID, and on "Restart to update" arranges a helper that swaps
the bundle after the process exits, keeps the old one aside for a
failed copy, and relaunches. A pre-release or unparsable tag is never
offered; a bare binary or an unsigned bundle refuses to install.

`Settings::check_for_updates` (on by default; a `settings.json` from
before it reads as on, so it needs no schema step) gates only the
scheduled check: off, the tick at launch and the daily one do nothing,
while Check for updates in the Settings menu still looks and a release
it finds is offered, downloaded and installed as before. The schedule
counts from launch and from the last check of either kind, so turning
the setting back on checks at once unless one was answered within the
day.

The schedule is on the wall clock: `settings.json` keeps when the host
last answered, so a relaunch or a sleep does not move the next check,
and the day is stretched by up to thirty minutes derived from that
answer's second, so copies that answered at different times ask at
different minutes; the 30 s launch delay applies on every launch. The
reply's `ETag` is kept with the release it described and sent back as
`If-None-Match`, and a 304 judges that kept release, which is also
offered again after a relaunch. A host that answers with an error
(403, 429, 5xx) is asked again only after the day; a check that got no
HTTP answer at all is retried after an hour; a scheduled failure is
quiet and leaves an offer in place, while a click on Check for updates
shows its reason. The check request alone is capped at 1 MiB and
follows redirects only over `https` to `api.github.com` or
`github.com`; the download path is not pinned or capped. Known gaps:
an unauthenticated 304 may still count against GitHub's 60 an hour, so
the `ETag` is relied on only to spare the body and the parse, and no
system-wake notification is observed (a due check starts on the next
tick).

Settings' "What's new" shows the changelog. The built-in one is
`CHANGELOG.md` compiled in (`core::changelog`, `include_str!`), from the
running version down; Unreleased and any section above the running
version are left out, so nothing reads as installed that is not, and
`changelog.d/` fragments appear only once a release folds them. While a
release is on offer its notes come first, marked not installed, with its
`published_at` date; they are the release body read as groups up to the
`Built from` footer `scripts/release.sh` writes, which never shows. Only
the newest release is fetched, so the sections between the running
version and it appear in the view after the update installs them.

## Token and time accounting (2026-10-20)

`dispatch report` and the ticket page's Usage tab show every session a
ticket started with its tokens by model (input, output, cache write
with its hour-long part, cache read) and its time: span, active (inside
turns), asking (an `AskUserQuestion` waiting on its answer) and idle
(between turns). The counts roll up per attempt, per stage and per
ticket, and across tickets in `report --project`. They are read from
Claude Code's transcripts after the fact; only counts, model names and
timestamps are kept, never message content.

The record keeps what the transcripts cannot be found again without.
`Ticket.transcripts` lists, per Switchboard record id, every
`TranscriptRef` (kind, provider id, path, when first seen), a `/clear`
adding one rather than replacing the last. `Attempt.run_sessions` keeps
a plan review's reviewer and planner, which only the workflow run names,
and `Attempt.agent` the kind of operator an agent attempt launched.
`SessionView.transcript` carries the path from the resume handle. The
runner notes a ref whenever it reads a session: the stage agent's poll
(`watched_view`), every review, fixer and rewriter poll (`seen`), the
first read of a run's sessions in `poll_workflow`, and
`retire_processes`, which notes a session before judging whether it
still runs, so one that exited long ago is still recorded at park and
close. `report --refresh` asks Switchboard once for a ticket from before
these fields and adds what it finds under the lock.

Each session counts within a window: from its attempt's start (its
round's, for a round's sessions) to where the next naming of the same
record id starts. The start drops a clone's copied history, whose
timestamps come before its attempt, and splits one session that two
attempts name. A plan review attempt's own `session` is the card's face
and is left to `run_sessions`. A message is written once per content
block with the usage growing, so the last record per `message.id` is
the message; `<synthetic>` is skipped; Agent-tool subagents' files under
`<transcript minus .jsonl>/subagents/` fold into their parent's tokens,
not its time. A session's kind is the one recorded beside its id, else
any ref's, else its operator in the ticket's pipeline copy, marked as
such. Codex and command sessions are not counted and show their round's
or attempt's span; a Claude session with no transcript read is "not
found", and one whose kind nothing says is "kind unknown", never zero.

The rows are cached in `<data>/tickets/<id>/usage.json`
(`store::USAGE_VERSION`, refused when newer), one entry per file with
its size, mtime and `read_ms`, so the counts outlive Claude Code's
deletion of transcripts after `cleanupPeriodDays`; a vanished file's
entry is kept and marked `gone`. A changed file is read again. Files
are read with the writer lock not held, and the lock is taken only to
merge by `read_ms` and write, so two writers keep each other's newer
entries. The close reads the counts once its processes are retired,
letting the runner's held lock go around the read as the tree removal
does. The port answers `usage` from the cache brought up to date, never
asking Switchboard.

Known gaps: a permission prompt's wait counts as active. A running
agent's counts on the tab are as of the ticket's last change. Codex is
not counted. A cold Usage read of a large open ticket holds the port's
handler for its duration. A ticket from before this change takes a
session's kind from its pipeline copy, which a restart may have
replaced. Prices are an estimate from a default table in the binary
under the project's live `[policy] rates` (five fields per model,
cache included), recomputed at report time; nothing is stored. The
estimate prices every row at standard speed, since the `speed` value
for fast mode is not known, and Haiku 5.5 at its up-to-100k tier,
since per-model totals cannot be split by request; both need pricing
per cache row. `claude-fable-5-1` has no default row and shows as
unpriced. No budget is wired. The session page counts each `message.id`
once and skips `<synthetic>`, as the tab does, but it reads only the
main transcript and counts an assistant record with no `message.id`
once, which the tab drops. The two agree only for a session with no
subagent files whose records all carry an id; a session whose
Agent-tool subagents wrote their own files shows fewer tokens on the
session page.

## Lost review prompts (2026-10-24)

A Claude Code first-run dialog (auto mode's environment setup, at
least) can take a review round's prompt: the reviewer sits idle with no
turn, the round file never comes, no Stop fires, and the run waited
forever. No launch flag that keeps the dialog out is confirmed (none
was looked for beyond the obvious), so the fix is on the detection side
(see Signals under Workflows). Each session keeps `last_prompt_at`, stamped by
`PromptSubmitted` only; a waiting run whose Claude Code
agent has none after the ask fails after `LOST_PROMPT_AFTER`, which
Dispatch reads as a failed review and offers to rerun. Continue
re-sends the ask once when no turn began. Nothing re-sends on its own,
since a re-send into a pane still at the dialog is lost the same way.

A pane this process spawned must report an event later than its spawn
before the check may blame it (`AppCore::spawned_at`, transient): the
record's `last_event_at` is the previous process's until then, so a
reviewer relaunched on Continue and still starting up used to fail the
round as "never started a turn" once the bound passed. Continue sends
nothing into such a pane either; once it reports and has still taken
no prompt, Continue re-sends as before. A restart loses the spawn time,
so an adopted pane keeps the plain "has reported" rule.

Known gaps: Codex agents report no hooks and keep the stall notice
only, which includes the built-in definition's reviewer. Whether the
dialog or its Esc writes any hook line is not settled; if a lost prompt
turns out to fire `UserPromptSubmit`, the check needs a turn count
rather than a timestamp. No event carries a process identity, so a
late event from the old process stamped after the spawn counts as the
new one's (Continue relaunches only a gone pane, whose `SessionEnd` has
already fired). Between the new process's `SessionStart` and its
`UserPromptSubmit`, milliseconds apart, the prompt reads as not yet
taken, the same window the `started` mark already has. A spawned pane
that never reports any event is no longer blamed by this check.
`PromptSubmitted` can be lost if Claude Code cancels the hook helper at
its 5 s timeout (whether the log line was already appended is not
documented), and a turn that then uses no tool for `LOST_PROMPT_AFTER`
fails as unprompted; any tool use or Stop covers it.

The agent's own standing ask no longer excuses it from the check: a
workflow's round prompt leaves the ask standing (see Asking the owner),
and a prompt that was taken stamps `last_prompt_at` whether or not it
answered, so only a prompt that was lost fails the run.

## A human gate's own question (2026-10-25)

A human gate's own `question` used to replace the default question, so
a custom-question gate lost the branch, diff, `Tree:`, notes and
`Deployed:`/`Served:` lines, and naming `{inputs.notes}` brought back
only the path, not the first line. The custom text now follows the
default after a blank line. The supervisor seed's "shown in the
question" is true for every human gate as a result. A `question` on the
built-in `lanes` gate, which was silently ignored, is refused at load.
The default's `git.head` and `git.summary` reads no longer fail the
gate attempt: the line is left out and the error logged.

Known gaps: a question that names `{inputs.notes}` (the shipped
`decisions` gate does) shows the notes path twice. There is no opt-out
key for a gate that wants only its own text; `Gate::Human` does not deny
unknown fields, so a misspelt key would pass silently, and the key
waits for a gate that needs it. A decision pending from before the
change keeps its stored question until `dispatch restart` asks again.

## A plan review rerun keeps the owner's revise (2026-10-25)

A plan review whose run failed after the owner answered `revise` (the
app answering `no such run`, say) was rerun from the plan stage's
original, so the owner's objection and the planner's edits for it were
lost. `Ticket::revised_copy` now says when a failed or cancelled review
attempt holds the owner's revision: a `revise` answered about it with
its `Revision` and round file in place, or a rerun that carried one
with its round 1 in place. The rerun copies that attempt's copy, writes
the revise text (and any `rerun` note) as its `plan.feedback-1.md`, and
sends it as `workflow.start`'s `owner_round`. The app opens such a run
with the owner's round: the reviewer launches idle, the planner answers
first, and the reviewer's first prompt is `review_first` at round 2.
The rerun clones the failed run's planner when the app still has its
transcript, else the plan stage's session. The rerun question, `show`
(`plan_revised`) and the Plan tab name the copy. No record version
changes: `revisions` and `carried_from` carry it.

A run leaves the app's memory only by `remove_workflow`,
`remove_project`, or a load of an older or unreadable workspace file;
the other cause of `no such run` is a different instance or data
directory answering. The `workflow` query holds no state of its own, so
nothing retries or reattaches. Instead the runner asks `find` for the
operation that started the run and fails the attempt saying which: the
app logged the start and no longer has the run (dropped), the app that
answered has no record of the start (another instance), or the log
could not say (a failed `find`, no start operation in the ledger, or
the run listed as present). A live reviewer or planner is left as it
was, and the reason lists the sessions recorded for the run without
checking that their panes exist.

Known gaps: only the last revise is carried (earlier ones are already
in the copy if their edits landed). A failed run's half-finished edits
are kept with the copy, the cost of reviewing the revised copy at all.
A `Starting` or `AwaitingFeedback` run's state after an app restart is
not traced, and the workspace `.bak` is a possible, untried source for a
lost run, left out.

## Poisoned locks recovered (2026-10-26)

The frame's locks are taken with `sync::lock_recover`, which takes a
poisoned mutex back (`PoisonError::into_inner`), clears the poison and
logs the site once: the Prompt Box outbox and saved paths, on both the
editor side and the take in `draw`. A panic on the UI thread is not
caught (eframe 0.36.1 has no `catch_unwind`; winit 0.30.13 stops the
app and re-raises it), so a poisoned `expect` there ended the app. In
this tree nothing can poison those two locks: every holder runs on the
UI thread and does one `push` or one `mem::take`. The change hardens
the sites a future worker would hit rather than fixing a reproduced
crash; the original report had no backtrace.

The content worker's locks (the poll interval and the watch set) go
through the same helper, so a poisoned one no longer kills the worker
or silently drops `set_watch` and `set_poll`. A read that panics is
caught per job like the diff read: it answers `failed` ("the read
panicked") through the usual refusal, stamps no revision, starts the
worker's cache over, and the worker reads on.

Known gaps, left out:

- The tmux Enter thread (`adapters/tmux.rs`): one detached thread per
  send sleeps 150 ms and writes the Enter. Two single-line sends inside
  150 ms should, from the code, type A, B and then two Enters, so the
  pane submits `AB` once plus a stray Enter; quitting inside the 150 ms
  drops the Enter. Neither has been run. Queueing the Enter means
  joining the text write to it, which changes the `write_line` failure
  contract the send relies on to hand the text back.
- The PDF preview (`ui/document.rs`): a `qlmanage` thread that panics or
  hangs leaves the preview `Pending`, a spinner forever, since the poll
  never checks for a disconnected channel. A stuck spinner, not a crash.
- The `promptbox` crate has its own poisonable `expect`s (the mic mutex
  read in `pump_live`, the mic error mutex written from the audio error
  callback); whether cpal calls that callback on its own thread was not
  checked. A fix is upstream plus a rev bump.

## Window frames fit the surface limit (2026-10-26)

A frame saved on a larger display setup asked for a window bigger than
the GPU makes a surface (and, maximized, a size the app could not
bound); wgpu's surface error had no handler, so its default panic ran
inside an `AppKit` callback and aborted the app at launch.
`core::window_fit` fits a frame to its display (the one it names, else
the one holding its centre, else the first): the size to `8192 / scale`
points a side and to the visible frame less a title bar (`TITLE_BAR`, 32
points, measured on macOS 26), the origin inside the visible frame. The
displays and their backing scales come from `adapters::displays`
(promptbox's screen list with `NSScreen.backingScaleFactor`); with no
display listed the fit is scale 1 and the 8192 cap alone. The fit runs
wherever a stored frame is read: `main.rs` before eframe starts,
`popout::opening` (pop-outs and the Dispatch window), `hold_at_launch`,
and the preview panel's restore. Each window open reads the display list
afresh, so a display attached during the run is known; only the launch
hold keeps one read in `UiState` for its per-frame check. A stored
oversize frame stays on disk until the window settles at its fitted frame and the settle path saves
that. The first launch with nothing saved opens at the primary display's
visible frame, position and size explicit, in place of `with_maximized`.
The wgpu device gets an uncaptured-error handler in the app creator, so
a later over-limit configure is logged. No record or schema change.

Known gaps: a live window dragged or resized past the limit logs the
error, and whether it then keeps drawing its previous surface or goes
blank was not established. The launch fit uses the displays listed when
`main` runs; a display that attaches later in startup (Sidecar) is not
seen. The ceiling is a fixed 8192 device pixels, so a frame of 4097 to
8192 points on a 2x display opens at 4096 although the GPU could draw
more. `TITLE_BAR` was measured on one macOS release; an older one with a
shorter title bar opens the first window a few points short of the
visible frame.

## CI on macOS (2026-10-27)

CI only ever ran on Linux, so nothing under `cfg(target_os = "macos")`
was compiled before a merge: the Keychain, Bonjour and Dock adapters,
`dispatch`'s macOS-only git tests and sandbox test, and the vendored
terminal widget's macOS key bindings. A `macos` job on `macos-latest`
(arm64, Homebrew under `/opt/homebrew`) now runs the pre-commit hook's
four commands: `cargo fmt --all --check`, Clippy over the workspace with
`--all-targets`, `cargo test --locked --workspace --no-fail-fast` (so one failing binary
does not hide the rest) and `cargo test --locked -p egui_term --lib`. It installs tmux with `brew`,
so the tmux integration tests run rather than skip, and node, which the
web page's tests need. Unlike Linux's `scripts/ci-test.sh` it has no
retry of a flaky test; a flake there is red.

A `deny` job runs `cargo deny` with `deny.toml`, as two matrix entries:
`advisories`, and `bans licenses sources`. Licences are an allow list of
what the lockfile uses today; git sources are limited to promptbox.
Advisories were red on the lockfile when the job was added; the three
TLS crates with fixes (`rustls`, `rustls-webpki`, `webpki-roots`) were
updated, and four unmaintained crates with no safe upgrade, all pulled
in by the GUI stack (`ttf-parser`, `zbus_names`, `zvariant`,
`zvariant_derive`), are ignored by id with a reason. Advisories are a
separate check because a new one turns it red with no code change; it
gates pull requests for now (the checks are named `deny (advisories)`
and `deny (bans licenses sources)`; those are the names branch
protection must require, along with `check`, `msrv` and `macos`), and making it non-blocking is a one-line
`continue-on-error` on that matrix entry. `cargo deny check` (0.20.2)
passes on this lockfile locally: advisories, bans, licenses and sources
all ok, with warnings only for a yanked `clipboard-win` and duplicate
versions.

The workflow reads the repository only (`permissions: contents: read`),
runs one at a time per branch or pull request (a newer push to a pull
request cancels the older run; pushes to `main` all finish), skips a
push to `main` that touches only Markdown or `docs/` (pull requests are
never filtered, since a skipped workflow leaves a required check
pending), and pins every action to a commit with its version in a
comment. `.github/dependabot.yml` proposes weekly updates for the cargo
lockfile (minor and patch grouped into one pull request), the actions,
and `tests/browser`'s npm lockfile. No record or schema change.

The first runs settled what was open: both Keychain round-trip tests
pass in a headless runner session, the tmux adapter tests run for real
once brew has installed tmux, and the job takes about 5 minutes on a
warm cache against its 45-minute bound. Known gaps: `macos-latest` is
moving from macOS 15 to 26, and `macos-15` is the label to pin if the
move breaks the build. Dependabot's first pass proposed no `rev` bump
for the promptbox git dependency, so whether it ever will is still
unknown. Nothing builds or signs the bundle in CI, the Swift client
stays outside it, and which checks `main` requires is the owner's
branch-protection setting.

The `browser` job runs the web page's `#[ignore]`d browser test on
`ubuntu-latest`: `ci-apt.sh`, the stable toolchain, node 22, `npm ci`
in `tests/browser`, `npx playwright install --with-deps chromium` with
`~/.cache/ms-playwright` cached on the npm lockfile, then `cargo test
--locked --test remote -- --ignored the_page_works_in_a_real_browser
--nocapture`. Naming the test keeps `tests/live.rs` and the gate's
agent items out. It has no flaky-test retry. Playwright advises
against caching browsers (the restore costs about what the download
does), so the cache is an optimisation the job passes without. It is
advisory: branch protection gets `browser` only after it has passed on
`main`. Its first run on `main` was green: about 11 minutes, nearly all
the Chromium download before the cache was saved, with the test itself
24 seconds against its 120 s deadline; node 22 suffices, and Linux
Chromium keeps the `__Host-` cookie and connects `wss://` to the
self-signed IP host as the macOS build does. Known gaps: whether
`--with-deps` is needed on the runner image, and the job's time once
the cache restores.

The runs after the merge answered the time question: on a warm cache
`check` takes 4 minutes, `msrv` 3.5, `browser` 2 and `macos` 5 to 6,
and the macOS runner's queue alone has reached 11 minutes. Two things
made cold runs of 14 to 20 minutes: a stable point release
(1.99.0 to 1.99.1) changed every cache key at once, and the
repository's caches, about a gigabyte per job per branch, passed
GitHub's 10 GB cap, after which `main`'s own entries were evicted by
branches' and jobs on `main` rebuilt from nothing. So `rust-cache`
saves only on `main` (`save-if`); a pull request restores `main`'s
entry, keyed on the toolchain and the lockfile, and saves nothing;
`browser` restores `check`'s entry (`shared-key`) rather than keeping
a near copy. The `macos` job moved to its own workflow,
`.github/workflows/macos.yml`: every non-docs push to `main`, pull
requests that touch the macOS-only files (the adapters, `main.rs`, the
dispatch sandbox and its git tests, `tests/gate.rs`, the vendored
terminal widget), the Cargo files or the toolchain file, and
`workflow_dispatch` for a branch the owner wants checked. It is
advisory: a required check that a path filter skips leaves the pull
request pending, so branch protection lists `check`, `msrv` and the
two `deny` entries, not `macos`.

## egui 0.37 (2026-10-30)

`eframe`, `egui`, `egui_extras` and `egui_kittest` moved to 0.37,
`egui_commonmark` to 0.26 and the vendored `egui_term` with them, in one
step: their types cross each other's APIs, so a single crate left on 0.36
puts two `egui`s in the graph and nothing type-checks. promptbox crosses
them too (`Context`, `Ui`, `Id`, `Rect`), so it moves in the same step.
`eframe::App` is unchanged. The changes the move did need:

- `Id::new` and `Ui::id` are deprecated (`Id::unique`, `Ui::scope_id`),
  `WidgetType` is `egui::Role`; all call sites migrated rather than
  allowed. `Id::new` forwards to `Id::unique`, so the ids keep their values.
- egui 0.37 has no default features, and its default fonts no longer
  carry the emoji fonts `theme::fonts()` falls back on, so `egui` takes
  `monochrome_emoji_fonts` (see the comment in `Cargo.toml`). kittest
  0.37 panics on a missing glyph, which is how the loss showed.
- A panel's stored id is now salted by its parent `Ui`, so a test drags
  the side's resize handle rather than writing its `PanelState`; a
  submenu button's accessible label no longer carries its `⏵`.
- The standalone `egui_term` test build passes without `default_fonts`.

Before and after screenshots of the terminal, a rendered document, the
code view and the Dispatch page, both themes, differ in three ways:
`egui_extras`' syntax highlighting draws function names yellow rather
than teal (the code view and a document's code blocks), a document's
code block has more padding, and a text field's hint and text sit
further in. Glyphs and the rest of the layout are unchanged, with
eframe's `system_fonts` default kept.

Known gaps: whether eframe 0.37 still runs only `App::logic` while
the window is hidden (spike 12, `src/cadence.rs`) is not re-measured;
the signed bundle's grants are the owner's to check. No record or
schema change.

## Open questions

- Shared project config runs with a hash-and-approve flow and no
  sandbox. Should approved commands be restricted to the variables they
  list instead of receiving the whole project environment?
- Notes editor in scope, or "open in editor"? Leaning open in editor.
- Multiple machines: sync the project list early? Leaning no.
