#!/bin/bash
# Spike 11 item 1 with the real dev server: `npm start` in a Orchard
# frontend worktree, launched as Dispatch launches a service.
S=switchboard-test-spike11
WT=${1:?usage: npm.sh <a Orchard frontend worktree with node_modules>}
tmux -L $S new-session -d -s svc -c "$WT" zsh -lc 'exec "$@"' dispatch-service env BROWSER=none PORT=3155 npm start
PANE=$(tmux -L $S list-panes -a -F '#{pane_pid}')
PGID=$(ps -o pgid= -p "$PANE" | tr -d ' ')
echo "pane $PANE pgid $PGID"
start=$(date +%s)
for i in $(seq 1 180); do
  code=$(curl -s -m 1 -o /dev/null -w '%{http_code}' http://127.0.0.1:3155/)
  if [ "$code" != "000" ]; then
    echo "answered $code after $(( $(date +%s) - start ))s"
    break
  fi
  sleep 1
done
ps -o pid,pgid,ppid,command -g "$PGID" | cut -c1-140
lsof -nP -iTCP:3155 -sTCP:LISTEN
tmux -L $S capture-pane -p -t svc | tail -15
tmux -L $S kill-session -t svc
for i in 1 2 3 4 5 6 7 8 9 10 11 12 13 14 15; do
  sleep 0.2
  left=$(pgrep -g "$PGID" | wc -l | tr -d ' ')
  listening=$(lsof -nP -iTCP:3155 -sTCP:LISTEN | wc -l | tr -d ' ')
  echo "after $((i*200))ms: $left in group, $listening listen lines"
done
pgrep -fl 'react-app-rewired|react-scripts' | grep 3155 || echo "no dev server process names left with this port"
tmux -L $S kill-server 2>/dev/null || true
