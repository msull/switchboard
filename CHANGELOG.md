# Changelog

Every user-visible change lands as a fragment in `changelog.d/`, not
as a line here (format in `changelog.d/README.md`). `scripts/bump.sh`
folds the fragments, with anything left under Unreleased, into a
section for the next version with the date, and `scripts/release.sh`
publishes that section as the release notes. The format follows
[Keep a Changelog](https://keepachangelog.com) and the versions follow
[Semantic Versioning](https://semver.org).

## [Unreleased]

## [0.11.0] - 2026-10-30

### Added

- A pipeline's agent stage can carry a human gate that holds its session open while you work in it; answering proceed checks the running note and a clean tree, records the head and sends the branch on to review, and relaunch starts a fresh session on the same note.

## [0.10.0] - 2026-10-29

### Added

- The remote port serves any text file under a project's root, not only pinned ones, and each turn of a conversation names the files the agent wrote about, so a paired device can open them.
- On the headset, a file the agent names in a conversation is a link, and a tap opens it in its own window at the line named.
- Pin boards on the headset: a window of live tiles of sessions and files from any host, equal in size in a grid of two, three or four across, reordered by dragging a tile's bar, and saved, so a board opened again after a relaunch shows the same tiles in the same order; a pin button on every card and file window, and the boards listed in the main window in place of the hosts' working sets.

### Changed

- On the headset a new session card opens in portrait, 580 by 900 points, rather than wider than tall.

## [0.9.0] - 2026-10-27

### Added

- The web page shows a command or service session's runs, with the output of the one you pick and the names of the files it produced.

### Changed

- Update checks now run once a day at a spread-out time instead of hourly, send the last ETag so an unchanged release is not downloaded again, wait a day after a rate limit or server error instead of retrying, and a failed background check no longer takes an offered update off the screen.
- While the window is hidden, Switchboard checks tmux every 5 seconds instead of every second (hook events still update at once), and no longer runs `hostname` to name a new server identity.
- A pull request's branch is folded into one commit with the implementation's message.
- dispatch report prices a ticket's tokens as a labelled dollar estimate (cache included, models with no rate named) and brief shows each open ticket's running estimate; [policy] rates now takes five fields per model and overrides a built-in default table.

### Fixed

- The web page no longer leaves a gap in "Earlier turns" when the conversation grew meanwhile, no longer shows the previous session's "Earlier turns" button or error after a switch, and no longer rereads the open conversation when another session printed.
- A question an agent asked you now stays on its card when a review round, a response or a hand-off compaction is sent to it, instead of vanishing when the agent starts the turn.
- A rebase that conflicted at the ready stage of a ticket with an open pull request is now reviewed before its checks are read, and a fix from that review is pushed to the pull request.
- A runner older than the app or its CLI is now named by `dispatch health` and `dispatch brief`, restarted by the app when no attempt is open (or offered otherwise), and a prompt naming a `{decisions.*}` value the runner did not set fails the attempt instead of reaching the agent as written.
- The Mac app now gives macOS a reason for its Local Network request and names the _switchboard._tcp service, so the first prompt after this update carries Switchboard's own text, and the Automation prompt says it uses System Events to bring a session's Ghostty window to the front.
- A window saved on a larger display setup no longer aborts the app at launch; windows now open inside their display and under the GPU surface limit.
- TLS dependencies (rustls, rustls-webpki, webpki-roots) updated for published security advisories.
- A panic on a background thread no longer takes down the window through a poisoned lock, and a content read that panics reports failed instead of stopping the worker.
- The web page's reply box no longer clears text you type while a send is in flight; it removes only the text that was sent.
- dispatch's plan review failure after the app reports a run gone now says whether the app dropped the run, never heard of it (another instance or data directory), or could not say, and lists the sessions recorded for the run.
- The `pr` stage refuses a branch holding more commits than its pipeline declares (a stage with `keeps_commit` keeps its own).

## [0.8.1] - 2026-10-26

### Fixed

- A paired device sees the screenshots of an attempt that ran with environment grants again (0.7.3 withheld them, which hid every tester screenshot); the attempt now says it ran with grants, so a client can label them.

## [0.8.0] - 2026-10-26

### Added

- Settings has a "Check for updates automatically" switch (on by default) that turns off the check at launch and the hourly one, and a "What's new" view that shows the changelog, with the newest release found above the running version's notes marked as not installed.

### Fixed

- A ticket's file reaches a device up to 256 KiB and its body up to 64 KiB instead of being cut at 4 KiB; a ticket file over 768 KiB is refused before it is read (evidence files, previously 1 MiB, now share that bound).
- An evidence image up to 700 KiB reaches a device whole instead of being cut at 3 KiB; an image that grew past the limit while being read is refused as too large.

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
