# Working on the live app and the headset

This is the guide for an agent, or a person, working on Switchboard
while the owner's own copy of it is running: rebuilding the Mac app
under them, pushing the Swift client to their Vision Pro, cutting a
release, and the lessons that were paid for once already. `CLAUDE.md`
says how the code is laid out; this says how the running system is
touched without breaking it. Each rule here came from a real mistake or
a real request, and the reason is given so it can be judged, not just
obeyed.

## Rules that bind

- **Kill only by an exact pid, never by pattern.** Every agent Switchboard
  launches carries the bundle path in its command line (through
  `--settings`), so `pkill`/`pgrep -f` on the bundle path matches the
  agents too. A pattern kill once took out the live app, and once killed
  an agent's pane instead of the app. The app's pid is the listener on
  the remote port (43187 by default); check the command before killing:

  ```sh
  PID=$(lsof -nP -iTCP:43187 -sTCP:LISTEN -t | head -1)
  ps -o command= -p "$PID"   # must end in /Switchboard.app/Contents/MacOS/switchboard
  ```

  The executable is lowercase `switchboard`; a `ps` match on
  `Switchboard` finds nothing, and `open` then sees the old app still
  running while a swap looks done.
- **Never send into a live agent session**: not `session.send` on the
  control port, not `tmux send-keys` on the `switchboard` socket, not a
  script `send` line. The owner pastes feedback into a pane themselves,
  or it travels as a Dispatch decision. Keys may go only into panes on a
  `switchboard-test-*` socket.
- **Never resume an agent.** A resume spends a turn; the owner decides.
  If a kill stops an agent, say which one and leave it.
- **Ask questions as prose** in the reply, numbered, each with a
  recommendation. The owner reads sessions through Switchboard's own
  views, where `AskUserQuestion` pickers show nothing and arrow keys do
  not reach them.
- **Branch from `origin/main` after a fetch, never from a local `main`.**
  A branch cut from a `main` fifty commits behind once bundled an app
  whose `SCHEMA_VERSION` refused the data directory, and cost a rebase
  with a renumbered spike.
- **No tool attribution and no client names** anywhere that reaches
  GitHub (`CLAUDE.md` has both rules). Check a PR body for `claude.ai`,
  `Generated with` and `Co-Authored-By` before creating it, and
  `git grep -i` for a client's name before committing docs.
- **Acceptance on the headset is the owner's word.** There is no
  screenshot of the Vision Pro. Say what was installed and what to try;
  do not report a device behaviour as verified until they have.
- **Headset off means hands off.** When the owner says the headset is
  offline, do not install to it or push for it until they say it is
  back. Work that needs no headset goes on.

## The Swift client on the headset

The one-time setup on a Mac: Xcode with the visionOS platform, the
headset in Developer Mode, paired in Xcode's Devices window (the Vision
Pro shows a code, typed on the Mac), and one run from Xcode with the
team. The signing team is `DEVELOPMENT_TEAM` in the repo's git-ignored
`.env`, read by `client/project.yml` when `xcodegen` writes the Xcode
project (`brew install xcodegen`). Nothing in the repo names the team.

- `client/scripts/device.sh run` builds Release, installs and launches on
  the paired headset; `device.sh devices` lists what is paired, with the
  device id the other commands take. `client/scripts/sim.sh build|run|shot
  <file>|log` does the same on the simulator with an empty team.
- `swift test` on the Mac runs the package tests (none of them run in the
  pre-commit hook or CI). A changed initializer signature can leave a
  stale object behind and fail the link with a missing symbol; `swift
  package clean` fixes that.
- **The simulator proves little without a host.** No Switchboard host is
  paired there by default, so no card has a field, no dictation runs,
  no file is served. A crash in the selection binding reached the
  headset because the simulator never opened a card. Pair a throwaway
  host (below) when the change touches a card, or test on the device.
- **Headset asleep:** `device.sh run` still installs, but the launch
  fails with CoreDevice error 3 or hangs past two minutes. Nothing is
  lost: once it is awake, launch alone with
  `xcrun devicectl device process launch --device <udid>
  com.sadburger.switchboard.vision`. A launch cut short can leave the
  app running with no window, and the icon then shows nothing: find it
  with `xcrun devicectl device info processes --device <udid> | grep
  Switchboard.app`, `device process terminate --pid <pid>`, launch again.
- **The device's only diagnostics are files it writes.** The app logs
  window lifecycle to `launch.log` in its container (`LaunchLog.note`):

  ```sh
  xcrun devicectl device copy from --device <udid> \
    --domain-type appDataContainer --domain-identifier com.sadburger.switchboard.vision \
    --source "Library/Application Support/launch.log" --destination launch.log
  ```

  A crash leaves an `.ips` under the system crash logs:

  ```sh
  xcrun devicectl device info files --device <udid> --domain-type systemCrashLogs
  xcrun devicectl device copy from --device <udid> --domain-type systemCrashLogs \
    --source <name>.ips --destination crash.ips
  ```

  The `.ips` is a JSON body after one header line; `procPath` names the
  installed bundle, which tells an old build's crash from a new one.
- Dev aids reach the simulator as environment variables through
  `simctl launch` (`SIMCTL_CHILD_SWITCHBOARD_INVITE` pairs on start,
  `SIMCTL_CHILD_SWITCHBOARD_OPEN` opens windows); `client/README.md`
  lists them. Everything the app opens at launch waits `LaunchGrace`
  (8 s) first, for the reason under visionOS below.
- Which host a session is on matters. The headset pairs with more than
  one Mac. A feature that needs a new host serves nothing from a Mac
  still on the released app; check the session's host before suspecting
  the client.

## Relaunching the Mac app on a branch build

The owner's Mac runs `~/Applications/Switchboard.app`. To put a branch
on it: `./scripts/bundle.sh` (signed with the Developer ID identity, so
Accessibility grants and Keychain ACLs survive; the previous bundle
stays at `.Switchboard.app.previous`), then take the pid as above,
`kill -TERM` it, wait for the listener to go, `open
~/Applications/Switchboard.app`, and confirm a new listener. The tmux
sessions are untouched; the owner's agents keep running. On a Mac that
hosts a Dispatch runner, the runner keeps its old image until restarted
(`dispatch runner restart`, refused while a stage is at a gate; retry,
never force). A cross-check: `strings` on the binary for a message the
branch removed or added says which build is really running.

A release install by hand is the same swap from the release zip:
download, verify the `.sha256` and `codesign --verify`, kill by pid,
move the bundle to `.previous`, `ditto -x -k` the new one in, `open`,
restart the runner.

A `cargo run` binary can check for updates but never install one (it
is not a bundle). A debug binary is unsigned, so the login Keychain
asks on its first read of an item; the prompt blocks the main thread
and the remote port drops every TLS handshake while it is up, which a
device shows as the host offline. Click Always Allow; it recurs after
the next build.

## Releases

The version in `Cargo.toml` is the one source. Every user-visible change
adds `changelog.d/<branch>.md` in the same commit (the hook does not
check this). A release is `./scripts/bump.sh patch|minor|major`, a commit
"Release X.Y.Z" through the hook on a branch, a pull request that merges
it to `main` (`main` takes pull requests only; nothing pushes it
directly, and the merge publishes nothing), then the "Release" workflow,
run by the owner from the Actions tab or `gh workflow run release`. It
runs from `main` only, waits for the owner's approval of the `release`
environment that holds the signing identity and the notarization key,
and then runs `scripts/release.sh`: build, sign, notarize, tag, publish
with the changelog section as notes. An agent prepares a release (the
bump pull request) on the owner's word and never starts the workflow;
the owner does, when ready. `scripts/release.sh` on a Mac is the
fallback and does the same without notarization unless the `NOTARY_*`
variables are set. Minor when the folded section has an `Added:`
heading, patch when it has only `Changed:` and `Fixed:`; the owner wants
this kept as the habit, not written into the scripts.

Every Mac running the app offers the release to its owner (daily check,
same Team ID required, swap helper with rollback). A Mac that is only an
agent box gets changes this way and no other, so a feature the headset
needs from that host is not done until it is released.

## Throwaway hosts and two-host tests

The remote port's end-to-end checks run debug instances on the LAN from
a `SWITCHBOARD_SCRIPT` (`add-project`, `new-shell`, `remote-listen lan
<port>`, `pairing <code>`) with `SWITCHBOARD_DATA_DIR` and
`SWITCHBOARD_TMUX_SOCKET` pointed at test locations; `client/README.md`
has the recipe. What they leave behind and what goes wrong:

- Start every throwaway host with `SWITCHBOARD_CONTROLLER=/dev/cu.none`.
  Otherwise its controller thread blocks in `open()` on the serial port
  the live app holds, the process cannot exit (`?E` in `ps`), and it keeps
  its data-directory lock and its port until the board is unplugged.
  The same uninterruptible open once pinned the live app at quit:
  `sample <pid> 2` showing a thread in `__open` is the sign.
- Each throwaway data directory puts a `remote-key:<id>` item in the
  login Keychain: `security delete-generic-password -s
  com.sadburger.switchboard -a remote-key:<id>`. The Swift CLI's registry
  goes to `~/Library/Application Support/switchboard-remote` unless
  `SWITCHBOARD_REMOTE_DIR` is set.
- Kill instances by the pid from `lsof -nP -iTCP:<port> -sTCP:LISTEN -t`,
  and the `switchboard-test-*` tmux servers.
- Restarting a host with its script re-runs the `new-shell` lines and
  makes duplicate sessions; restart without the script (the listen
  preference and pairing survive in `identity.json`). A consumed pairing
  code needs the `pairing` line again to reopen the window. Uninstalling
  the simulator app drops its Keychain items and registry.
- A conversation mode can be exercised by pointing a throwaway host's
  record at one of the owner's own transcript files (edit the record
  JSON while the host is stopped: `kind: {"Agent":"ClaudeCode"}`,
  `resume: {"ClaudeCode": {session_id, transcript}}`). Real agent runs
  live only in ignored tests.
- Re-pairing a device with the same server id at a new port only puts
  that address first in its `servers.json`.

## visionOS and SwiftUI lessons

- **No window survives a close; a launch opens the Sessions window and
  nothing else.** The owner ruled this after restored windows twice left
  the app alive with nothing to tap. Window groups other than the main
  one are `.restorationBehavior(.disabled)`; do not bring restoration
  back without being asked. Underneath: visionOS tears every window
  down about 7 s after launch when a second window is open during scene
  restoration, which is why anything the app opens at launch waits the
  8 s `LaunchGrace`.
- **An open `Menu` scrolls to its top whenever its body is re-evaluated**,
  and the model's two-second pull re-evaluates everything that reads
  the store. A menu that lists sessions or filters reads a projection
  the model replaces only when it differs (`FilterChoices`,
  `sessionChoices`) and is `.equatable()`.
- **`TextField(text:selection:)` hands back indices into its own copy of
  the text**, which can be longer than the bound text in the same frame;
  `utf16Offset(in:)` on one traps ("String index is out of bounds") and
  the app dies seconds after a card opens. Bounds-check a selection
  against the text before measuring it, and test with a range from a
  longer string.
- A window sized to its content (`.windowResizability(.contentSize)`)
  must hold no `maxWidth: .infinity` view, or it takes the default width.
  Bordered buttons pad their labels by about 24 points, which squeezes
  whatever shares the row. `WindowPlacement(.utilityPanel)` is the
  virtual keyboard's spot: low, close, angled.
- A popover dismisses on any tap outside it. Something that must stay
  open beside a window while it is used is an **ornament** attached to
  the window's edge; it moves with the window and needs its own
  `.glassBackgroundEffect()`.
- A `ScrollView` clips what leaves its content box, which is exactly
  where a dragged tile goes; a drag-and-drop canvas uses a scroll view
  only when it must scroll. A drag's landing slot should follow the
  finger's location in a named coordinate space, not the moved view's
  centre.
- A `LazyVStack` drops the views it is not showing and their visibility
  reports with them, so a marker that says "the end is on screen" must
  sit outside it. A view that follows the end opens from the bottom with
  `.defaultScrollAnchor(.bottom)`, and following is a mode (scrolling
  off, a button releases it), not a guess from marker visibility.
- visionOS gives an app no camera (main-camera access is an
  enterprise-only entitlement), so pairing on the headset is by typed
  code: the client resolves the host's Bonjour record, reads `/info`
  under no trust, shows the key's short fingerprint for the owner to
  compare with the Mac's dialog, and pins that key for the pairing.
- A Bonjour resolve through `NWConnection` can sit in `.waiting` for
  ever; a reconnect loop that resolves must time it out (5 s) and count
  `.waiting` as a failure, or the device says "reconnecting (1)" until
  relaunched.
- A SwiftUI view name must not collide with a wire type (`TicketFileView`
  is the wire type; the page is `TicketFilePage`). `await` is not allowed
  inside an `XCTAssert*` autoclosure; bind first. `print` into a
  redirected file is fully buffered; a long-running CLI needs
  `setvbuf(stdout, nil, _IOLBF, 0)`.
- The headset has no keyboard extensions and no input accessory view;
  a panel of keys is a window of the app's own. A `Toggle` with
  `.toggleStyle(.button)` is the idiomatic on/off button. A board of
  several windows is impossible (nothing glues windows or moves them as
  a group), so a board is one window with tiles inside it, and the
  layout is the client's, not the host's working set.
- The base text sizes on the headset are three quarters of the system
  sizes, which the owner found right; cards open in portrait, 580 by
  900 points.

## Rust-side and tooling lessons

- Keep the per-worktree `target` directory (`.cargo/config.toml` in a
  Dispatch worktree). Cargo names workspace-member artifacts without the
  source path and judges freshness by mtime, so a shared target lets a
  sibling worktree's newer build pass for yours and you compile against
  its API. Never export `CARGO_TARGET_DIR` for an agent.
- A test that reads a sandbox `deny(1)` line back through `log show`
  fails about half the time: the kernel reports only about half of the
  denies, and reported ones expire within a minute (issue #182). The
  suite asserts only the refusal; a failure on the deny line is the
  environment, not `confine.rs`.
- A git dependency that moved to an ssh URL needs
  `CARGO_NET_GIT_FETCH_WITH_CLI=true cargo fetch` once under a sandbox
  where libgit2 cannot use the `insteadOf` rewrite.
- CI: the apt step is `scripts/ci-apt.sh` with bounded, retried calls,
  because an unbounded `apt-get update` once hung 86 minutes; a cold
  Rust cache makes the macOS job 20 minutes instead of 5, so caches are
  saved from `main` only.
- A pre-commit hook run takes about five minutes (fmt, clippy pedantic,
  the workspace tests, the vendored terminal's tests); run commits in
  the background and keep working. A commit that stages only files
  under `client/` runs `swift test` instead, in seconds and with no
  Rust build, which is what lets headset work go on from a Mac whose
  `target/` is cold. `--no-verify` is never used. The
  Python-script edits in this repo's history assert each anchor occurs
  exactly once and write only at the end, so a failed assert changes
  nothing.
- zsh: no `timeout` command (use `gtimeout` or a loop); an unquoted
  glob that matches nothing aborts the command (`--include='*.swift'`
  quoted); `"$ONE:screen"` is a history modifier (`"${ONE}:screen"`);
  `echo ====` fails on `=cmd` expansion (quote it); `cd` persists
  between commands in one shell.

## Claude Code launch gotchas

Measured while building Dispatch's live test, each one looked like a
hang in the agent pane:

- A fresh git repository is its own workspace for the folder-trust
  dialog even under a trusted parent, so every new worktree asks once;
  a plain directory inherits the trust. Switchboard never writes
  `~/.claude.json`.
- `--add-dir` and `--allowedTools` are variadic and swallow a following
  positional prompt (the pane sits idle at 0 tokens, or shows a trust
  dialog for a "directory" that is the prompt text). Switchboard inserts
  `--` before a first prompt.
- `--add-dir` alone still asks before creating a file there; only an
  `Edit(//abs/dir/**)` allow rule writes unattended, a `Write(...)` rule
  is ignored.

When an unattended launch stalls, capture the test tmux pane first.
