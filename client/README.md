# Switchboard client

The Swift half of `docs/remote-client.md`: the packages the visionOS
and iPadOS apps are built from, and a command-line browser for the Mac
that exercises them against real hosts. Builds with Xcode 26 for
macOS 14, iOS 17 and visionOS 1.

```sh
swift build                      # every target
swift test                       # 25 tests: fixtures, pinning, the connection core, events
swift run switchboard-remote     # the command-line browser (usage below)
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
  dropping the token and keeping the entry), and `ConnectionManager`
  with the unified store, filters and the cross-host order; its loop
  only retries hosts that are not connected.
- `SwitchboardUI`: `BrowserView`, `SessionCardView`, `SourceLabel` and
  `ServersView` in SwiftUI, over a `BrowserModel`.

## The command-line browser

```sh
switchboard-remote discover [seconds]     what advertises on this network
switchboard-remote pair '<invite json>'   the text of the host's QR code
switchboard-remote pair "$(pbpaste)"      the same, after "Copy invite" in the host's dialog
switchboard-remote pair <host:port> <code> <spki_sha256>
switchboard-remote hosts                  the known hosts and their state
switchboard-remote forget <server id>
switchboard-remote watch [seconds]        the unified view, refreshed on a timer
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
