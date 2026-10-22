#!/bin/sh
# Records the base commit, then takes one issue in every arm of a set at
# once, so every arm plans against the same main. The set is the six
# first-trial arms of a repository (the default) or the six facts arms
# (`facts`, Switchboard only). A facts run of an issue the first trial
# already planned is refused unless main is still at that issue's base
# commit, so the six new plans compare with the twelve old ones.
# With `facts`, scenario names after it limit the take to those arms
# (a project that already holds the issue is refused by dispatch, so a
# later batch names only its new scenarios).
# usage: take.sh <switchboard|simplesingletable> <issue> [facts|facts5 [<scenario>...]]
set -eu
here=$(cd "$(dirname "$0")" && pwd)
. "$here/lib.sh"
repo=$1; issue=$2; set_=${3:-}
[ $# -ge 3 ] && shift 3 || shift $#
named=$*
repo_vars "$repo"
out="$here/results/$short-$issue"
mkdir -p "$out"
base=$(git ls-remote "https://github.com/$gh_repo.git" refs/heads/main | cut -f1)
if [ -s "$out/base.txt" ] && [ "$(cat "$out/base.txt")" != "$base" ]; then
  echo "main is at $base but $out/base.txt says $(cat "$out/base.txt"); the arms would plan against different code" >&2
  exit 1
fi
echo "$base" > "$out/base.txt"
echo "base $base"
if [ -n "$set_" ]; then
  [ "$repo" = switchboard ] || { echo "the facts arms exist for switchboard only" >&2; exit 2; }
  set_vars "$set_"
  for scenario in ${named:-$scenarios}; do
    $models "$scenario" > /dev/null
    for rep in a b; do
      "$DISPATCH" take "$prefix-$scenario-$rep" "$issue"
    done
  done
else
  for model in opus sonnet haiku; do
    for rep in a b; do
      "$DISPATCH" take "$short-$model-$rep" "$issue"
    done
  done
fi
"$DISPATCH" status | awk -v n="#$issue" '$3==n'
