#!/bin/sh
# The whole revision 5 pipeline on one issue, stage by stage through
# stage.sh: investigate, facts, plan, review round one, the planner's
# answer, review round two, the planner's answer. What differs from the
# pipeline is in stage.py's doc comment (claude -p, acceptEdits, respond
# in a fresh session). The final plan is respond2's plan.md.
# usage: chain.sh <label> <investigate model> <model for the rest> <rep> <results/<issue> dir> [pipeline.toml]
set -eu
here=$(cd "$(dirname "$0")" && pwd)
label=$1; inv=$2; model=$3; rep=$4; res=$5; pipe=${6:-$here/pipelines/switchboard-facts-v5.toml}
from=$(ls -d "$res"/pipeline/* | head -1)      # any finished run: only its issue and base are used
st="$res/stages"
"$here/stage.sh" investigate "$label" "$inv" "$rep" "$from" "$pipe"
NOTES="$st/investigate/$label-$inv-$rep/notes.md" \
  "$here/stage.sh" facts "$label" "$model" "$rep" "$from" "$pipe"
NOTES="$st/investigate/$label-$inv-$rep/notes.md" FACTS="$st/facts/$label-$model-$rep/facts.md" \
  "$here/stage.sh" plan "$label" "$model" "$rep" "$from" "$pipe"
PLAN="$st/plan/$label-$model-$rep/plan.md" FACTS="$st/facts/$label-$model-$rep/facts.md" \
  "$here/stage.sh" review "$label" "$model" "$rep" "$from" "$pipe"
PLAN="$st/plan/$label-$model-$rep/plan.md" FEEDBACK="$st/review/$label-$model-$rep/plan.feedback-1.md" \
  "$here/stage.sh" respond "$label" "$model" "$rep" "$from" "$pipe"
if grep -qx 'No further feedback.' "$st/review/$label-$model-$rep/plan.feedback-1.md"; then
  echo "converged after round one"; exit 0
fi
PLAN="$st/respond/$label-$model-$rep/plan.md" RESPONSE="$st/respond/$label-$model-$rep/plan.response-1.md" \
  FACTS="$st/facts/$label-$model-$rep/facts.md" \
  "$here/stage.sh" review2 "$label" "$model" "$rep" "$from" "$pipe"
PLAN="$st/respond/$label-$model-$rep/plan.md" FEEDBACK="$st/review2/$label-$model-$rep/plan.feedback-2.md" \
  "$here/stage.sh" respond2 "$label" "$model" "$rep" "$from" "$pipe"
