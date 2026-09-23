# Spike 09: a session in a window of its own

Question: can a session page, with its embedded terminal and the side
panel, be drawn in a second native window from the same frame, and does
the window come back where it was?

Mechanism: `Context::show_viewport_immediate` (egui 0.36), the same
call Prompt Box uses for its caption and preview overlays, with the
session page drawn into a `CentralPanel` inside the callback. The
callback runs inside the main window's frame, so it borrows the same
`DrawCtx` and dispatches the same actions.

Verified on 2026-09-23 with the debug build, driven by
`SWITCHBOARD_SCRIPT` and screenshotted with `screencapture -l`:

```
add-project demo <dir>
new-shell demo sh1
show-board demo
files on
pop-out sh1
```

- Two windows for the process (the `winid` helper listed both). The
  second is titled `sh1 · Switchboard`, shows the session header with
  Close window in place of Back, the run bar, the embedded terminal,
  and the side panel with its own Files, Run, and Notes tabs.
- A line sent to the pane with `tmux -L switchboard-test-shot
  send-keys` appeared in the pop-out's terminal within a frame.
- The main window stayed on the board; the session's card was still
  drawn there (cards read the pane snapshot, they attach nothing).
- `settings.json` held the window's frame after it settled:
  `{"session": "...", "frame": {"x": 198, "y": 51, "w": 1100, "h": 780}}`,
  which is what the window is opened with on the next launch.
- Headless (`egui_kittest`) embeds the viewport as a `Window` inside
  the main one, so the UI test finds the page's labels there.

Not verified by script, because there is no keystroke injection:
typing into the pop-out's terminal with the window focused, and Cmd+W
closing it. Both use the ordinary focus and input paths that the main
window's page already relies on.
