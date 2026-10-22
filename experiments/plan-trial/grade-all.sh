#!/bin/sh
# Grades every blind plan of an issue (A to L, and the letters after
# them as facts arms are blinded) with both graders, four at a time,
# skipping any grade already written.
# usage: grade-all.sh <switchboard|simplesingletable> <issue> [fable|codex]
set -eu
here=$(cd "$(dirname "$0")" && pwd)
repo=$1; issue=$2; only=${3:-}
for l in A B C D E F G H I J K L M N O P Q R S T U V W X Y Z; do
  [ -s "$here/results/"*"-$issue/blind/$l.md" ] || continue   # not blinded yet
  for g in fable codex; do
    [ -n "$only" ] && [ "$g" != "$only" ] && continue
    [ -s "$here/results/"*"-$issue/grades/$l.$g.md" ] && continue
    echo "$l $g"
  done
done | xargs -P 4 -L 1 sh -c '"$0/grade.sh" "$1" "$2" "$3" "$4"' "$here" "$repo" "$issue"
