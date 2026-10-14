#!/bin/sh
# Publishes the app as a GitHub release: the signed bundle, built by
# bundle.sh into a staging directory and zipped with ditto (which keeps
# the signature and the resource forks), attached to a release of the
# current commit under a tag of the crate version, the date and the
# commit. Another Mac installs it with `gh release download`, which sets
# no quarantine flag, so Gatekeeper asks nothing of a bundle that is
# signed but not notarized; the same Developer ID on every release keeps
# that Mac's Accessibility grant and Keychain allowances across updates.
# `--dry-run` builds, signs and zips without tagging or publishing, and
# leaves the zip under the cargo target directory.
set -eu
cd "$(dirname "$0")/.."
DRY=0
if [ "${1:-}" = "--dry-run" ]; then DRY=1; fi
if [ -n "$(git status --porcelain --untracked-files=no)" ]; then
  echo "the tree has uncommitted changes; commit or stash them first" >&2
  exit 1
fi
VERSION=$(grep '^version' Cargo.toml | head -1 | sed 's/.*"\(.*\)".*/\1/')
SHA=$(git rev-parse --short HEAD)
DATE=$(date +%Y%m%d)
TAG="v$VERSION-$DATE-$SHA"
REPO=$(gh repo view --json nameWithOwner -q .nameWithOwner)
if git rev-parse -q --verify "refs/tags/$TAG" >/dev/null; then
  echo "tag $TAG exists; this commit is released" >&2
  exit 1
fi

STAGE=$(mktemp -d "${TMPDIR:-/tmp}/switchboard-release.XXXXXX")
trap 'rm -rf "$STAGE"' EXIT
BUNDLE_BUILD="$VERSION.$DATE" ./scripts/bundle.sh "$STAGE"
APP="$STAGE/Switchboard.app"
codesign --verify --deep --strict "$APP"
AUTHORITY=$(codesign -dvv "$APP" 2>&1 | sed -n 's/^Authority=//p' | head -1)
case "$AUTHORITY" in
  "Developer ID Application:"*) ;;
  *) echo "warning: signed by \"$AUTHORITY\", not a Developer ID; another Mac keeps no grant across releases" >&2 ;;
esac

ZIP="$STAGE/Switchboard-$TAG.zip"
ditto -c -k --keepParent "$APP" "$ZIP"
SUM=$(shasum -a 256 "$ZIP" | cut -d' ' -f1)
PREVIOUS=$(git describe --tags --abbrev=0 --match 'v*' 2>/dev/null || true)
NOTES="$STAGE/notes.md"
{
  echo "Switchboard $TAG, built from $SHA on $(date +%Y-%m-%d)."
  echo
  echo "Install on a Mac that has tmux and Ghostty:"
  echo
  echo '```sh'
  echo "gh release download $TAG -R $REPO -p 'Switchboard-*.zip' -D ~/Downloads"
  echo "ditto -x -k ~/Downloads/Switchboard-$TAG.zip ~/Applications"
  echo "open ~/Applications/Switchboard.app"
  echo '```'
  echo
  echo "SHA-256 of the zip: \`$SUM\`"
  echo
  if [ -n "$PREVIOUS" ]; then
    echo "Changes since $PREVIOUS:"
    echo
    git log --format='- %s' "$PREVIOUS..HEAD"
  else
    echo "The first release."
  fi
} > "$NOTES"

if [ "$DRY" = 1 ]; then
  TARGET=$(cargo metadata --format-version 1 --no-deps | sed 's/.*"target_directory":"\([^"]*\)".*/\1/')
  cp "$ZIP" "$TARGET/"
  echo "Dry run: $TARGET/$(basename "$ZIP") ($SUM); nothing tagged or published."
  echo "--- notes"
  cat "$NOTES"
  exit 0
fi
git tag -a "$TAG" -m "Switchboard $TAG"
git push origin "$TAG"
gh release create "$TAG" "$ZIP" --title "Switchboard $TAG" --notes-file "$NOTES"
echo "Published $TAG: https://github.com/$REPO/releases/tag/$TAG"
