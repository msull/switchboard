#!/bin/sh
# tmux part of spike 03. Private server, no keystrokes are ever sent to a
# window: every pane is started with its command line. Kills the server at end.
S=$(cd "$(dirname "$0")/.." && pwd)
T="tmux -L switchboard-spike-state -f /dev/null"
$T kill-server 2>/dev/null; rm -f "$S/tmux-alerts.log" "$S/tmux-claude.raw" "$S/tmux-osc.raw" "$S/hooks.log"
$T new-session -d -s spike -x 120 -y 40 -c /Users/sully/code_repos -n idle 'sleep 600'
$T set -g remain-on-exit on
$T set -g monitor-bell on
$T set -g monitor-activity on
$T set -g monitor-silence 3
$T set -g bell-action any
$T set -g set-titles off
$T set -g allow-passthrough on
for h in alert-bell alert-activity alert-silence pane-died pane-exited; do
  $T set-hook -g $h "run-shell 'echo \"\$(date +%s) $h #{hook_window} #{hook_pane}\" >> $S/tmux-alerts.log'"
done
F='#{window_name}\t#{pane_current_command}\tdead=#{pane_dead}\tstatus=#{pane_dead_status}\tbell=#{window_bell_flag}\tact=#{window_activity_flag}\tsil=#{window_silence_flag}\ttitle=#{pane_title}'
snap() { echo "--- t=$1"; $T list-panes -a -F "$F"; }

echo "=== 1. exit status via remain-on-exit"
$T new-window -d -n dead 'sleep 1; exit 3'
sleep 0.3; snap "0.3s (dead window still running)"; sleep 1.5; snap "1.8s"

echo "=== 2. bell / activity / silence flags (window not current, so flags can set)"
$T new-window -d -n bell 'sleep 1; printf "\a"; sleep 1; echo more; sleep 30'
sleep 3; snap "3s after bell window started"; sleep 3; snap "6s (silence 3s should have tripped)"
echo "--- alerts log:"; cat "$S/tmux-alerts.log"

echo "=== 3. OSC passthrough: 0 (title), 777 (notify), 133 (prompt marks), 7 (cwd)"
$T new-window -d -n osc 'sleep 2; printf "\033]0;TITLE-FROM-OSC0\007"; printf "\033]777;notify;X;needs you\007"; printf "\033]133;A\007prompt$ \033]133;B\007cmd\033]133;C\007out\n\033]133;D;7\007"; printf "\033]7;file://h/tmp\007"; printf "\033Ptmux;\033\033]777;notify;X;wrapped\007\033\\"; sleep 30'
$T pipe-pane -t osc -O "cat >> $S/tmux-osc.raw"
sleep 3.5; snap "after osc window"
echo "--- capture-pane -e -p (escapes tmux kept in the grid):"; $T capture-pane -e -p -t osc | grep -v '^$' | cat -v
echo "--- pipe-pane raw bytes (what the pty wrote):"; cat -v "$S/tmux-osc.raw"; echo
echo "--- real Ghostty bash integration, commands on stdin (no keystrokes):"
$T new-window -d -n gsh -e GHOSTTY_RESOURCES_DIR=/Applications/Ghostty.app/Contents/Resources/ghostty -e TERM=xterm-ghostty "bash --norc --noprofile -i <<'X'
source /Applications/Ghostty.app/Contents/Resources/ghostty/shell-integration/bash/ghostty.bash
echo hi; false
sleep 20
X"
$T pipe-pane -t gsh -O "cat >> $S/tmux-gsh.raw"
sleep 2; snap "ghostty bash in pane"; echo "--- pipe-pane raw:"; cat -v "$S/tmux-gsh.raw" | head -c 800; echo

echo "=== 4. Claude Code in a pane, initial prompt as argv (no keystrokes), hooks via --settings"
$T new-window -d -n claude -c /Users/sully/code_repos -e TERM=xterm-ghostty -e CLAUDECODE= -e CLAUDE_CODE_CHILD_SESSION= -e CLAUDE_CODE_SESSION_ID= -e CLAUDE_CODE_ENTRYPOINT= \
  "claude --model haiku --settings $S/settings-abs.json 'Use the Bash tool to run exactly: touch $S/spike-perm-test.txt'"
$T pipe-pane -t claude -O "cat >> $S/tmux-claude.raw"
i=0; while [ $i -lt 24 ]; do sleep 1; i=$((i+1)); printf '[%2ds] ' $i; $T list-panes -t claude -F "$F"; done
echo "--- hooks seen:"; jq -r '.hook_event_name + " " + (.notification_type // .tool_name // "")' "$S/hooks.log" 2>/dev/null
echo "--- OSC sequences in pipe-pane raw:"; python3 - "$S/tmux-claude.raw" <<'P'
import re,sys
b=open(sys.argv[1],'rb').read(); seen=[]
for m in re.finditer(rb'\x1b\](?:0|2|9|777);[^\x07\x1b]{0,80}',b):
    x=m.group(0)
    if x not in seen: seen.append(x); print("  ",x)
print("  BEL count:",b.count(b'\x07'),"bytes:",len(b))
P
echo "--- capture-pane -e tail (grid):"; $T capture-pane -e -p -t claude | grep -v '^\s*$' | tail -6 | cat -v | cut -c1-160
echo "--- pane_title of claude pane:"; $T display -p -t claude '#{pane_title} | cmd=#{pane_current_command} | pid=#{pane_pid}'
$T kill-window -t claude; sleep 1
echo "--- after kill-window, hooks:"; jq -r '.hook_event_name + " " + (.reason // "")' "$S/hooks.log" | tail -2
echo "=== 5. control mode notifications (tmux -C) for a dying pane"
( $T -C attach -t spike <<'C'
new-window -d -n ctl 'sleep 1; echo bye; exit 5'
C
) 2>&1 | head -20 &
sleep 3; wait
$T kill-server; echo "server killed: $($T ls 2>&1)"
rm -f "$S/spike-perm-test.txt"
