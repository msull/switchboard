#!/bin/sh
# Records the base commit, then takes one issue in all six pipeline arms of
# a repository at once, so every arm plans against the same main.
# usage: take.sh <switchboard|simplesingletable> <issue>
set -eu
here=$(cd "$(dirname "$0")" && pwd)
. "$here/lib.sh"
repo=$1; issue=$2
repo_vars "$repo"
out="$here/results/$short-$issue"
mkdir -p "$out"
base=$(git ls-remote "https://github.com/$gh_repo.git" refs/heads/main | cut -f1)
echo "$base" > "$out/base.txt"
echo "base $base"
for model in opus sonnet haiku; do
  for rep in a b; do
    "$DISPATCH" take "$short-$model-$rep" "$issue"
  done
done
"$DISPATCH" status | awk -v n="#$issue" '$3==n'
