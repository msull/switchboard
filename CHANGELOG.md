# Changelog

Every user-visible change goes under Unreleased as it lands.
`scripts/bump.sh` moves that section under the next version with the
date, and `scripts/release.sh` publishes that section as the release
notes. The format follows [Keep a Changelog](https://keepachangelog.com)
and the versions follow [Semantic Versioning](https://semver.org).

## [Unreleased]

### Changed

- The headset's conversation view follows the end as a mode: it is held at the end, nothing growing under it can leave it behind, and a button at the right edge releases it for reading; the ellipsis at the foot returns it to following. Before, a new turn that began with a tool call could push the end off screen and switch following off, and the view could jump up to the turn it last held at the top.

### Fixed

- The headset's filter menu no longer jumps back to its top every couple of seconds while open: it is rebuilt only when a host or a session state it lists changes, not on every refresh.

## [0.2.1] - 2026-10-15

### Fixed

- A first launch on a display macOS reports at an odd scale (a Sidecar
  screen) no longer aborts before the window shows: the GPU's own
  texture limit is asked for instead of egui's 8192 (#185).

## [0.2.0] - 2026-10-15

### Added

- The app's version in the Settings menu, with a check for a newer
  release on GitHub twenty seconds after launch and hourly after that.
  A newer one is offered for download, checked against its published
  checksum and the running app's signing team, and installed on
  "Restart to update", which quits, swaps the bundle in place and
  relaunches.
- The headset pairs with a discovered host by its typed code, with the
  host's key shown for a check by eye, since visionOS gives an app no
  camera.
- `scripts/bump.sh` and `scripts/release.sh`: the version, this
  changelog and a GitHub release with the signed bundle and its
  checksum.
- The host's `/info` reply names the app's version, and the headset
  shows it beside each known host.

## [0.1.0] - 2026-10-14

### Added

- The remote port and the visionOS client: pairing with a pinned key,
  sessions and working sets from several hosts, live screens,
  conversations, run output, the draft and dictation into the host's
  editor, commands under the control scope.
- The first GitHub release, built by hand.
