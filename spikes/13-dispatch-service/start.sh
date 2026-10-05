#!/bin/bash
# Spike 11 item 1: a forking dev server in a tmux pane launched the way
# Dispatch launches a service.
cd "$(dirname "$0")"
S=switchboard-test-spike11
tmux -L $S new-session -d -s svc -c "$PWD" zsh -lc 'exec "$@"' dispatch-service env BROWSER=none PORT=3155 node server.js
sleep 1.5
PANE=$(tmux -L $S list-panes -a -F '#{pane_pid}')
PGID=$(ps -o pgid= -p "$PANE" | tr -d ' ')
echo "pane $PANE pgid $PGID"
ps -o pid,pgid,ppid,command -g "$PGID"
curl -s -m 1 -o /dev/null -w 'http %{http_code}\n' http://127.0.0.1:3155/
lsof -nP -iTCP:3155 -sTCP:LISTEN
echo "$PGID" > pane.pgid
