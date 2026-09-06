# Switchboard

## Status

Initial design draft, 2026-09-05. Written before any code beyond the
scaffold. Expect the terminal section to change after the spike.

## Product concept

Switchboard is a desktop app for the person who runs many things at once
with coding agents: several clients, several repos, and a few non-code
responsibilities that still benefit from notes, tooling, and an agent.

Everything hangs off a **project**: a root directory on disk, almost always
with a git repo somewhere inside it. Within a project you:

- see the folder structure and get to any file fast, with a preview
  (Markdown rendered) and a one-keystroke jump to the default program;
- launch coding agents, several at a time, give each session a name, and
  come back to it later;
- run saved commands and long-running services (a dev server, a watcher)
  in a terminal-like pane;
- keep the project's environment variables straight.

Concrete projects: one per client; one each for Prompt Box and
Switchboard; one for the PTA role; one for Cub Scouts. Some projects *are*
the repo (Prompt Box). Others are a workspace folder holding notes,
tooling, and metadata, with the real work in repos one level down.
Switchboard must be comfortable with both.

## Priorities, in order

1. **Never lose a session.** A named agent session, a running service, or
   an unsaved note must survive Switchboard quitting or crashing, or at
   least be resumable with one click.
2. **Fast to the file.** Open a project, find a file, read it, open it
   elsewhere: seconds, keyboard-driven.
3. **One place for the running things.** What is running, in which
   project, since when, is it healthy. No hunting through terminal tabs.
4. **Stay out of the way.** Switchboard organizes; it does not wrap or
   reinvent the agents, editors, or shells. If the native tool is better
   at something, hand off to it.
5. **Portable core.** The project model, session registry, and command
   runner do not depend on egui, so a CLI or a different front end can
   reuse them.

## Concepts

### Project

- `name`, `root` (absolute path), optional `notes` (Markdown, stored in
  the project), optional tags for grouping (client, personal, code).
- **Layout kinds**, detected rather than declared: `root` is itself a git
  repo, or `root` contains one or more repos in subdirectories. Both are
  shown the same way; the git decorations attach to whichever directories
  are repos.
- Project-local metadata lives in `<root>/.switchboard/` (sessions,
  commands, env settings) so it travels with the folder and can be
  git-ignored or committed as the user prefers. The global list of
  projects lives in the platform data directory.

### File browser and preview

- Tree of the root, lazily loaded, with a fuzzy file finder (type to
  filter across the whole project, honoring `.gitignore`).
- Preview pane for the selected file: rendered Markdown, syntax-highlighted
  text, images, and a size-capped hex/plain fallback. Read-only.
- Actions: open in default app, open in editor, reveal in Finder, copy
  path. Directories: open in terminal (a new session in that directory).
- Git decorations on tree rows: modified, untracked, branch name on repo
  roots. Cheap polling, not a full git client.

### Session

A session is anything with a terminal attached, tracked by the project.

- Fields: `name`, `kind` (agent, command, service, shell), `cwd`,
  `command line`, `env profile`, `created`, `last seen`, free-form `notes`.
- **Agent sessions** are the reason for the app. Naming is required at
  launch ("dock badge", "refactor persistence"). Where the agent supports
  it, record its own resume handle (Claude Code's session id, for example)
  so "return to it" works even after the process is gone.
- **Services** are long-running commands with a start/stop button, a
  health line (running since, exit code if it died), and a log tail. Think
  `npm start`.
- **Commands** are one-shot, saved per project with a name, run in a
  terminal pane, exit code recorded.
- Every session's transcript is kept on disk (a scrollback file) so
  reading what happened does not depend on the process still existing.

### Environment

- Each project has zero or more **env profiles**: named sets of variables,
  layered: global secrets (stored in the macOS Keychain, referenced by
  name) < project `.env` files < profile overrides.
- Sessions pick a profile at launch; the resolved environment is shown
  with secret values masked.
- Helpers: diff `.env` against `.env.example`, flag variables a saved
  command references but the profile does not define, never write secrets
  to disk in plaintext outside the `.env` files the user already owns.

## The terminal question (Spike 0)

Everything session-shaped needs a terminal. The choice decides the app's
feel, so it gets a spike before Milestone 1. Options, roughly in order of
how much Switchboard has to build:

1. **Hand off entirely.** Switchboard launches a named tmux session in
   Ghostty (or the user's terminal) and tracks it. Persistence comes free
   from the tmux server surviving app restarts. Switchboard shows only the
   metadata and a "jump to" button. Cheapest, and consistent with
   priority 4, but the app is then a launcher, and reading a session means
   leaving the app.
2. **tmux as backend, Switchboard as renderer.** Use tmux control mode
   (`tmux -CC`, what iTerm2 does) to own the processes and their
   persistence, and render panes inside egui. Requires a terminal renderer
   in egui but no PTY management of our own, and sessions outlive the app.
3. **Own the PTYs.** `portable-pty` plus `alacritty_terminal` for the grid
   and a custom egui view (evaluate the `egui_term` crate before writing
   one). Full control and no tmux dependency, but sessions die with the
   app unless we add our own daemon, which is exactly what tmux is.

The spike measures, for options 2 and 3: does an egui-rendered terminal
handle a full-screen TUI (a coding agent, `vim`, `htop`) with acceptable
latency and correct colors, and how much code that takes. It also checks
whether Ghostty can be embedded (libghostty) or scripted well enough to
make option 1 feel integrated. Deliverable: a `spikes/terminal` crate with
a README of findings and a recommendation.

Likely outcome: start with option 1 for agents (the agent's own TUI is
best in a real terminal) and option 2 or 3 for commands and services
(where a log-like pane is enough), then revisit.

## Architecture

Follows the template layering. Nothing below touches egui.

- `core`: project registry, session registry, env resolution, the state
  machine for launching, watching, and retiring sessions. Actions in,
  effects out, clock injected.
- `ports`: `FileSystem` (list, read, watch), `Git` (status, branch, repo
  discovery), `ProcessRunner` (spawn with env and cwd, stream output,
  signal), `Terminal` (whatever the spike chooses), `Opener` (default app,
  editor, Finder), `SecretStore` (Keychain), `Store` (project and session
  metadata).
- `adapters`: real implementations, each with a fake. The fake process
  runner plays back scripted output so UI tests never spawn anything.
- `app`: runs effects, drains worker channels, feeds results back.
- `ui`: three-column layout: projects, tree and finder, detail (preview or
  session pane). Sessions list per project with status dots.

## Milestones

0. **Terminal spike.** Answer the question above; pick an approach.
1. **Projects and files.** Add and switch projects, tree, fuzzy finder,
   preview (Markdown, text, images), open in default app and editor,
   reveal in Finder. Persisted project list. Usable on its own.
2. **Commands and services.** Saved commands, run in a pane, exit codes.
   Services with start/stop and health. Scrollback on disk.
3. **Agent sessions.** Named launches, session list, return-to, resume
   handles for Claude Code. Notes per session.
4. **Environment.** Profiles, Keychain-backed secrets, masked view,
   `.env.example` diff, missing-variable warnings.
5. **Git awareness and polish.** Decorations, sub-repo discovery, global
   quick-switcher across projects, dock badge with running-service count.

## Open questions

- How much of a session's state is worth persisting beyond "how to resume
  it"? Full scrollback for every command may be more disk than value.
- Should `.switchboard/` be committed by default? Sessions are personal;
  saved commands are arguably shared.
- Is a Markdown *editor* for project notes in scope, or is "open in
  editor" enough? Leaning enough, per priority 4.
- Multiple machines: is a project list sync (iCloud Drive, a dotfiles
  repo) needed early? Leaning no.
