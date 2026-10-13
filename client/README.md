# Switchboard client

The Swift half of `docs/remote-client.md`: the packages the visionOS
and iPadOS apps are built from, and a command-line browser for the Mac
that exercises them against real hosts. Builds with Xcode 26 for
macOS 14, iOS 17 and visionOS 1.

```sh
swift build                      # every target
swift test                       # 29 tests: fixtures, pinning, the connection core, events, watches
swift run switchboard-remote     # the command-line browser (usage below)
scripts/sim.sh run               # the visionOS app on the simulator (below)
```

## Modules (§7 of the spec)

- `SwitchboardProtocol`: the wire views as `Codable` with the host's
  own snake_case keys spelled out, the reply envelope, `/info` and
  `/pair`, the QR invite, and qualified refs (`SessionRef`,
  `WorkingSetRef`, `FileRef`). `Fixtures/*.json` are written by the
  Rust port's test (`SWITCHBOARD_WRITE_FIXTURES=1 cargo test --test
  remote fixtures`) and decoded by both sides' tests.
- `SwitchboardTransport`: `HostClient` over `URLSession` with
  `PinnedSessionDelegate`, which accepts a certificate only when the
  SHA-256 of its P-256 `SubjectPublicKeyInfo` is the pinned one
  (`.expect`) or records what it saw for an unpaired `/info`
  (`.observe`); the events socket (`URLSessionWebSocketTask`) shares
  the session and so the pin; `Discovery` browses `_switchboard._tcp`
  with `Network.framework` and resolves a record to an address.
- `SwitchboardClientCore`: `KnownServer` and the registry, the
  credential stores (`KeychainCredentials` for the apps,
  `FileCredentials` for the command line, memory for tests), the
  `ServerConnection` actor (the §6.2 states, backoff, subscribe then
  snapshot with the buffered events reconciled against each resource's
  held revision, live events refetching only past it, the epoch check,
  `resync_required` and a closed socket reconnecting, a key change
  dropping the token and keeping the entry, reference-counted screen
  watches sent at the first holder and released at the last, resent
  after a reconnect before the snapshot, with the screen read on
  `changed` past its held revision), and `ConnectionManager`
  with the unified store, filters and the cross-host order; its loop
  only retries hosts that are not connected.
- `SwitchboardUI`: `RootView` (the main window's tabs), `BrowserView`,
  `SessionCardView`, `SourceLabel` and `ServersView` in SwiftUI, over a
  `BrowserModel`; nothing visionOS-only, so the iPad shell draws the
  same views.
- `Apps/SwitchboardVision`: the visionOS app, an entry that declares
  the scenes and where the registry lives (`servers.json` in the app's
  container, the token and pin in the Keychain, nothing from a host on
  disk).

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

Bonjour on visionOS needs `NSBonjourServices` and
`NSLocalNetworkUsageDescription` in `Info.plist`; both are there. The
headset itself needs the one-time Xcode setup: Developer Mode on the
device, the device paired in Xcode's Devices window, and a development
team chosen for the target.

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
