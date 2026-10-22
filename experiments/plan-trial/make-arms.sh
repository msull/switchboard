#!/bin/sh
# Writes the trial projects into Dispatch's pipelines directory: the
# twelve first-trial arms (two repositories, three models, two
# repetitions), the ten facts arms and the six revision 5 arms (sbf5-*).
# Each is the base file under pipelines/ with its project name and models
# filled in. Safe to run again: a file is replaced, and a ticket already
# taken keeps its own copy.
set -eu
here=$(cd "$(dirname "$0")" && pwd)
. "$here/lib.sh"
mkdir -p "$DATA/pipelines"
for pair in switchboard:sb simplesingletable:sst; do
  file=${pair%%:*}
  short=${pair##*:}
  for model in opus sonnet haiku; do
    for rep in a b; do
      arm="$short-$model-$rep"
      sed -e "s/^name = \"PROJECT\"/name = \"$arm\"/" \
          -e "s/\"MODEL\"/\"$model\"/g" \
          "$here/pipelines/$file.toml" > "$DATA/pipelines/$arm.toml"
      echo "$DATA/pipelines/$arm.toml"
    done
  done
done
for set_ in facts facts5; do
  set_vars "$set_"
  for scenario in $scenarios; do
    set -- $($models "$scenario")
    for rep in a b; do
      arm="$prefix-$scenario-$rep"
      sed -e "s/^name = \"PROJECT\"/name = \"$arm\"/" \
          -e "s/\"INVESTIGATE_MODEL\"/\"$1\"/" \
          -e "s/\"FACTS_MODEL\"/\"$2\"/" \
          -e "s/\"PLAN_MODEL\"/\"$3\"/" \
          -e "s/\"REVIEW_MODEL\"/\"$4\"/" \
          "$here/pipelines/$pipeline" > "$DATA/pipelines/$arm.toml"
      echo "$DATA/pipelines/$arm.toml"
    done
  done
done
