#!/bin/sh
# Usage: watch-new-session.sh <cwd> [timeout-seconds]
# Polls ~/.claude/projects/<slug-of-cwd>/ and prints the first new *.jsonl
# basename (the session id) that appears. Slug = cwd with every '/' , '_' and '.'
# replaced by '-'.
cwd="$1"; timeout="${2:-60}"
slug=$(printf '%s' "$cwd" | sed 's/[\/_.]/-/g')
dir="$HOME/.claude/projects/$slug"
mkdir -p "$dir"
before=$(ls "$dir" 2>/dev/null)
i=0
while [ "$i" -lt $((timeout*10)) ]; do
  for f in "$dir"/*.jsonl; do
    [ -e "$f" ] || continue
    b=$(basename "$f" .jsonl)
    case "$before" in *"$b.jsonl"*) ;; *) echo "$b"; exit 0;; esac
  done
  sleep 0.1; i=$((i+1))
done
echo "timeout" >&2; exit 1
