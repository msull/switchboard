# Investigation notes: issue #230, a web control surface on the remote port

Worktree: `/Users/sbgragent/.dispatch/worktrees/da9387bb`, branch
`dispatch/230-a-web-control-surface-the-remote-port-se` at `7fe8949`
(Release 0.4.1). Line numbers below are from this tree.

## What is on disk, and what is rehydrated

Nothing new has to be durable on the host for this feature. The records
that exist already carry everything a browser page needs:

- **The device record** is `devices.json` in the data directory,
  `PairedDevice` at `src/ports/remote.rs:97-109`: id, the name the device
  gave itself at `/pair`, scope, the token's SHA-256 (never the token),
  created, last seen. Loaded once at start by `IdentityLoaded`
  (`src/core/remote.rs:23-30`), saved through `Effect::SaveDevices`
  (`src/core/remote.rs:127-131`) on `Paired`, `RevokeDevice`,
  `SetDeviceScope` and a stale `DeviceSeen` (every five minutes at most,
  `LAST_SEEN_SAVE_INTERVAL_SECS` at `src/core/remote.rs:18`). A browser
  that pairs is one more row here, with whatever `device_name` the page
  posts; the host does not lift `User-Agent` out of the headers
  (`read_request`, `src/adapters/remote.rs:487-494`, keeps only
  `content-length`, `authorization`, `upgrade` and `sec-websocket-key`), so
  "Safari on iPhone" is a name the page derives from `navigator.userAgent`
  client-side, or the adapter gains a field.
- **The host's identity** is `identity.json`, `Identity` at
  `src/ports/remote.rs:25-43`: server id, name, the certificate as DER
  base64, its SPKI hash, and `listen {lan, port}`. The certificate the
  browser will be warned about is this field; a "download the
  certificate" endpoint would serve `certificate_der`
  (`src/adapters/remote.rs:130-134`). The certificate is made at
  `make_identity` (`src/adapters/remote.rs:95-127`) with
  `CertificateParams::new(Vec::<String>::new())`: **no subject
  alternative name at all**, a common name of `Switchboard <server id>`,
  and rcgen's default validity (the doc comment says to the year 4096).
  The private key is the Keychain item `remote-key:<server id>`.
- **The pairing window** is never saved: `PairingWindow {until, code,
  scope, failures}` in the core, opened by `OpenPairing`
  (`src/core/remote.rs:45-53`), cleared by `ClosePairing`, `Paired`, three
  `PairingFailed`s, or `Tick` past `until`. The code is made in the UI
  (`remote::pairing_code()`, `src/ui/remote.rs:239`) or by the script line
  `pairing <code>` (`src/script.rs:730`).
- **The page itself** is proposed as `include_str!` in the binary; there is
  no `web/` directory and no `include_str!` under `src/` today (checked).
  Rehydration of the page is therefore a rebuild, which is also why the
  issue asks for no caching across versions; `/info` already carries
  `version` (`src/app.rs:3582`) the page could key a cache-bust on.
- **The browser's own state** (token, pinned key hash, server id) lives on
  the device, as the native client's does in its Keychain (spec §7). The
  host holds no mirror of it. What the host can answer about it is only
  `/info` (unauthenticated) and 401 `unauthorized` from `authenticate`
  (`src/app.rs:3119-3148`), which gives the same code and message for a
  never-paired token and a revoked one ("the token is not known; pair
  again", `src/app.rs:3142-3147`); the page tells "revoked" from "never
  paired" only by remembering that it had a token.

The on-disk shape is therefore unchanged by this issue unless the plan
chooses to record something new on the device record (for example that
it is a browser, or a cookie's expiry), which would touch
`src/ports/remote.rs` additively; `devices.json` has no schema version
field of its own, and `IDENTITY_SCHEMA_VERSION` (`src/ports/remote.rs:17`)
covers only `identity.json`.

## The code the issue touches

### The listener and the connection loop, `src/adapters/remote.rs`

- `read_request` (444-514): request line, headers lowercased into a
  vec, `MAX_HEAD` 8 KiB (437), `MAX_BODY` 64 KiB (438), the query split
  undecoded into `(name, value)` pairs (495-503; no percent-decoding, so a
  URL-encoded `?code=` value arrives as typed, and `param` at 377 returns
  the first match). `RemoteRequest` (360-372) carries `method`, `path`,
  `query`, `body`, `bearer`, `upgrade`. Any header the page relies on
  (`Cookie`, `Sec-WebSocket-Protocol`, `Origin`, `User-Agent`,
  `Accept`) is read into `headers` and dropped before the struct is built.
- `respond` (530-552): one write, status reasons for 200/400/401/403/404/
  405/413/429/503/504 only, and **always `Content-Type: application/json`**
  with `Content-Length` and `Connection: keep-alive`. No other header can
  be sent today: a static page with `text/html`, a CSP, `Cache-Control`,
  `Set-Cookie` or `Sec-WebSocket-Protocol` needs `RemoteReply` (389-394:
  status, body, authenticated) and `respond` to carry headers.
- `respond_upgrade` (519-526): writes the 101 with `Upgrade`,
  `Connection` and `Sec-WebSocket-Accept` only; `tungstenite::accept` is
  not used because the request was already read, and
  `from_raw_socket` wraps the stream afterwards (886-887).
- `connection` (770-867): 5 s handshake and first-request-line deadline
  (`HANDSHAKE_TIMEOUT`, 570), 30 s idle between requests on a kept
  connection (573), 10 s writes (574), one request in flight, the
  `/pair` and `/info` rate windows checked by exact path
  (`Limits::limited`, 631-638, against `PAIR_PATH`/`INFO_PATH` at 586-587,
  6 and 60 per minute per listener), 503 `busy` on a full inbox of 64
  (581, 819-824), and the pool move: a reply with `authenticated: true`
  moves the connection from the unauthenticated pool (4,
  `MAX_UNAUTHENTICATED` 567) to the paired one (12, 568) at 835-849. A
  stranger past four is dropped at accept (699-706). A page load before
  pairing counts against the four, and a browser opens several
  connections per origin for a page and its files.
- `subscriber` (878-923): the WebSocket loop, non-blocking, 256-deep queue
  (409), `resync_required` then close on overflow, the device's text
  frames (`watch`, `unwatch`) handed to the frame as `SubscriberFrame`.
- `local_address` (962-971): the one outward-route IP the QR's
  `addresses` holds; `None` with no network.
- `pairing_code` (325), `bearer_token` (337), `token_sha256` (345),
  `CODE_ALPHABET`/`CODE_LENGTH` (319-321).

### The route table and the handshake, `src/app.rs`

- `serve_pending` (1572-1612): drains at most `DRAIN_PER_PASS` (8)
  remote requests per pass after the control socket's, then the
  subscriber frames, then the worker's observations.
- `serve_remote` (3017-3117): **the prefix match is the allowlist**:
  `strip_prefix("/api/v1/")` at 3023, anything else refused 404
  `not_found` at 3024 through `refuse` (3754-3772), which logs every
  refusal at `info` by method, path and code. So a browser's `GET /`,
  `/favicon.ico`, `/apple-touch-icon.png` each log a line today. `/info`
  and `/pair` at 3028-3029 are the only unauthenticated routes;
  `authenticate` at 3031 reads `request.bearer` only; `events` at
  3035-3037 goes to `remote_subscribe` (2723-2760), which requires
  `request.upgrade` and pushes a `Subscriber {id, queue, device, all}`;
  the content and command routes follow (3043-3101).
- `authenticate` (3119-3148): 401 `unauthorized` with "pair this device
  first" when there is no bearer, "the token is not known; pair again"
  when the hash matches no device; a match dispatches `DeviceSeen`.
- `remote_info` (3568-3604): the unauthenticated identity reply:
  `server_id`, `server_name`, `protocol` (`REMOTE_PROTOCOL` 1 at 4252),
  `capabilities` (a fixed list at 3580: read, control, draft, scope,
  tickets, evidence, create), `spki_sha256`, `version`, `limits`,
  `pairing_open`. A `web` capability, if the plan wants one, goes here.
- `remote_pair` (3606-3674): refuses `pairing_closed` (403) outside the
  window, `no_identity` (503), `bad_request` (400) for non-JSON, then
  compares `spki_sha256` (3637, `wrong_key`, counted) and the code
  uppercased (3647, `wrong_code`, counted), takes `device_name` with
  "Device" as the default (3650-3655), makes the token, builds the device
  through `AppCore::new_device` (`src/core/remote.rs:166-175`, name cut
  at 64 chars, scope from the window), logs the name and scope, dispatches
  `Paired`, and answers `{token, scope, server_id, device_id}` in the
  envelope with `authenticated: false` (3673): **the pairing connection
  itself stays in the unauthenticated pool**. The token is in the JSON
  body; nothing sets a cookie.
- `envelope` (3730-3741) and `error_json` (3743-3751): every reply and
  every refusal carries `epoch` and `revision`.
- `dispatch_inner` (480-497): `RevokeDevice` drops the device's
  subscribers, which closes their sockets; the revoked device's next HTTP
  request then fails in `authenticate`.
- `bind_remote` (2936-2975): binds `0.0.0.0` when `listen.lan`, else
  `127.0.0.1` (2962-2966), port from `SWITCHBOARD_REMOTE_PORT` or the
  identity; `remote_addr` (3010) is what the dialog shows as the port.
  With `lan` off a phone cannot reach the port at all, and the pairing
  window still prints the LAN address (`src/ui/remote.rs:150-153`).

### The pairing window, `src/ui/remote.rs`

- `pairing` (131-247): the payload is built at 154-160 as JSON
  `{server_id, spki_sha256, code, addresses: [ "<ip>:<port>" ]}`, QR'd by
  `qr_texture` (280-300, `qrcode` 0.14 at `Cargo.toml:70`, 4 px per
  module, quiet zone 4), shown beside the spaced code (`spaced`, 313),
  the short key (`short_hash`, 303-311, first sixteen hex digits in
  fours), the address, the scope sentence (182-187), "Copy invite" (191-
  197, copies the same JSON) and "Stop pairing". The texture is remade
  when the code changes (161-167). A URL payload is longer than the JSON
  (the hex hash is 64 chars either way; the scheme, host and parameter
  names add ~40), which grows the QR version; the dialog is 620 wide
  (`DIALOG_WIDTH`, 39).
- `host` (82-129): the "Allow devices on the local network" checkbox and
  the port field; the hover text at 105 already states the rule "only
  paired devices get past /info".
- `devices` (250-278): the device grid with scope combo and Revoke; the
  redaction sentence is at 60-66 of `show`. The issue's "the click-through
  is the trust step, said in the paired-devices dialog" lands in this
  dialog.
- The UI test is `remote_devices_dialog_opens_pairing_and_revokes_a_device`
  at `tests/ui.rs:7424`, which finds the spaced code by label (7467-7469)
  and does not read the payload; `DrawCtx::remote_port` (`src/ui/mod.rs:401`,
  filled at 420) is what the invite's port comes from.

### The core, `src/core/remote.rs`

- The transitions listed under "on disk" above (21-116); `pairing_at`
  (147-150) is the window while open; `device_by_token` (138-141);
  `new_device` (166-175). `PAIRING_WINDOW` and `PAIRING_ATTEMPTS` come
  from `core/action.rs`. Nothing here knows a device kind.

### The native text path that must keep working

- `client/Sources/SwitchboardProtocol/Info.swift:93-116`: `PairingInvite`
  is `Decodable` from JSON only; `parse` (113-115) is `Wire.decode` of the
  text. `addresses` are `host:port` strings.
- `client/Sources/SwitchboardTransport/HostClient.swift:15-24`:
  `Address.init?(_ text:)` splits at the last colon and parses the port
  as `UInt16`; a URL (`https://host:port/?code=...`) fails it, so a URL
  invite needs its own parser before `Address` sees the host.
- Consumers of `PairingInvite.parse`: the CLI `switchboard-remote pair`
  (`client/Sources/switchboard-remote/main.swift:109-125`; the usage text
  at 12 says "invite json"), the headset's paste field
  (`client/Sources/SwitchboardUI/BrowserView.swift:849, 883`), the launch
  dev aid `SWITCHBOARD_INVITE` (`BrowserView.swift:93-107`,
  `client/Apps/SwitchboardVision/SwitchboardVisionApp.swift:56-61`), and
  the Swift test `testInviteParsesTheQRText`
  (`client/Tests/SwitchboardProtocolTests/FixturesTests.swift:157-165`).
  `ConnectionManager.pair(invite:)` (`ConnectionManager.swift:482-487`)
  takes the first parseable address and pins the invite's key;
  `ServerConnection.pair` (`ServerConnection.swift:215-240`) checks the
  reply's `server_id` against the entry.
- README dev aids at `README.md:267-279` describe "Copy invite" as the
  QR's text and `switchboard-remote pair "$(pbpaste)"`; the script lines
  `remote-listen` and `pairing` are at `src/script.rs:729-746`.

### Tests and fixtures

- `tests/remote.rs`: `Port` (36-48) is the app with fakes on a free
  loopback port; `http` (245-297) is the host's own HTTP client over the
  pinned `rustls` config, writing a raw request with `Host: localhost` and
  reading **only the status line and `Content-Length`**: response headers
  are discarded, so header assertions (content type, CSP, cookies) need
  the helper extended or a sibling. `call` (299-325) pumps `serve_pending`
  while the client thread waits. `pair` (406-424) opens the window and
  posts `/pair` with the host's own key. The allowlist test is
  `the_allowlist_refuses_everything_else_with_no_side_effect` (682); the
  unpaired test is `info_is_served_to_anyone_and_every_other_read_is_refused_unpaired`
  (426); `pair_is_rate_limited_per_listener` (1406); `revoke_closes_the_devices_socket`
  (2892); the events helpers `subscribe`/`subscribe_at` (1955-1960) open
  the socket with `tungstenite` over the pinned stream with the bearer
  header, which a browser cannot do.
- Fixtures: `client/Fixtures/*.json`, written by `write_fixtures`
  (1587) under `SWITCHBOARD_WRITE_FIXTURES=1` and checked by
  `the_client_fixtures_decode_on_both_sides` (1829); `pair.json` carries a
  placeholder token. No fixture holds the invite; the Swift test at
  `FixturesTests.swift:157` inlines the JSON.

### Spec and status text the plan amends

- `docs/remote-client.md` §4.1 bounds (124-131), §4.3 table (159-194,
  "Everything else on the wire is refused before translation, by an
  allowlist"), §5 (302-319: "Require pairing before anything but `/info`
  and `/pair`" at 306; the token "sent only as a bearer header over the
  pinned connection, never in a URL" at 312; the QR payload at 309), §9
  (393-395).
- `docs/design.md` "Trust boundary" (219-266; the remote sentence at
  249-253) and the "Remote client (2026-10-12)" status (3107-3583; the
  pairing-by-typed-code entry at 3564-3573 and "Known gaps" at 3574-3583).
- `README.md` Layout lines for `src/adapters/remote.rs` (383), `src/app.rs`
  (401), `src/ui/remote.rs` (423), `tests/remote.rs` (437), `client/` (446),
  and the data directory lines (78-83); `CHANGELOG.md` Unreleased is
  empty.

## Findings against the issue's shape

- **The card's one-line excerpt is not on the wire.** `SessionView`
  (`control/src/lib.rs:637-688`) has `card` (the state word), `waiting`,
  `waiting_reason`, `kind`, `liveness`, `last_stop_at_ms`,
  `last_activity_ms`, `trust_question`, `prompt_refusal`, `pending`, but no
  excerpt; spec §4.3 (207) speaks of a `last_message` field that does not
  exist in `control/src/lib.rs` or `src/core/model.rs` (grep: none). The
  Mac's card gets it from the parsed transcript (`agent_excerpt`,
  `src/ui/cards.rs:558`, used at 178). The page's first cut either reads
  `last-response` per Claude session through the content worker, shows
  `waiting_reason` and the state word only, or the plan adds the field
  (an additive wire change). This is the "new read model unless the page
  cannot do without one" the issue allows.
- **`/pair`'s connection stays unauthenticated** (`authenticated: false`
  at `src/app.rs:3673`), and the pool move happens per connection on the
  first authenticated reply (`src/adapters/remote.rs:835-849`). A browser
  reuses connections, so its first authenticated fetch moves that one;
  others it opened for the page stay in the pool of four until idle (30
  s) closes them.
- **The events socket authenticates by bearer only**; `read_request`
  drops `Cookie` and `Sec-WebSocket-Protocol`, and `respond_upgrade`
  echoes no subprotocol. Whichever of the issue's two candidates the plan
  picks, both the parser and the 101 writer change, and `remote_subscribe`
  needs the device from somewhere other than `request.bearer` through
  `authenticate`.
- **Query values are not decoded** (`read_request`, 495-503). A `key`
  and `code` are hex and Crockford, so they survive; a `name` or an
  address would not.
- **The QR holds one address**: `local_address()` returns the outward
  route's IP. A Mac on Wi-Fi and Ethernet, or one whose phone sits on
  another interface, gets one URL for one interface.
- **Refusals are logged per request** (`refuse`, 3754), so a browser's
  automatic requests for icons would log unless the page's paths cover
  them or the log line is made conditional.
- **No existing test opens a plain TLS client without the pin**: `http`
  uses `pinned_client` (`src/adapters/remote.rs:168`), which is what a
  browser does not do; the "certificate warning then trust" step has no
  stand-in in the test harness and cannot have one.
- **A stale URL is refused, not counted.** `remote_pair` reads
  `pairing_at(now)` (`src/app.rs:3615`, `src/core/remote.rs:147-150`),
  which filters on the time itself between ticks, so a code scanned after
  the five minutes answers `pairing_closed` and does not bump `failures`.
- **The certificate has no SAN and a very long validity** (`make_identity`,
  95-127). The native pin ignores both; a browser's warning and its
  willingness to let the owner proceed depend on them.

## Open questions

- [source] Whether iOS Safari, after the owner clicks through the certificate warning for `https://<ip>:<port>/`, also accepts a `wss://` WebSocket to the same host and port under that self-signed certificate, or fails the WebSocket handshake silently; and the same for Chrome and Firefox on Android and desktop.
- [source] Whether a browser proceeds past the warning for a leaf certificate with no subject alternative name and a validity running to the year 4096 (Apple's 398-day and SAN rules for TLS server certificates), or refuses outright with no click-through, which would force `make_identity` to change and rotate every identity.
- [source] Whether a browser fails a WebSocket whose request carried `Sec-WebSocket-Protocol` when the 101 echoes none (RFC 6455 §4.1 client handshake, Chrome's "Sent non-empty 'Sec-WebSocket-Protocol' header but no response was received"), which decides whether `respond_upgrade` must echo the chosen value.
- [source] Whether a `Set-Cookie` with `Secure; HttpOnly; SameSite=Strict` set on an IP-address origin with a self-signed certificate is stored and sent on `fetch` and on `new WebSocket()` by Safari (iOS and macOS), Chrome and Firefox, and whether a bare IP host counts as a "secure context" for `Secure` cookies and for `history.replaceState`.
- [source] How many TCP connections a browser opens to one origin for a page with two or three files plus a fetch and a WebSocket (per-host connection limits in Safari, Chrome, Firefox), against the unauthenticated pool of four and the drop-at-accept at `src/adapters/remote.rs:699-706`.
- [source] Whether iOS's camera app opens an `https://<ip>:<port>/?...` QR in Safari directly, or shows it only as text when the host is a bare IP, and whether Android's camera does the same.
- [source] Whether `Content-Security-Policy` with no inline script and `connect-src 'self'` permits a same-origin `wss://` connection without naming the `wss:` scheme explicitly (the CSP3 rule that `'self'` matches `ws`/`wss` to the same host in newer browsers and not in older ones).
- [source] Whether `qrcode` 0.14 at the default error-correction level encodes a URL of about 140 to 160 characters in a version a phone camera reads reliably from a 4 px module on a Mac display, or whether `QR_SCALE` at `src/ui/remote.rs:41` must grow.
- [tree] Whether `read_request` can be extended to keep `cookie`, `sec-websocket-protocol`, `user-agent` and `origin` within `MAX_HEAD` 8 KiB and without a test depending on the dropped headers; I did not read the adapter's request tests at `src/adapters/remote.rs:1045-1062` in full.
- [tree] Whether the `scope=all` subscription and the `hide_workers` listing rule (`src/app.rs:2687`) apply unchanged to a page that reads `waiting` and `projects/{id}/sessions`, or whether the page needs `scope=all` for anything; I did not trace `hide_workers`.
- [tree] What `app.rs` does when `identity.listen.lan` is false and a device reaches `/info` through a loopback tunnel; whether the page should show "the host is not on the local network" and how it could know (the identity's `listen` is not in `/info`).
- [run] Whether rustls with the current `tls_config` (no ALPN set, TLS 1.2 and 1.3) completes a handshake with iOS Safari and Chrome, which negotiate HTTP/2 through ALPN, and whether they fall back to HTTP/1.1 against a server that offers no ALPN; the adapter speaks HTTP/1.1 only.
- [run] Whether a page load, its files, `/info`, `/pair`, the collections and the WebSocket all fit inside the four unauthenticated slots and the 5 s first-request deadline when a phone's browser preconnects speculatively.
- [owner] Whether the acceptance flow (scan, warning, click-through, sessions listed, a reply sent, revoke seen) works on the owner's phone and iPad under the signed bundle; the Keychain-held key and the stable signature are only on the owner's machine.
- [owner] Whether the owner's network routes the one address `local_address()` chooses to the phone, or whether the Mac has several interfaces and the URL must offer more than one.
