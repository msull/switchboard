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
    # Every session of the ticket, for the write-up: its transcript (what
    # it searched, read and wrote, turn by turn) and the launch Switchboard
    # recorded for it (argv and cwd). Switchboard's project records name
    # each session's transcript file; the planner's session also holds
    # its review-round answers, since Dispatch resumes it.
    python3 -I - "$d" "${SWITCHBOARD_DATA_DIR:-$HOME/Library/Application Support/Switchboard}" <<'PYIN'
import json, pathlib, shutil, sys
d, sb = pathlib.Path(sys.argv[1]), pathlib.Path(sys.argv[2])
show = json.loads((d / "show.json").read_text())
wanted = {a["session"]: f"{a['stage']}-{a['n']}" for a in show["attempts"] if a.get("session")}
records = {}
for f in sb.glob("projects/*.json"):
    for s in json.loads(f.read_text()).get("sessions", []):
        cc = (s.get("resume") or {}).get("ClaudeCode") or {}
        if cc.get("session_id") in wanted:
            records[cc["session_id"]] = {"name": s.get("name"), "cwd": s.get("cwd"), "launch": s.get("launch"),
                                        "transcript": cc.get("transcript"), "notes": s.get("notes")}
out = d / "transcripts"; out.mkdir(exist_ok=True)
launches = {}
for sid, label in wanted.items():
    r = records.get(sid)
    launches[label] = {"session": sid, **(r or {"missing": "no Switchboard record for this session"})}
    t = pathlib.Path(r["transcript"]) if r and r.get("transcript") else None
    if t and t.exists():
        shutil.copy(t, out / f"{label}.jsonl")
    else:
        for c in pathlib.Path.home().glob(f".claude/projects/*/{sid}.jsonl"):
            shutil.copy(c, out / f"{label}.jsonl")
(out / "launches.json").write_text(json.dumps(launches, indent=1) + "\n")
have = sorted(p.name for p in out.glob("*.jsonl"))
print(f"  transcripts: {', '.join(have) if have else 'none found'}")
PYIN
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
