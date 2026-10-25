#!/bin/sh
# Moves the version on: sets it in Cargo.toml (and so in the binary,
# the bundle and the host's /info), refreshes Cargo.lock, and folds the
# fragments under changelog.d/ and whatever CHANGELOG.md holds under
# Unreleased into a section for the new version dated today
# (fold-changelog.sh), removing the fragments. The argument is patch,
# minor, major, or a version. Nothing is committed: review the diff,
# commit it as "Release X.Y.Z", then run release.sh.
set -eu
cd "$(dirname "$0")/.."
PART="${1:-}"
CURRENT=$(grep '^version' Cargo.toml | head -1 | sed 's/.*"\(.*\)".*/\1/')
MAJOR=${CURRENT%%.*}; REST=${CURRENT#*.}; MINOR=${REST%%.*}; PATCH=${REST#*.}
case "$PART" in
  major) NEXT="$((MAJOR + 1)).0.0" ;;
  minor) NEXT="$MAJOR.$((MINOR + 1)).0" ;;
  patch) NEXT="$MAJOR.$MINOR.$((PATCH + 1))" ;;
  [0-9]*.[0-9]*.[0-9]*) NEXT="$PART" ;;
  *) echo "usage: $0 patch|minor|major|X.Y.Z   (now $CURRENT)" >&2; exit 2 ;;
esac
if ! grep -q '^## \[Unreleased\]' CHANGELOG.md; then
  echo "CHANGELOG.md has no Unreleased section" >&2; exit 1
fi
if grep -q "^## \[$NEXT\]" CHANGELOG.md; then
  echo "CHANGELOG.md already has a section for $NEXT" >&2; exit 1
fi
if awk '/^## \[Unreleased\]/{f=1; next} /^## \[/{f=0} f && NF && !/^### /' CHANGELOG.md | grep -q .; then :;
elif [ -d changelog.d ] && find changelog.d -type f -name '*.md' ! -path changelog.d/README.md | grep -q .; then :; else
  echo "nothing to release: no fragment in changelog.d/ and nothing under Unreleased in CHANGELOG.md" >&2; exit 1
fi
TODAY=$(date +%Y-%m-%d)
# The fold runs first: it checks every fragment before writing, so a
# bad line stops here with nothing edited.
./scripts/fold-changelog.sh CHANGELOG.md changelog.d "$NEXT" "$TODAY"
sed -i '' "s/^version = \"$CURRENT\"/version = \"$NEXT\"/" Cargo.toml
# The lock file records the root crate's version; cargo refreshes it
# without touching any dependency.
cargo update --workspace --offline --quiet
echo "$CURRENT -> $NEXT in Cargo.toml, Cargo.lock and CHANGELOG.md, with changelog.d/ folded in."
echo "Review, commit as \"Release $NEXT\", then ./scripts/release.sh"
