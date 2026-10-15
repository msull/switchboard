#!/bin/sh
# Publishes the committed version as a GitHub release: the signed
# bundle, built by bundle.sh into a staging directory and zipped with
# ditto (which keeps the signature), plus the zip's SHA-256 as a file of
# its own, attached to a release tagged v<version> whose notes are the
# changelog's section for that version. The running app finds the
# release through the GitHub API, checks the zip against the SHA-256
# file and the bundle against the signing team, and installs it; another
# Mac installs it by hand with `gh release download`, which sets no
# quarantine flag, so Gatekeeper asks nothing of a bundle that is signed
# but not notarized. `--dry-run` builds, signs and zips without tagging
# or publishing, and leaves the zip under the cargo target directory.
set -eu
cd "$(dirname "$0")/.."
DRY=0
if [ "${1:-}" = "--dry-run" ]; then DRY=1; fi
if [ -n "$(git status --porcelain --untracked-files=no)" ]; then
  echo "the tree has uncommitted changes; commit or stash them first" >&2
  exit 1
fi
VERSION=$(grep '^version' Cargo.toml | head -1 | sed 's/.*"\(.*\)".*/\1/')
TAG="v$VERSION"
SHA=$(git rev-parse --short HEAD)
REPO=$(gh repo view --json nameWithOwner -q .nameWithOwner)
if git rev-parse -q --verify "refs/tags/$TAG" >/dev/null; then
  echo "tag $TAG exists; bump the version first (scripts/bump.sh)" >&2
  exit 1
fi
if ! grep -q "^## \[$VERSION\]" CHANGELOG.md; then
  echo "CHANGELOG.md has no section for $VERSION; run scripts/bump.sh" >&2
  exit 1
fi
if [ "$(git branch --show-current)" != "main" ]; then
  echo "warning: releasing from branch $(git branch --show-current), not main" >&2
fi

STAGE=$(mktemp -d "${TMPDIR:-/tmp}/switchboard-release.XXXXXX")
trap 'rm -rf "$STAGE"' EXIT
./scripts/bundle.sh "$STAGE"
APP="$STAGE/Switchboard.app"
codesign --verify --deep --strict "$APP"
AUTHORITY=$(codesign -dvv "$APP" 2>&1 | sed -n 's/^Authority=//p' | head -1)
case "$AUTHORITY" in
  "Developer ID Application:"*) ;;
  *) echo "warning: signed by \"$AUTHORITY\", not a Developer ID; the app refuses an update signed under another team" >&2 ;;
esac

ZIP="$STAGE/Switchboard-$VERSION.zip"
ditto -c -k --keepParent "$APP" "$ZIP"
(cd "$STAGE" && shasum -a 256 "$(basename "$ZIP")" > "$ZIP.sha256")
SUM=$(cut -d' ' -f1 "$ZIP.sha256")
NOTES="$STAGE/notes.md"
{
  # The changelog's section for this version, without its heading.
  awk -v v="$VERSION" '$0 ~ "^## \\[" v "\\]" {f=1; next} /^## \[/{f=0} f' CHANGELOG.md
  echo
  echo "Built from $SHA. Install by hand on a Mac that has tmux and Ghostty:"
  echo
  echo '```sh'
  echo "gh release download $TAG -R $REPO -p 'Switchboard-*.zip' -D ~/Downloads"
  echo "ditto -x -k ~/Downloads/Switchboard-$VERSION.zip ~/Applications"
  echo "open ~/Applications/Switchboard.app"
  echo '```'
  echo
  echo "A running Switchboard offers this release from its Settings menu. SHA-256 of the zip: \`$SUM\`"
} > "$NOTES"

if [ "$DRY" = 1 ]; then
  TARGET=$(cargo metadata --format-version 1 --no-deps | sed 's/.*"target_directory":"\([^"]*\)".*/\1/')
  cp "$ZIP" "$ZIP.sha256" "$TARGET/"
  echo "Dry run: $TARGET/$(basename "$ZIP") ($SUM); nothing tagged or published."
  echo "--- notes"
  cat "$NOTES"
  exit 0
fi
git tag -a "$TAG" -m "Switchboard $TAG"
git push origin "$TAG"
gh release create "$TAG" "$ZIP" "$ZIP.sha256" --title "Switchboard $TAG" --notes-file "$NOTES"
echo "Published $TAG: https://github.com/$REPO/releases/tag/$TAG"
