# Feedback from daily use (September 2026)

Collected after a few days of using Switchboard for full-time
development. Nothing here is built yet; each item is a candidate for a
future milestone. Items are numbered so plans can refer to them.

## What works

The core loop is good: launch a Claude Code session, talk to it through
the message box, read the consolidated summary view. The summary view
keeps the whole conversation visible with tool calls collapsed, and any
tool call can still be expanded. That is the view the user wants to live
in, so the theme of most items below is "the summary view must carry
everything the raw terminal carries that matters".

## A. Session control

- **A1. Interrupt from the message box.** In the raw terminal, Escape
  interrupts the agent. From the message box there is no equivalent, so
  stopping the agent means finding Open Terminal and pressing Escape.
  Wanted: an interrupt that works without leaving the summary view
  (a button and a keyboard shortcut, delivered as Escape to the pane).

## B. Layout and sizing

- **B1. Summary view wrapping ignores the sidebar.** Text wraps at the
  width the pane had when rendered. When the sidebar expands, the pane
  narrows and the end of each line is cut off with no horizontal scroll
  and no re-wrap. Wanted: re-wrap on width change (or, at minimum,
  horizontal scroll).
- **B2. Embedded terminal is wider than the window.** Even with the
  window maximized, the tmux pane ends up wider than the visible area,
  hiding the right edge of terminal content. Wanted: pane columns and
  rows derived from the actual visible widget size, re-sent on resize.
- **B3. Collapsing the raw terminal is awkward.** After expanding it,
  the control to collapse it is back at the top, so collapsing means
  scrolling all the way up. Wanted: a collapse control reachable from
  wherever the user is (sticky header, floating button, or a shortcut).

## C. Visibility of agent state

- **C1. Prompts needing a reply are easy to miss.** At least once the
  agent was doing something that blocked input (exploring the project)
  and the message saying so only appeared in the raw terminal. The
  summary view gave no sign the session was blocked or asking for
  something. Wanted: surface "waiting on you" and "busy, cannot accept
  input" states in the summary view and the project list, including
  states that do not arrive through hooks.
- **C2. Context size is not visible.** Wanted: the session's context
  usage shown in the conversation view, without opening the terminal.

## D. Commands and services

Commands and services are not agent sessions and should not be treated
like them. Commands are fire-and-forget with a result to read; services
are toggled on and off with logs to watch. Both belong in an
always-visible panel.

- **D1. Commands panel.** A pane on the right-hand side listing every
  command for the project (for example Rebundle here). Rerunning one is
  a single click. The output of the last run is preserved and easy to
  see, not lost when the run finishes or buried behind a click.
- **D2. Services panel.** Several services per project visible at once
  (the npm server here; several servers in Everworld), each with quick
  start and stop and easy access to its log output.
- **D3. Definition by file, discovered by the app.** The user does not
  want to configure services and commands through Switchboard's UI.
  Instead an agent working in the project writes a well-formatted
  definition file (JSON) declaring them, and Switchboard discovers it
  and populates the panel. The agent is the way a project gets
  configured; the file is the record.
- **D4. Approval of discovered definitions.** This deliberately crosses
  the trust boundary in `design.md` ("nothing in a project directory is
  parsed as config"), so the rule becomes:
  - Every entry discovered from a project file starts unapproved and
    cannot run until the user approves it.
  - Approval is per entry, and the user sees exactly what they approve:
    command line, working directory, requested environment.
  - If a definition changes after approval it drops back to unapproved
    and must be re-approved. A silently edited command never runs on
    an old approval.
  - Approvals live in Switchboard's data directory, keyed to the
    content of each definition. The project file itself stays untrusted.

## Added during planning (2026-09-08)

- **A2. Arrow keys in the embedded terminal.** When Claude Code shows a
  picker (AskUserQuestion, plan approval), the arrow keys do not reliably
  reach it through the embedded terminal, so options cannot be selected
  there. Needs investigation in the vendored terminal widget's input
  path; not planned yet.
- **C3. Questions are invisible in the summary view.** An
  AskUserQuestion prompt shows nowhere in the summary view; the only
  sign is the raw terminal. Covered by C1: it arrives as a permission
  request for that tool, so the state can read "waiting on you:
  question". Showing the question text and the choices in the summary
  view, and answering from there, is a follow-up.
