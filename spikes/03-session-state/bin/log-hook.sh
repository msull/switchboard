#!/bin/sh
# Spike hook: read the JSON payload Claude Code sends on stdin, append one
# JSON line to hooks.log next to this script's project. $1 = event name as
# registered (the payload also carries hook_event_name; we log both).
LOG="$(dirname "$0")/../hooks.log"
payload=$(cat)
printf '%s\n' "$payload" | jq -c --arg ev "$1" --arg ts "$(date +%s)" '{ts:$ts, registered:$ev} + .' >> "$LOG" 2>>"$LOG.err" || printf '{"registered":"%s","raw":%s}\n' "$1" "$payload" >> "$LOG"
exit 0
