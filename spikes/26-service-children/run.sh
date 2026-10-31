#!/bin/bash
# usage: ./run.sh   — each case: a pane runs `$SHELL -lc 'exec "$@"' dispatch-service bash -c '<worker> & exec <web>'`
S=switchboard-test-spike26
T="tmux -L $S"
alive() { ps -o pid=,pgid=,stat=,command= -p "$@" 2>/dev/null | grep -v '^$' || echo "(gone)"; }
case_() {  # name, shell body, action
  local name=$1 body=$2 action=$3
  $T kill-server 2>/dev/null
  $T new-session -d -s hold -x 80 -y 24 "sleep 600"
  $T set -g remain-on-exit on
  $T new-session -d -s c -x 80 -y 24 "$SHELL -lc 'exec \"\$@\"' dispatch-service bash -c '$body'"
  sleep 1
  echo "== $name"
  echo "pane_pid=$($T display -p -t c '#{pane_pid}')"
  echo "sleepers before (pid pgid ppid):"; ps -axo pid,pgid,ppid,stat,command | grep 'sleep 7[0-9][0-9]' | grep -v grep
  eval "$action"
  sleep 1
  echo "pane_dead=$($T display -p -t c '#{pane_dead}' 2>&1)"
  echo "sleepers after:"; ps -axo pid,pgid,ppid,stat,command | grep 'sleep 7[0-9][0-9]' | grep -v grep || echo "(none)"
  pkill -f 'sleep 7[0-9][0-9]'; $T kill-server 2>/dev/null; sleep 0.3
}
KS='$T kill-session -t c'
case_ plain  'sleep 701 & exec sleep 702' "$KS"
case_ trap-hup 'trap "" HUP; sleep 703 & exec sleep 704' "$KS"
case_ nohup 'nohup sleep 705 >/dev/null 2>&1 & exec sleep 706' "$KS"
case_ setsid 'perl -MPOSIX -e "setsid(); exec q(sleep), q(707)" & exec sleep 708' "$KS"
case_ web-exits-first 'sleep 709 & exec sleep 2' 'sleep 3'
case_ web-exits-then-kill 'sleep 710 & exec sleep 2' 'sleep 3; $T kill-session -t c'
