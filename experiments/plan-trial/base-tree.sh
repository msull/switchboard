#!/bin/sh
# Prints the path of a worktree at an issue's base commit, making it on
# first use under ~/code_repos/plan-trial (a trusted directory, so Claude
# asks no trust question). Switchboard comes from this repository, the
# others from GitHub.
# usage: base-tree.sh <switchboard|simplesingletable> <issue>
set -eu
here=$(cd "$(dirname "$0")" && pwd)
. "$here/lib.sh"
repo=$1; issue=$2
repo_vars "$repo"
base=$(cat "$here/results/$short-$issue/base.txt")
root="$HOME/code_repos/plan-trial"
clone="$root/$repo"
tree="$root/$short-$issue-base"
mkdir -p "$root"
if [ ! -d "$clone/.git" ]; then
  if [ "$repo" = switchboard ]; then git clone -q "$here/../.." "$clone"
  else git clone -q "https://github.com/$slug" "$clone"; fi
fi
if [ ! -d "$tree" ]; then
  git -C "$clone" fetch -q origin "$base" 2>/dev/null || git -C "$clone" fetch -q origin
  git -C "$clone" worktree add -q --detach "$tree" "$base"
fi
echo "$tree"
