#!/bin/sh
# One headless Claude Code session writes a plan for an issue with no
# investigator and no review: the baseline the pipeline is measured against.
# The prompt is the issue plus the pipeline planner's guidance, verbatim.
# usage: single.sh <switchboard|simplesingletable> <issue> <opus|sonnet|fable> <a|b>
# The base commit comes from results/<short>-<issue>/base.txt (take.sh writes it).
set -eu
here=$(cd "$(dirname "$0")" && pwd)
. "$here/lib.sh"
repo=$1; issue=$2; model=$3; rep=$4
repo_vars "$repo"
out="$here/results/$short-$issue/single/$model-$rep"
base=$(cat "$here/results/$short-$issue/base.txt")
root=${PLAN_TRIAL_ROOT:-"$HOME/code_repos/plan-trial"}
clone="$root/$repo"
tree="$root/$short-$issue-single-$model-$rep"
mkdir -p "$root" "$out"
[ -d "$clone/.git" ] || git clone -q "https://github.com/$gh_repo.git" "$clone"
git -C "$clone" fetch -q origin
[ -d "$tree" ] || git -C "$clone" worktree add -q --detach "$tree" "$base"
{
  gh issue view "$issue" --repo "$gh_repo" --json number,title,body \
    --template 'Issue #{{.number}}: {{.title}}

{{.body}}

'
  cat "$here/prompts/$repo.md"
  printf '\nRead the repository first. Then write the plan to plan.md in the current directory, and write nothing else.\n'
} > "$out/prompt.md"
echo "$base" > "$out/base.txt"
cd "$tree"
start=$(date +%s)
claude -p --model "$model" --output-format json \
  --permission-mode acceptEdits \
  --allowedTools "Bash(git *) Bash(ls *) Bash(cat *) Bash(grep *) Bash(rg *) Bash(find *) Bash(sed *) Bash(wc *) Bash(head *) Bash(tail *)" \
  < "$out/prompt.md" > "$out/run.json" || echo "claude exited $?" >&2
echo $(( $(date +%s) - start )) > "$out/seconds.txt"
if [ -f plan.md ]; then
  cp plan.md "$out/plan.md"
  echo "$out/plan.md"
else
  echo "no plan.md written for $short-$issue $model-$rep" >&2
fi
