#!/bin/sh
# Folds the changelog fragments into a new version section. Usage:
#   fold-changelog.sh <changelog> <fragments dir> <version> <date>
# Each fragment is changelog.d/<branch>.md holding lines "Added: text",
# "Changed: text" or "Fixed: text" (changelog.d/README.md). The new
# section goes under the Unreleased heading with the fragments grouped
# Added, Changed, Fixed; what Unreleased already holds joins the group
# of the same name after the fragments' entries.
# The fragments are then removed. One file per change means two
# branches never edit the same lines, so they merge without conflict.
# Every line is checked before anything is written, so a bad fragment
# leaves the changelog and the fragments as they were.
set -eu
LOG="$1"; DIR="$2"; VERSION="$3"; DATE="$4"

if ! grep -q '^## \[Unreleased\]' "$LOG"; then
  echo "$LOG has no Unreleased section" >&2; exit 1
fi

LIST=$(mktemp "${TMPDIR:-/tmp}/fold-changelog.XXXXXX")
OUT=$(mktemp "${TMPDIR:-/tmp}/fold-changelog.XXXXXX")
trap 'rm -f "$LIST" "$OUT"' EXIT

if [ -d "$DIR" ]; then
  find "$DIR" -type f -name '*.md' ! -path "$DIR/README.md" | LC_ALL=C sort > "$LIST"
fi

# Fragment paths come from branch names, which have no spaces or
# newlines, so a line of the list is one path.
while IFS= read -r FRAG; do
  if ! awk -v f="$FRAG" '
    NF && !/^(Added|Changed|Fixed): / {
      printf "%s:%d: not an Added:, Changed: or Fixed: line: %s\n", f, NR, $0 > "/dev/stderr"
      bad = 1
    }
    END { exit bad }' "$FRAG"; then
    exit 1
  fi
done < "$LIST"

# Unreleased may hold entries under the same three group headings. Text
# outside them has no group to join, so it is refused like a bad fragment.
UNRELEASED_AWK='/^## \[Unreleased\]/ { u = 1; next } u && /^## / { exit }'
if ! awk "$UNRELEASED_AWK"'
  u && /^### (Added|Changed|Fixed)$/ { g = 1; next }
  u && /^### / { g = 0 }
  u && NF && !g {
    printf "%s: Unreleased holds text outside ### Added, ### Changed and ### Fixed: %s\n", FILENAME, $0 > "/dev/stderr"
    bad = 1
  }
  END { exit bad }' "$LOG"; then
  exit 1
fi

# The fragments' contents are read once per group so the groups come
# out in a fixed order, and within a group in the sorted path order.
{
  awk '{ print } /^## \[Unreleased\]/ { exit }' "$LOG"
  echo
  echo "## [$VERSION] - $DATE"
  for GROUP in Added Changed Fixed; do
    BULLETS=$(while IFS= read -r FRAG; do
      awk -v g="$GROUP: " 'index($0, g) == 1 { print "- " substr($0, length(g) + 1) }' "$FRAG"
    done < "$LIST"
    awk -v g="### $GROUP" "$UNRELEASED_AWK"' u && /^### / { cur = ($0 == g); next } u && cur && NF { print }' "$LOG")
    if [ -n "$BULLETS" ]; then
      echo
      echo "### $GROUP"
      echo
      printf '%s\n' "$BULLETS"
    fi
  done
  awk 'f { print } /^## \[Unreleased\]/ { u = 1; next } u && /^## / { f = 1; print ""; print; u = 0 }' "$LOG"
} > "$OUT"
# mktemp makes the file private; the changelog keeps ordinary modes.
chmod 644 "$OUT"
mv "$OUT" "$LOG"

while IFS= read -r FRAG; do
  rm "$FRAG"
done < "$LIST"
if [ -d "$DIR" ]; then
  find "$DIR" -mindepth 1 -type d -empty -delete
fi
