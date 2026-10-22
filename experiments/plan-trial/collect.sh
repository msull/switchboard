#!/bin/sh
# Copies each pipeline arm's files out of Dispatch's data directory into
# results/<short>-<issue>/pipeline/<model>-<rep>/ (the first trial's
# arms) or results/<short>-<issue>/facts/<scenario>-<rep>/ (the facts
# arms, with `facts`): the investigation, the facts file, the first plan,
# the reviewed plan with its rounds, and the ticket's report. A facts arm
# also gets spikes.txt: the files its facts stage committed on the
# branch, or an empty file. Run it once every arm of the set waits at
# finalize. Safe to run again. A closed ticket for the arm (one retaken
# after main moved) is skipped.
# usage: collect.sh <switchboard|simplesingletable> <issue> [facts]
set -eu
here=$(cd "$(dirname "$0")" && pwd)
. "$here/lib.sh"
repo=$1; issue=$2; set_=${3:-}
repo_vars "$repo"
status=$("$DISPATCH" status)
last() { ls "$1" | sort -n | tail -1; }
if [ "$set_" = facts ]; then
  out="$here/results/$short-$issue/facts"
  arms=$(for s in $FACTS_SCENARIOS; do for r in a b; do echo "sbf-$s-$r $s-$r"; done; done)
else
  out="$here/results/$short-$issue/pipeline"
  arms=$(for m in opus sonnet haiku; do for r in a b; do echo "$short-$m-$r $m-$r"; done; done)
fi
echo "$arms" | while read -r arm name; do
    id=$(printf '%s\n' "$status" | awk -v p="$arm" -v n="#$issue" '$2==p && $3==n && $0 !~ /· closed/ {print $1; exit}')
    if [ -z "$id" ]; then echo "no ticket for $arm #$issue" >&2; continue; fi
    d="$out/$name"
    mkdir -p "$d"
    echo "$id" > "$d/ticket.txt"
    t="$DATA/tickets/$id"
    [ -d "$t/investigate" ] && cp "$t/investigate/$(last "$t/investigate")/root/notes.md" "$d/notes.md" 2>/dev/null || true
    [ -d "$t/facts" ] && cp "$t/facts/$(last "$t/facts")/repo/facts.md" "$d/facts.md" 2>/dev/null || true
    [ -d "$t/plan" ] && cp "$t/plan/$(last "$t/plan")/repo/plan.md" "$d/plan.first.md" 2>/dev/null || true
    if [ -d "$t/review" ]; then
      r="$t/review/$(last "$t/review")/repo"
      cp "$r"/plan*.md "$d/" 2>/dev/null || true
    fi
    "$DISPATCH" report "$id" --json > "$d/report.json" 2>/dev/null || true
    "$DISPATCH" show "$id" --json > "$d/show.json" 2>/dev/null || true
    if [ "$set_" = facts ]; then
      # What the facts stage committed: every file on the branch beyond
      # the base commit. The facts stage is the only one that commits
      # before finalize, so this is the spike, or nothing.
      tree=$(python3 -I -c 'import json,sys; d=json.load(open(sys.argv[1])); print(d["lanes"][0]["worktree"])' "$d/show.json" 2>/dev/null || true)
      base=$(cat "$here/results/$short-$issue/base.txt")
      if [ -n "$tree" ] && [ -d "$tree" ]; then
        git -C "$tree" diff --name-only "$base" HEAD > "$d/spikes.txt" 2>/dev/null || : > "$d/spikes.txt"
        if [ -s "$d/spikes.txt" ]; then
          mkdir -p "$d/branch" && git -C "$tree" diff "$base" HEAD > "$d/branch/spike.diff"
        fi
      else
        : > "$d/spikes.txt"
      fi
    fi
    printf '%s %s: rounds %s%s\n' "$arm" "$id" "$(ls "$d" | grep -c 'plan.feedback')" \
      "$([ -s "$d/spikes.txt" ] && echo ", spike: $(wc -l < "$d/spikes.txt" | tr -d ' ') files" || true)"
done
