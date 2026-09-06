#!/bin/sh
# Follow-up: control-mode notifications with the client kept attached, and
# Claude Code's title behaviour under TERM=xterm-ghostty inside tmux.
S=$(cd "$(dirname "$0")/.." && pwd)
T="tmux -L switchboard-spike-state -f /dev/null"
$T kill-server 2>/dev/null; rm -f "$S/hooks.log" "$S/tmux-claude2.raw"
$T new-session -d -s spike -x 120 -y 40 -c /Users/sully/code_repos -n idle 'sleep 600'
$T set -g remain-on-exit on
echo "=== 5b. control mode, client held open 4s"
( echo "new-window -d -n ctl 'sleep 1; echo bye; printf \"\\a\"; exit 5'"; sleep 4 ) | $T -C attach -t spike 2>&1 | grep -vE '^%(begin|end)' | cat -v
echo "=== 4b. Claude Code in pane with default-terminal xterm-ghostty (terminfo: $(infocmp xterm-ghostty >/dev/null 2>&1 && echo present || echo missing))"
$T set -g default-terminal xterm-ghostty
$T new-window -d -n claude -c /Users/sully/code_repos -e CLAUDECODE= -e CLAUDE_CODE_CHILD_SESSION= -e CLAUDE_CODE_SESSION_ID= -e CLAUDE_CODE_ENTRYPOINT= \
  "claude --model haiku --settings $S/settings-abs.json 'Use the Bash tool to run exactly: touch $S/spike-perm-test.txt'"
$T pipe-pane -t claude -O "cat >> $S/tmux-claude2.raw"
i=0; while [ $i -lt 30 ]; do sleep 0.25; i=$((i+1)); printf '[%5.2fs] ' $(echo "$i*0.25" | bc); $T display -p -t claude 'TERM=#{pane_start_command} title=#{pane_title} cmd=#{pane_current_command}' | cut -c1-90; done | uniq -c -f1
echo "--- hooks:"; jq -r '.hook_event_name + " " + (.notification_type // .tool_name // "")' "$S/hooks.log"
echo "--- OSC in raw:"; python3 - "$S/tmux-claude2.raw" <<'P'
import re,sys
b=open(sys.argv[1],'rb').read(); seen=[]
for m in re.finditer(rb'\x1b\](?:0|2|9|777);[^\x07\x1b]{0,80}',b):
    x=m.group(0)
    if x not in seen: seen.append(x); print("  ",x)
print("  BEL:",b.count(b'\x07'),"OSC9;4:",b.count(b'\x1b]9;4'),"bytes:",len(b))
P
$T display -p -t claude 'env TERM inside pane: #{pane_pid}'; ps eww -p $($T display -p -t claude '#{pane_pid}') | tr ' ' '\n' | grep -E '^TERM=' 
$T kill-server; echo "server killed: $($T ls 2>&1)"; rm -f "$S/spike-perm-test.txt"
