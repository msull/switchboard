# Switchboard client

The Swift half of `docs/remote-client.md`: the packages the visionOS
and iPadOS apps are built from, and a command-line browser for the Mac
that exercises them against real hosts. Builds with Xcode 26 for
macOS 15, iOS 18 and visionOS 2 (the packages); the visionOS app itself
targets visionOS 26.

```sh
swift build                      # every target
swift test                       # 72 tests: fixtures, commands, pinning, the connection core, events, watches, conversations, the draft, presentation, browsing
swift run switchboard-remote     # the command-line browser (usage below)
scripts/sim.sh run               # the visionOS app on the simulator (below)
```

## Modules (§7 of the spec)

- `SwitchboardProtocol`: the wire views as `Codable` with the host's
  own snake_case keys spelled out, the reply envelope, `/info` and
  `/pair`, the QR invite, the content replies (`ConversationBody` with
  its turns and their activity, `LastResponseBody`, `RunsBody`,
  `RunOutputBody`, `FileBody`), and qualified refs (`SessionRef`,
  `WorkingSetRef`, `FileRef`, `RunRef`). `Fixtures/*.json` are written by the
  Rust port's test (`SWITCHBOARD_WRITE_FIXTURES=1 cargo test --test
  remote fixtures`) and decoded by both sides' tests.
- `SwitchboardTransport`: `HostClient` over `URLSession` with
  `PinnedSessionDelegate`, which accepts a certificate only when the
  SHA-256 of its P-256 `SubjectPublicKeyInfo` is the pinned one
  (`.expect`) or records what it saw for an unpaired `/info`
  (`.observe`), and a failed request is a key mismatch only when the
  delegate refused a certificate under another key, never when the
  request was merely cut short (a host relaunching, a task cancelled),
  which is a transport failure to retry; the events socket (`URLSessionWebSocketTask`) shares
  the session and so the pin; `Discovery` browses `_switchboard._tcp`
  with `Network.framework` and resolves a record to an address.
- `SwitchboardClientCore`: `KnownServer` and the registry, the
  credential stores (`KeychainCredentials` for the apps,
  `FileCredentials` for the command line, memory for tests), the
  `ServerConnection` actor (the §6.2 states, backoff, subscribe then
  snapshot with the buffered events reconciled against each resource's
  held revision, live events refetching only past it, the epoch check,
  a frame with no snapshot to reconcile against fetching everything
  rather than seeding the list from one resource, a replaced
  subscription's late end ignored, `resync_required` and a closed
  socket reconnecting, a key change
  dropping the token and keeping the entry, reference-counted content
  watches by resource key sent at the first holder and released at
  the last, resent after a reconnect before the snapshot, with what is
  held on a key re-read on `changed` past its held revision: the
  screen, the last response and the conversation from its last turn,
  a run's output, a pinned file (`watch(file:)`, `readFile`); `ConversationState`, the pages merged by turn;
  `command`, a send, resume or trust under the control scope with a
  UUID op per command, and on a timeout the op's status rather than a
  resend: done takes the logged reply, interrupted is reported as
  started, unknown goes once more with the same op; the draft, a
  session's prompt as the host's Prompt Box editor holds it, read on
  `changed {draft}` without a watch, replaced whole, worked with the
  editor's tools, dictated into as speech events, and sent under an
  op as a command is),
  and `ConnectionManager`
  with the unified store, filters and the cross-host order; its loop
  only retries hosts that are not connected.
- `SwitchboardUI`: `RootView` (the main window's tabs), `BrowserView`,
  `SessionCardView`, `SourceLabel` and `ServersView` (known and
  discovered hosts; a discovered one pairs through `PairSheet`, which
  shows the key it presents for the owner to check against the host's
  dialog and takes the typed code) in SwiftUI, over a
  `BrowserModel` that mirrors the manager's store when it changes;
  `CardWindowView`, one window on one session (a click on a listed
  session brings its open window forward; the context menu opens
  another) with the `compact`, `screen`, `last_response` (the answer
  with its inline Markdown, its tables cut out by `MarkdownTable` and
  laid out as grids), `conversation` (turns with the activity
  folded, each tool call's text whole on demand, earlier pages on
  demand, the end following the host while any of a tall marker after
  the last turn is on screen, so a bounce leaves the view following,
  the end staying in view when the field under it grows, and an
  ellipsis button at the foot of the turns, shown only off the end,
  that takes the view back to following it; the window keeps its place,
  `ConversationPlace`: the turn at the top or the end it follows and
  the folds open, so a page merging under the view, a reconnect or a
  relaunch puts it back where it was, and another transcript starts
  it over at the end) and
  `run_output` (a run picker and the tail) modes (the screen holds a watch while it is on screen, and
  the text of every mode but the card scales through a readout in the mode row that opens a
  slider and the snap sizes, `ScreenScale`, as state of the window:
  kept across a mode switch, gone when it closes), a small header (the
  host pill and an icon-only Sessions button) and mode pills that
  turn into icons rather than wrap, so a tall narrow card fits, and a
  banner when the host is away or the session was removed; on a
  Claude session a control bar (`ControlBar`): the prompt field with
  Send and a microphone button that starts dictation at once through
  the Speech framework (`Dictation`: on-device recognition where the
  device has it; what is heard is kept as utterances, `Utterances`,
  the finished ones and the live one revised in place, since the
  recognizer starts over after a pause and would otherwise replace the
  earlier phrase, and closes one by sending its words once more,
  re-timed; the system keyboard's own mic still works; visionOS gives
  an app no gaze or dwell event and "look to dictate" belongs to
  search fields only, so the button is a tap). On a host that keeps a
  draft (`/info` lists `draft`) the field is the host's Prompt Box
  editor: the draft is held while the bar is up, typing replaces its
  text once the typing pauses (the host's reply is the text, a newer
  edit outranks it), dictation goes to it as the editor's speech
  events in order (`Dictation.onEvents`, one recognizer session per
  run), the editor's tools sit in a row over the field at its left,
  away from Send (`DraftTools`: undo,
  redo, the last sentence deleted, clear, the AI pass where the host
  has a key) and Send takes the draft to the pane under an op; one
  reserved line under the field carries a note, a degraded editor or
  the AI pass running, and says nothing of listening, so nothing
  shifts. On an
  older host the field is the device's own, what is heard goes after
  what was typed, and a hand edit while listening keeps the field's
  text with the live utterance going on after it; the prompt window
  (`PromptWindowView`, one per app like the main window, opened from
  a card's header or the Sessions toolbar): the whole draft of the
  session whose input is active (`BrowserModel.activeInput`, the last
  one a control bar typed, dictated or used a tool in), large and
  scaled (`ScaleControl`), the live utterance in amber, with a
  microphone, the tools and Send of its own, placed once and left
  where it is while the cards come and go, as the desktop preview is
  used with the Working Set; it is a view of the host's editor and
  changes nothing else; "Trust
  this folder" when the host says the question is up, and Resume for a
  session that is not running, behind "Resume spends a turn"; a read
  device sees why the bar is off instead; the
  presentation model (`ViewMode`, `CardWindowRequest`, `CardStanding`,
  `BrowseMode`, `FocusCursor`) and `PresentationState` in a JSON file
  in the app's container: the layout (list, grid, or one session in
  focus), the filters (hosts, kinds, states), the focused session, each
  and each card window's mode by window id. A launch opens the main
  window and nothing else: the card and prompt window groups are not
  restored, and the main one is declared as presented, so the app
  always comes back to the browser however it was left. The first
  seconds of a launch belong to the system's scene restoration, and a
  window the app opens before it is done is torn down with every other
  window when it finishes (6.8 s after launch on the visionOS 26
  simulator, the app left running with no window at all), so the dev
  aids that open windows at launch wait `LaunchGrace` (8 s). A card's
  header Sessions button brings the main window forward at any time
  (`MainWindow`). Nothing visionOS-only, so the iPad shell draws the
  same views.
- `Apps/SwitchboardVision`: the visionOS app, an entry that declares
  the scenes and where the registry lives (`servers.json` in the app's
  container, the token and pin in the Keychain, nothing from a host on
  disk), and its icon: `Assets.xcassets/AppIcon.solidimagestack`, the
  Mac icon split into the three layers visionOS lights in parallax
  (background, faceplate with jacks, cords), rendered from
  `assets/vision/*.svg` by the repo's `scripts/icon.sh`.

## The Xcode project

Swift Package Manager builds the libraries and the command-line
browser; an app for the headset needs an Xcode app target, which
`project.yml` describes and `xcodegen generate` writes as the
git-ignored `SwitchboardClient.xcodeproj` (`brew install xcodegen`).
The project references the package beside it, so every view and test
stays in the package.

`scripts/sim.sh` drives the simulator from the terminal: `build`
generates and builds, `run` boots the simulator, installs and launches,
`shot <file>` screenshots it, `log` streams the app's log. The
simulator shares the Mac's network, so it discovers and pairs with a
Switchboard running here. A dev aid skips the typing: launched with
`SWITCHBOARD_INVITE` set to an invite's JSON (the QR code's text) the
app pairs with it on start, which `simctl launch` passes as
`SIMCTL_CHILD_SWITCHBOARD_INVITE`:

```sh
INVITE=$(pbpaste)   # after "Copy invite" in the host's Remote devices dialog
SIMCTL_CHILD_SWITCHBOARD_INVITE="$INVITE" xcrun simctl launch --terminate-running-process \
  "Apple Vision Pro" com.sadburger.switchboard.vision
```

`SWITCHBOARD_INVITE` may hold several invites, one per line, for two
hosts at once. A second dev aid, `SWITCHBOARD_OPEN`, lists card windows to open once
their sessions are listed, as `<record id>[:<mode>]` entries separated
by commas (`SIMCTL_CHILD_SWITCHBOARD_OPEN="$ID:screen"`); the entry
`prompt` opens the prompt window, `prompt:<seconds>` after a wait. A
third, `SWITCHBOARD_CLOSE_WINDOWS_AFTER=<seconds>`, closes every window
that long after launch, once per process: the state the app is in when
its last window is closed by hand, so that a `simctl launch` of the
running app then stands in for a tap on its icon. The launch
variables are read by the process, so an activation of a running app
sees the ones it was launched with; a `prompt` entry without a wait
opens inside the launch grace, which is how the window teardown above
was measured.

The headset gives the Mac no log, so the app keeps one of its own:
`launch.log` in its container records what the windows did since
launch (appeared, disappeared, scene phases), under 64 KiB, window
ids and events only. Copy it off with

```sh
xcrun devicectl device copy from --device <udid> --domain-type appDataContainer \
  --domain-identifier com.sadburger.switchboard.vision \
  --source "Library/Application Support/launch.log" --destination launch.log
```

Bonjour on visionOS needs `NSBonjourServices` and
`NSLocalNetworkUsageDescription` in `Info.plist`; both are there. The
headset itself needs the one-time Xcode setup: Developer Mode on the
device (Settings, Privacy & Security), the device paired in Xcode's
Devices window (with the headset on its Remote Devices settings
screen), and the app run from Xcode once with a development team on
the target so the device trusts the developer. After that
`scripts/device.sh run` builds (Release, unless
`SWITCHBOARD_CONFIGURATION=Debug`), installs and launches on the
headset from the terminal, and `devices` lists what the Mac is paired
with.
The team comes from `DEVELOPMENT_TEAM` in the environment or in the
repo's git-ignored `.env`; `project.yml` reads it at generation, so a
project generated without one (as for the simulator) has no team.

## The command-line browser

```sh
switchboard-remote discover [seconds]     what advertises on this network
switchboard-remote pair '<invite json>'   the text of the host's QR code
switchboard-remote pair "$(pbpaste)"      the same, after "Copy invite" in the host's dialog
switchboard-remote pair <host:port> <code> <spki_sha256>
switchboard-remote hosts                  the known hosts and their state
switchboard-remote forget <server id>
switchboard-remote watch [seconds]        the unified view, refreshed on a timer
switchboard-remote screen <session id>    a session's pane, live through a watch
```

The registry and credentials live under
`~/Library/Application Support/switchboard-remote`, or wherever
`SWITCHBOARD_REMOTE_DIR` points; set it for a throwaway run, since
Application Support ignores `HOME`.

Two throwaway hosts on one Mac, for an end-to-end run (each opens a
window; the Keychain gets a `remote-key:<id>` item per data directory,
which `security delete-generic-password -s com.sadburger.switchboard -a
remote-key:<id>` removes afterwards):

```sh
printf 'add-project alpha /tmp/pa\nnew-shell alpha one\nremote-listen lan 43190\npairing ABCD2345\n' > /tmp/a.script
SWITCHBOARD_DATA_DIR=/tmp/sba SWITCHBOARD_TMUX_SOCKET=switchboard-test-a \
  SWITCHBOARD_REMOTE_PORT=43190 SWITCHBOARD_SCRIPT=/tmp/a.script cargo run --locked
# the same with another data directory, socket, port and code
switchboard-remote discover
switchboard-remote pair 127.0.0.1:43190 ABCD2345 "$(curl -sk https://127.0.0.1:43190/api/v1/info | jq -r .body.spki_sha256)"
switchboard-remote watch
```
