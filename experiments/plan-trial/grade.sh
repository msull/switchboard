#!/bin/sh
# Scores one blind plan with one grader, from the same packet each time:
# the issue, rubric.md, the owner-checked reference, the plan, and a
# worktree at the issue's base commit. The grader never sees the key or
# another plan. Writes results/<short>-<issue>/grades/<letter>.<grader>.md.
# A stage run's plan is graded the same way by naming its run directory in
# place of the letter (results/<short>-<issue>/stages/plan/<run>): the
# grade lands beside the plan as grade.<grader>.md, the letter in the
# packet is the run's name, and the facts file the run was given is the
# one under its inputs.
# usage: grade.sh <switchboard|simplesingletable> <issue> <A..Z | stage run dir> <fable|codex>
set -eu
here=$(cd "$(dirname "$0")" && pwd)
. "$here/lib.sh"
repo=$1; issue=$2; letter=$3; grader=$4
repo_vars "$repo"
res="$here/results/$short-$issue"
tree=$("$here/base-tree.sh" "$repo" "$issue")
if [ -d "$letter" ] && [ -s "$letter/plan.md" ]; then
  run_dir=$(cd "$letter" && pwd); plan_file="$run_dir/plan.md"; facts_file="$run_dir/inputs/facts.md"
  letter=$(basename "$run_dir"); grades="$run_dir"
  out="$grades/grade.$grader.md"; prompt="$grades/grade.$grader.prompt.md"
  json="$grades/grade.$grader.json"; log="$grades/grade.$grader.log"; secs="$grades/grade.$grader.seconds"
else
  run_dir=""; plan_file="$res/blind/$letter.md"; facts_file=""
  grades="$res/grades"; mkdir -p "$grades"
  out="$grades/$letter.$grader.md"; prompt="$grades/$letter.$grader.prompt.md"
  json="$grades/$letter.$grader.json"; log="$grades/$letter.$grader.log"; secs="$grades/$letter.$grader.seconds"
fi
{
  echo "You are scoring one implementation plan for a GitHub issue, blind: you do not know who or what wrote it, and you score this plan on its own."
  echo "The repository at the issue's base commit is checked out at $tree (read-only; change nothing there). Every file path in the plan is relative to that tree."
  echo
  echo "Score by the rubric, column by column. Length is not thoroughness. For grounding, take the first five distinct file or line references the plan makes, in order of appearance, open each in the tree, and say what you found. For departures and decisions, use the reference as the yardstick; where you disagree with the reference, say so in the evidence rather than silently scoring your own view."
  echo
  echo "Your final message must be exactly this shape and nothing else:"
  echo
  echo '```'
  echo "| plan | grounding | departures caught | decisions to overturn | over-specification | hand-off |"
  echo "|---|---|---|---|---|---|"
  echo "| $letter | <0-3> | <0-3> | <0-3> | <0-3> | <yes|note|no> |"
  echo '```'
  echo
  echo "## Grounding"
  echo "- <ref 1 as the plan wrote it>: found / wrong (<what is there>)"
  echo "- ... five rows"
  echo "## Departures caught"
  echo "- <each departure in the reference>: caught / missed / caught with a wrong fix"
  echo "- <any departure the plan found that the reference lacks>"
  echo "## Decisions to overturn"
  echo "- <each call you would reverse, with the reference decision it conflicts with>; or none"
  echo "## Over-specification"
  echo "- <what an implementer would not need, and roughly what fraction of the plan it is>"
  echo "## Hand-off"
  echo "- <the one line of guidance if note; why not if no>"
  echo "## Notes"
  echo "- best: <one line>"
  echo "- worst: <one line>"
  echo
  echo "---"; echo; echo "# The issue"; echo; cat "$res/issue.md"
  echo; echo "---"; echo; echo "# The rubric"; echo; cat "$here/rubric.md"
  echo; echo "---"; echo; echo "# The reference (owner-checked)"; echo; cat "$res/reference.md"
  echo; echo "---"; echo; echo "# The plan ($letter)"; echo; cat "$plan_file"
  # A facts-arm plan cites the facts file its planner was given, which
  # lives in Dispatch's data directory, not the tree. The packet carries
  # it so a citation into it is checkable; the grader still sees one
  # plan and no key. The script reads the key only to find the file.
  if [ -z "$run_dir" ]; then
    arm=$(awk -v l="$letter" '$1==l {print $2}' "$res/blind/key.txt" 2>/dev/null || true)
    [ -n "$arm" ] && [ -s "$res/$arm/facts.md" ] && facts_file="$res/$arm/facts.md"
  fi
  if [ -n "$facts_file" ] && [ -s "$facts_file" ]; then
    echo; echo "---"; echo; echo "# The facts file the planner was given"
    echo; echo "The plan may cite this file as facts.md or facts:<line>; such a citation is checked against this text, not the tree. It was written by an earlier stage of the same process, not by the owner, so it is not the reference and may itself be wrong."
    echo; cat -n "$facts_file"
  fi
} > "$prompt"
start=$(date +%s)
case $grader in
  fable)
    (cd "$tree" && claude -p --model fable --output-format json \
      --permission-mode default \
      --allowedTools "Read,Grep,Glob,Bash(git diff *),Bash(git log *),Bash(git show *),Bash(git grep *),Bash(sed *),Bash(grep *),Bash(rg *),Bash(cat *),Bash(ls *),Bash(head *),Bash(tail *),Bash(wc *),Bash(find *)" \
      < "$prompt" > "$json")
    python3 -I - "$json" "$out" <<'PY'
import json, sys
r = json.load(open(sys.argv[1]))
r = r[-1] if isinstance(r, list) else r
open(sys.argv[2], "w").write(r.get("result", ""))
print(f"cost_usd={r.get('total_cost_usd')} turns={r.get('num_turns')} ms={r.get('duration_ms')}")
PY
    ;;
  codex)
    (cd "$tree" && codex exec -s read-only --ephemeral --color never \
      -o "$out" - < "$prompt" > "$log" 2>&1) || echo "codex exited $?; see $log"
    ;;
  *) echo "grader must be fable or codex" >&2; exit 2 ;;
esac
echo "$(( $(date +%s) - start ))" > "$secs"
echo "$out ($(wc -w < "$out") words, $(cat "$secs")s)"
