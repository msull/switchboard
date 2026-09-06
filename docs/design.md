# Switchboard

## Status

Design draft, 2026-09-05, revised the same day to put persistence at the
center. Written before any code beyond the scaffold. Expect the terminal
section to change after the spike.

## The problem

Cmux (a macOS multiplexer built on embedded Ghostty, aimed at agent coding
sessions) does most of what is needed: workspaces per project, several
agent sessions side by side, a terminal that feels native. It has two
gaps, and the first is the reason Switchboard exists:

1. **Workspaces do not survive a restart.** A good workspace takes real
   effort to set up: named agent sessions mid-task, a dev server, a few
   shells in the right directories. Those get revisited weeks or a month
   later. A reboot, an app update, or a crash loses all of it.
2. **No file management.** Finding, previewing, and opening the files a
   session is working on means leaving for Finder or an editor.

Switchboard is that tool with persistence as the first design constraint
and a file browser beside the sessions.

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

- `name`, `kind` (agent, command, service, shell), `cwd`, `command line`,
  `env profile`, `created`, `last seen`, `notes`;
- `resume` (kind-specific: agent session id, or nothing);
- `layout` hints (card position and grouping on the board) so the view
  comes back as it was;
- `scrollback` path.

Also holds the project's pinned documents. Written on every change, never
only on quit. A crash loses at most the last few seconds.

### Session kinds

- **Agent sessions** are the reason for the app. Naming is required at
  launch. On launch, Switchboard captures the agent's resume handle;
  "return to it" runs the agent's resume command in the recorded
  directory. If the agent cannot resume, it still comes back with its
  scrollback, notes, and directory, and a fresh agent can be started with
  the notes as context.
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

## Architecture

Follows the template layering. Nothing below touches egui.

- `core`: project registry, workspace records, env resolution, and the
  state machine for launching, watching, reattaching, and retiring
  sessions. Actions in, effects out, clock injected. The rehydration
  logic (record in, list of launch effects out) is pure and fully unit
  tested.
- `ports`: `Store` (records), `FileSystem` (list, read, watch), `Git`
  (status, branch, repo discovery), `ProcessHost` (spawn with env and
  cwd, attach, signal, stream output), `Terminal` (whatever the spike
  chooses), `Opener` (default app, editor, Finder), `SecretStore`.
- `adapters`: real implementations, each with a fake. The fake process
  host plays scripted output so UI tests never spawn anything.
- `app`: runs effects, drains worker channels, feeds results back.
- `ui`: workspace switcher, board of cards, session view, file panel.

## Milestones

0. **Resumability spike.** Prove agent resume and choose the process
   host and terminal view.
1. **Workspace records.** Projects, sessions as records on a board of
   cards, launch and return-to for agents via hand-off to a real
   terminal, session state from the spike's signal, and the cross-project
   switchboard view built from the same records. Restart the app,
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
- Should `.switchboard/` be committed by default? Sessions are personal;
  saved commands are arguably shared.
- Notes editor in scope, or "open in editor"? Leaning open in editor.
- Multiple machines: sync the project list early? Leaning no.
