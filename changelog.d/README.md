# Changelog fragments

A user-visible change does not edit `CHANGELOG.md`. It adds one file
here in the same commit, named after its branch:
`changelog.d/<branch>.md`, so a branch `dispatch/<number>-<slug>` writes
`changelog.d/dispatch/<number>-<slug>.md`. Every branch writes its own
file, so concurrent branches never touch the same lines and merge
without a conflict.

Each non-blank line of a fragment is one changelog entry, starting with
its group:

```
Added: A new thing the user can do.
Changed: Something that now behaves differently.
Fixed: A bug that no longer happens.
```

A fragment may hold more than one line, each naming its own group. To
change the wording, edit the fragment.

`scripts/bump.sh` runs `scripts/fold-changelog.sh`, which writes the
fragments into the new version's section of `CHANGELOG.md` as `- text`
bullets under `### Added`, `### Changed` and `### Fixed`, in that order
and, within a group, in the sorted order of the fragment paths, then
deletes them. Entries still written under Unreleased in `CHANGELOG.md`
under those three headings join the group of the same name, after the
fragments' entries; text there outside them stops the fold. A line
without one of the three prefixes stops the fold before anything is
changed.

This file is not a fragment; it keeps the directory in git between
releases.
