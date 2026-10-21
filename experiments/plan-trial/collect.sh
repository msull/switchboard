#!/bin/sh
# Copies each pipeline arm's files out of Dispatch's data directory into
# results/<short>-<issue>/pipeline/<model>-<rep>/: the investigation, the
# first plan, the reviewed plan with its rounds, and the ticket's report.
# Run it once every arm of the issue waits at finalize. Safe to run again.
# usage: collect.sh <switchboard|simplesingletable> <issue>
set -eu
here=$(cd "$(dirname "$0")" && pwd)
. "$here/lib.sh"
repo=$1; issue=$2
repo_vars "$repo"
out="$here/results/$short-$issue/pipeline"
status=$("$DISPATCH" status)
last() { ls "$1" | sort -n | tail -1; }
for model in opus sonnet haiku; do
  for rep in a b; do
    arm="$short-$model-$rep"
    id=$(printf '%s\n' "$status" | awk -v p="$arm" -v n="#$issue" '$2==p && $3==n {print $1; exit}')
    if [ -z "$id" ]; then echo "no ticket for $arm #$issue" >&2; continue; fi
    d="$out/$model-$rep"
    mkdir -p "$d"
    echo "$id" > "$d/ticket.txt"
    t="$DATA/tickets/$id"
    [ -d "$t/investigate" ] && cp "$t/investigate/$(last "$t/investigate")/root/notes.md" "$d/notes.md" 2>/dev/null || true
    [ -d "$t/plan" ] && cp "$t/plan/$(last "$t/plan")/repo/plan.md" "$d/plan.first.md" 2>/dev/null || true
    if [ -d "$t/review" ]; then
      r="$t/review/$(last "$t/review")/repo"
      cp "$r"/plan*.md "$d/" 2>/dev/null || true
    fi
    "$DISPATCH" report "$id" --json > "$d/report.json" 2>/dev/null || true
    "$DISPATCH" show "$id" --json > "$d/show.json" 2>/dev/null || true
    printf '%s %s: rounds %s\n' "$arm" "$id" "$(ls "$d" | grep -c 'plan.feedback')"
  done
done
