#!/bin/bash
# Spike 11 item 1, second half: kill-session, then what is left.
cd "$(dirname "$0")"
S=switchboard-test-spike11
PGID=$(cat pane.pgid)
tmux -L $S kill-session -t svc
for i in 1 2 3 4 5 6 7 8 9 10; do
  sleep 0.2
  left=$(pgrep -g "$PGID" | wc -l | tr -d ' ')
  listening=$(lsof -nP -iTCP:3155 -sTCP:LISTEN | wc -l | tr -d ' ')
  echo "after $((i*200))ms: $left in group, $listening listen lines"
done
ps -ax -o pid,pgid,ppid,command | grep -v grep | grep 'createServer' || echo "no grandchild left"
tmux -L $S kill-server 2>/dev/null || true
