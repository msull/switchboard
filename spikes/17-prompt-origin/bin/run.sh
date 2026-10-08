#!/bin/sh
# Starts an interactive haiku `claude` in a throwaway directory under
# $HOME/code_repos on a private test tmux socket, with the spike's hooks.
# Usage: bin/run.sh <scratch dir>
set -eu
here=$(cd "$(dirname "$0")/.." && pwd)
scratch=$1
mkdir -p "$scratch"
work="$HOME/code_repos/switchboard-spike17-$$"
mkdir -p "$work"
sock=switchboard-test-spike17
tmux -L "$sock" -f /dev/null kill-server 2>/dev/null || true
tmux -L "$sock" -f /dev/null new-session -d -s s -x 200 -y 50 -c "$work" \
  env -u CLAUDECODE -u CLAUDE_CODE_ENTRYPOINT -u CLAUDE_CODE_CHILD_SESSION \
  SPIKE_LOG="$scratch/hooks.log" SPIKE_BIN="$here/bin" \
  claude --model haiku --settings "$here/settings-spike.json"
echo "$work"
