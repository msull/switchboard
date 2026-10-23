# Changelog

Every user-visible change goes under Unreleased as it lands.
`scripts/bump.sh` moves that section under the next version with the
date, and `scripts/release.sh` publishes that section as the release
notes. The format follows [Keep a Changelog](https://keepachangelog.com)
and the versions follow [Semantic Versioning](https://semver.org).

## [Unreleased]

### Added

- The headset has a keys panel: a small movable window placed where the virtual keyboard goes, showing the caret's line of the prompt, with punctuation, a line break, a paragraph break that closes the sentence, delete and delete word, caret moves and the host's undo and redo, pressed by touch, that edits the prompt of the session being worked on after dictation.

## [0.4.1] - 2026-10-21

- Dispatch takes an issue once per project rather than once across the data directory, so a second project on the same repository can take an issue the first already holds.

## [0.4.0] - 2026-10-21

### Added

- Dispatch reports each ticket's tokens by model and agent time, per attempt, stage and ticket, in `dispatch report` and a Usage tab on the ticket page.

### Fixed

- The session page's token count counts each message once, not once per content block, skips Claude Code's synthetic messages, and names each model when a session used more than one.

## [0.3.0] - 2026-10-19

### Added

- A session can be opened from the headset: a plus button on the Sessions tab, or "New session in this project" on a listed session, opens a Claude Code, Codex or shell session in a project on any connected host paired under the control scope, in the project's root or a directory under it, with a first message for an agent; the new session's window opens on the reply. The remote port takes `POST projects/{id}/sessions` for it and lists `create` among its capabilities.
- The remote port serves Dispatch's tickets: a listing with each ticket's standing and decisions, one ticket with its body, attempts and lanes, and under the control scope a decision answered or a parked ticket resumed. The headset gets a Tickets tab on it.
- The headset reads what each stage produced: every attempt lists its notes and other artifacts, its checks log, its review rounds' findings and responses and its evidence, each opened with a tap as text or as a picture; a ticket's timeline is read from the runner's event log; and a closable ticket can be closed. The ticket list is ordered last moved first, with a menu for the order (waiting first, newest, by project), the hosts, projects and states (closed hidden unless asked) and a "waiting on me" switch.

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
