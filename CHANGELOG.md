# Changelog

Every user-visible change lands as a fragment in `changelog.d/`, not
as a line here (format in `changelog.d/README.md`). `scripts/bump.sh`
folds the fragments, with anything left under Unreleased, into a
section for the next version with the date, and `scripts/release.sh`
publishes that section as the release notes. The format follows
[Keep a Changelog](https://keepachangelog.com) and the versions follow
[Semantic Versioning](https://semver.org).

## [Unreleased]

## [0.7.3] - 2026-10-26

### Fixed

- A paired device no longer sees secret values in a ticket's check logs, evidence text, body or events, which are masked like a session's screen, and images from an attempt that ran with environment grants are withheld.
- The remote port no longer reads a request line or header without limit, so a stranger on the network cannot grow the app's memory; a paired device's WebSocket frame is capped at 16 KiB.
- The remote port no longer lets one stranger on the network hold every unauthenticated connection slot: each address gets three, a slow handshake, request or body is closed at its deadline, and a connection closes after any reply to someone not yet paired, so a paired device is no longer shut out.
- One device on the local network can no longer close your pairing window or use up the pairing limit for the others: wrong codes are counted per address, an address that sends three is locked out while the window stays open, and Remote devices shows who is locked out.

## [0.7.2] - 2026-10-25

### Changed

- A pipeline file that sets a decisions dial other than ask for a human gate's decision is now refused at take and restart, naming the stage; a human gate always asks, and a supervisor's decides is how to automate it.
- A commit made in a Dispatch lane now runs the project's .githooks (set once on the clone, existing clones included), while Dispatch's own git calls still run no hook.

### Fixed

- A pipeline prompt can name `{decisions.stuck}` and `{decisions.service}`, and a running ticket's pipeline copy is no longer re-checked for unknown decision names.

## [0.7.1] - 2026-10-25

### Changed

- Concurrent tickets no longer conflict on the changelog: each change is a file under changelog.d/ that scripts/bump.sh folds into the release section.

### Fixed

- A plan review rerun after a failed attempt starts from the owner's revised copy, sends the revise note and any rerun note to its first round, and says which copy it reviews in the rerun question, in show and on the Plan tab.
- Dispatch runner children started through git (and its hooks), gh, curl, df, ps and kill no longer inherit the runner's launch token.
- A review round no longer fails as "never started a turn" when its reviewer pane was relaunched on Continue and is still starting up; Continue no longer sends the ask a second time into that pane.
- A ticket taken from the command line is now followed by the project's supervisor from the moment it is taken, and an active ticket it was missing is picked up on the next pass, with a gate already waiting delivered once.

## [0.7.0] - 2026-10-25

### Added

- The web page's sessions list opens on live sessions in one list, each row naming its project, with a search box and a Filters button for state, kind, projects and sort; the choices are remembered in the browser, a line counts the waiting sessions the filters hide, and an ended session shows how long ago it was last seen.

## [0.6.1] - 2026-10-25

### Changed

- A human gate's own question now follows the branch, diff, tree, notes and deployed lines instead of replacing them, a branch or diff line git cannot read is left out instead of failing the gate, and a `question` on the `lanes` gate is refused when the pipeline file loads.

## [0.6.0] - 2026-10-24

### Added

- Dispatch hands a human gate's answer and note to later prompts as `{decisions.<name>}` and `{decisions.<name>.note}`, empty while unanswered, and a human gate can carry its own `question` in place of the default one, so a `decisions` stage after `lanes` puts the lead's answers in the planner's first prompt.
- The web page shows a Claude Code session's conversation, turn by turn with each turn's steps folded under it, and loads earlier turns on request; on a tablet the page reads in a centred column.

### Fixed

- A review round whose prompt never started a turn in the reviewer now fails after three minutes, so Dispatch offers a rerun, instead of waiting forever.
- A reply sent from a paired device, by its Send or the web page's, now answers a session's standing ask, as one typed in the app does; a tool's `session.send` still leaves it.

## [0.5.0] - 2026-10-24

### Added

- The headset has a keys panel: a small movable window placed where the virtual keyboard goes, showing the caret's line of the prompt, with a microphone, a peek that keeps the whole prompt open above the panel, the AI pass, punctuation, a line break, a paragraph break that closes the sentence, delete and delete word, caret moves and the host's undo and redo, pressed by touch, that edits the prompt of the session being worked on after dictation.
- The remote port serves a web page: pair a phone's browser from the pairing window's QR, see the waiting sessions, read the last response or screen and send a reply.

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
