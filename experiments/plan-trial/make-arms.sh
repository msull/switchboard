#!/bin/sh
# Writes the twelve trial projects into Dispatch's pipelines directory:
# two repositories, three models, two repetitions. Each is the base file
# under pipelines/ with its project name and model filled in. Safe to run
# again: a file is replaced, and a ticket already taken keeps its own copy.
set -eu
here=$(cd "$(dirname "$0")" && pwd)
data=${DISPATCH_DATA_DIR:-"$HOME/Library/Application Support/Dispatch"}
mkdir -p "$data/pipelines"
for pair in switchboard:sb simplesingletable:sst; do
  file=${pair%%:*}
  short=${pair##*:}
  for model in opus sonnet haiku; do
    for rep in a b; do
      arm="$short-$model-$rep"
      sed -e "s/^name = \"PROJECT\"/name = \"$arm\"/" \
          -e "s/\"MODEL\"/\"$model\"/g" \
          "$here/pipelines/$file.toml" > "$data/pipelines/$arm.toml"
      echo "$data/pipelines/$arm.toml"
    done
  done
done
