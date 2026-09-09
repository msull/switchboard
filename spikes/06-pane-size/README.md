# Spike 06: pane size follows the attached client

**Question.** The embedded terminal showed a 200-column pane through a
narrower widget, hiding the right edge even with the window maximized.
Does tmux `window-size latest` make the window follow the size the
attached client reports, and what happens to a detached window?

**Setup.** The private server config had `set -g window-size manual`, and
every session is created with `new-session -d -x 200 -y 50`. `egui_term`
already computes columns and rows from the widget size and pushes them
into its pty (`vendor/egui_term/src/backend/mod.rs`, `resize`), which
reaches the attached tmux client as a window-size change. Under `manual`
tmux ignores the client.

**Measured** (tmux 3.7c, macOS, `window_follows_the_attached_client_size`
in `src/adapters/tmux.rs`; a control-mode client stands in for the widget
because it has no tty and sets its size with `refresh-client -C`):

```sh
tmux -L switchboard-test-X new-session -d -s sized -x 200 -y 50
tmux -L switchboard-test-X display-message -p -t =sized: '#{window_width}x#{window_height}'
# 200x50
tmux -L switchboard-test-X set-option -g window-size latest
printf 'refresh-client -C 120,40\n' | tmux -L switchboard-test-X -C attach -t =sized
tmux -L switchboard-test-X display-message -p -t =sized: '#{window_width}x#{window_height}'
# 120x40 while the client is attached
```

A daemon started from a foreground shell command does not survive in the
sandbox this was run from, so the measurement lives in the integration
test, which keeps the server alive for its duration.

**Detached windows.** With `latest`, a window with no client keeps its
last size; a window that never had a client keeps the size it was created
with (200x50), so snapshots and captions of unattached sessions are
unchanged.

**Recommendation.** Ship `window-size latest` in the config and re-apply
it on launch with `set-option -g window-size latest`, since the server
outlives the app and may have started with the old config. No resize
plumbing through the port is needed: the widget and Ghostty are both
clients. Fallback if a client ever fails to report its size: a
`ProcessHost::resize` method over `resize-window -x -y`.
