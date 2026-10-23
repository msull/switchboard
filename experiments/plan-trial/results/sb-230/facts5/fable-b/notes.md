# Issue #230: a web control surface on the remote port — investigation notes

Worktree: `/Users/sbgragent/.dispatch/worktrees/f96f6569`, branch
`dispatch/230-a-web-control-surface-the-remote-port-se`, at 0.4.1. Line
numbers below are from this tree.

## The on-disk record and the rehydration, before the pane

The issue adds no new kind of record. A browser is a `PairedDevice` in
`<data dir>/devices.json` (`src/ports/remote.rs:95-108`: id, name, scope,
`token_sha256`, created, `last_seen`), written through the store's atomic
path by `RemoteFiles::save` (`src/adapters/remote.rs:66-71`) on
`Effect::SaveDevices`, which `AppAction::Paired` emits
(`src/core/remote.rs:66-72`). Rehydration is the existing one:
`load_identity` in `app.rs` reads `identity.json` and `devices.json` at
start, and a token presented later is looked up by its SHA-256
(`AppCore::device_by_token`, `src/core/remote.rs:145`). Nothing about a
browser needs a schema bump unless the plan adds a field to
`PairedDevice` (for example, a flag that the token was issued as a cookie);
`identity.json` has `IDENTITY_SCHEMA_VERSION` (`src/ports/remote.rs:17`)
but `devices.json` is a bare `Vec<PairedDevice>` with serde defaults and no
version field, so an added field needs `#[serde(default)]` and nothing
more.

The pairing window is transient and never saved: `PairingWindow`
(`src/core/action.rs:978-983`: `until`, `code`, `scope`, `failures`), held
in `AppCore::pairing` (`action.rs:1133`), opened by `OpenPairing` and
closed by time, three failures or the first success
(`src/core/remote.rs:44-72`). The code is made outside the core
(`remote::pairing_code`, `src/adapters/remote.rs:325`, eight Crockford
characters) and arrives in the action. A URL form of the invite changes
no record: the QR payload is built on the frame from `identity` plus the
window (`src/ui/remote.rs:160-165`).

The page itself has no record either. "Embedded at compile time" means
the HTML, CSS and JS are bytes in the binary; the token the page holds is
the device's state, kept wherever decision 2 puts it (a cookie the host
sets, or script storage), and the host's half of it is the device row.
So the durable store question is already answered by A1: the record is
the device row, the hydration is the token lookup, the browser's
"process" is the connection and its WebSocket subscriber, dropped on
revoke (`app.rs:486-498`: the subscriber queue is dropped, and the
connection thread closes the socket when the channel ends).

## What exists, with line numbers

### The listener (`src/adapters/remote.rs`)

- `read_request` (444-517) parses the request line, lowercases headers,
  and keeps only four things from them: `content-length` (480), the
  `authorization: Bearer` value into `RemoteRequest::bearer` (484-487),
  and `upgrade: websocket` plus `sec-websocket-key` into
  `RemoteRequest::upgrade` (488-491). `Cookie`, `Sec-WebSocket-Protocol`,
  `Origin`, `Host`, `Accept` and `If-None-Match` are read into the header
  list and discarded. The query is split into undecoded pairs (492-500);
  `RemoteRequest` (360-372) carries method, path without query, query
  pairs, body, bearer and upgrade, nothing else. Decision 2 either way
  needs a new field here and a new header read.
- `respond` (530-554) hardcodes `Content-Type: application/json` and
  `Connection: keep-alive`; the status-to-reason table covers 200, 400,
  401, 403, 404, 405, 413, 429, 503, 504. A static page needs a content
  type and the extra headers of decision 4 (CSP, cache control, and a
  `Set-Cookie` if the plan chooses a cookie), so `RemoteReply` (389-393:
  `status`, `body`, `authenticated`) gains headers or a content type, or
  the adapter answers static paths itself before the frame.
- `respond_upgrade` (519-527) writes the 101 with only
  `Sec-WebSocket-Accept`; a `Sec-WebSocket-Protocol` carrying the token
  would have to be echoed here, and the token would then pass through
  the connection thread's `RemoteRequest` as today's bearer does.
- `connection` (770-876): TLS handshake with the 5 s deadline (570), 30 s
  idle (573), one request at a time; `Limits::limited` (631-639) rate
  limits by exact path for `PAIR_PATH` and `INFO_PATH` only (586-587), so
  `GET /` and the page's files count against no window today; a request
  not reaching the frame is answered 429 or 503 by the thread. The pools
  are `MAX_UNAUTHENTICATED = 4`, `MAX_PAIRED = 12` (567-568); a connection
  moves pools on its first reply with `authenticated: true` (831-845).
  A browser opening the page and several files over keep-alive on one
  or two connections sits in the unauthenticated pool until its first
  authenticated reply; browsers open several parallel connections per
  origin, which is what decision 4's "which pool" is about.
- `subscriber` (878-927) runs the WebSocket non-blocking from a queue of
  256; the token never appears here, since authentication happened on
  the upgrade request.
- TLS: `tls_config` (144-158) is rustls with safe defaults, no ALPN set.
  `make_identity` (95-128) makes a P-256 key and a self-signed
  certificate with a CN of `Switchboard <server_id>`, **no subject
  alternative names** (`CertificateParams::new(Vec::<String>::new())`,
  line 102), and rcgen's default `not_after` (year 4096). Both matter
  for a browser's warning and its "trust this certificate" path; see
  the open questions.
- `local_address` (962-973) picks the outward interface's IP by a
  connected UDP socket; it is the one address the QR carries.

### The frame (`src/app.rs`)

- `serve_remote` (3017-3122): the `/api/v1/` prefix match at 3023 answers
  404 `not_found` for every other path, before authentication, so `GET /`
  is refused today with a JSON error body. `/info` and `/pair` are
  matched at 3028-3029 before `authenticate`; everything else is behind
  the bearer (3031-3034), with the events upgrade at 3035-3037.
- `authenticate` (3127-3157): bearer only, 401 `unauthorized` with
  "pair this device first" when absent and "the token is not known; pair
  again" when unknown; dispatches `DeviceSeen`. "Revoked" is therefore
  indistinguishable from "never paired" on the wire, by design (the
  refusal "says nothing of the host"). The page's "revoked" state has to
  be inferred from a 401 on a token that once worked.
- `remote_info` (3568-3604): `server_id`, `server_name`, `protocol`,
  `capabilities`, `spki_sha256`, `version`, `limits`, `pairing_open`.
  This is what the page compares the URL's `key` against.
- `remote_pair` (3606-3674): requires an open window, the host's own
  `spki_sha256` (wrong one counts a failure, 403 `wrong_key`), the code
  uppercased (403 `wrong_code`), `device_name` defaulting to "Device"
  and cut to 64 chars by `new_device`; replies `{token, scope, server_id,
  device_id}` in the envelope. The token is in the JSON body and nowhere
  else; a cookie variant would add a header here, through whatever
  `RemoteReply` gains.
- `remote_subscribe` (2723-2756): the device id is bound to the
  subscriber at the upgrade; `hello` is queued on the frame.
- `refuse` (3754-3766) logs `method path code` only; the path has no
  query, so a `GET /?code=…` that misses the allowlist does not log the
  code. Nothing else logs a request's path or query.
- `bind_remote` (2936-2985): loopback or `0.0.0.0` by `identity.listen.lan`,
  port from the identity or `SWITCHBOARD_REMOTE_PORT`. Nothing here
  changes for the page.
- `Served` (277-280) is `Now(RemoteReply)` or `Deferred`; the static paths
  would be `Now`.

### The pairing window (`src/ui/remote.rs`)

- `pairing` (141-228): while a window is open, the payload at 160-165 is
  the JSON `{server_id, spki_sha256, code, addresses}` with one
  `ip:port`; `qr_texture` (280-299) renders it with the `qrcode` crate
  (0.14.1); the text beside it is the spaced code, `Key` plus
  `short_hash` (303-312, first sixteen hex digits), `Address`, and the
  sentence "Scan, or type the key and the code. Grants … scope. Open for
  five minutes or three wrong codes." "Copy invite" (200-206) copies the
  same JSON. The headless UI test
  `remote_devices_dialog_opens_pairing_and_revokes_a_device`
  (`tests/ui.rs:7424-7489`) parses the clipboard as JSON and checks
  `code` and `spki_sha256`, so a URL payload breaks that assertion and
  the test must follow.
- `devices` (230-276): name, scope combo, paired-ago, seen-ago, Revoke.
  A browser row needs nothing new unless the plan wants a kind column.
- The dialog has no sentence about redaction or about certificate trust
  yet; decision 1's wording lands here.

### The native text path that must keep working

- `PairingInvite.parse` (`client/Sources/SwitchboardProtocol/Info.swift:
  97-117`) decodes the JSON only. Its callers: `switchboard-remote pair
  '<json>'` (`client/Sources/switchboard-remote/main.swift:107-125`), the
  headset's paste field (`client/Sources/SwitchboardUI/BrowserView.swift:
  883`), the launch-time `SWITCHBOARD_INVITE` split into lines and parsed
  per line (`BrowserView.swift:104-108`), and the fixture test
  `testInviteParsesTheQRText` (`client/Tests/SwitchboardProtocolTests/
  FixturesTests.swift:158-166`) with an inline JSON string, not a shared
  fixture file (`tests/remote.rs` writes no invite fixture: grep finds
  none). The three-argument form of `pair` takes `host:port code key` and
  reads `/info` pinned to the given key first (main.swift:112-121).
  `Address.init?(_:)` (`client/Sources/SwitchboardTransport/HostClient.
  swift:15-25`) parses `host:port` with bracketed IPv6. Accepting the
  URL means `parse` recognising `https://` and reading `code`, `key`,
  `id` from the query and the host and port from the authority, keeping
  the JSON branch.
- `ConnectionManager.pair(invite:)` (`client/Sources/SwitchboardClientCore/
  ConnectionManager.swift:482-491`) needs an address from the invite or
  the caller; the URL carries exactly one.

### Tests and harness (`tests/remote.rs`)

- `http` (245-297) is a hand-rolled HTTP/1.1 client over a pinned rustls
  connection: writes method, path, `Host: localhost`, optional bearer,
  `Content-Length`, reads status and `Content-Length` and the body. It
  ignores every other response header, so the header assertions of
  decision 5 need the helper to return headers too; a cookie flow needs
  it to send `Cookie:`. `call` (299-325) pumps `serve_pending` on the
  test thread while the client thread waits.
- `pair` (406-423) opens the window with `OpenPairing` and posts the JSON
  body; the end-to-end pairing test for the page would open the window,
  read the URL the dialog would build (a pure function the plan should
  lift out of `ui/remote.rs` so the test and the dialog share it), fetch
  `/info`, compare, post `/pair`, then read a view.
- `info_is_served_to_anyone_and_every_other_read_is_refused_unpaired`
  (426-475) asserts `GET /other` is 404 `not_found`; a served `GET /`
  changes the analogous expectation for the root only.
- `the_allowlist_refuses_everything_else_with_no_side_effect` (682) is
  where the static set's complement should keep being refused.
- `write_fixtures` (1587) and `the_client_fixtures_decode_on_both_sides`
  (1829): the fixtures cover the views; no view changes in this cut
  unless the excerpt question below adds a field.

### Dependencies and embedding

- No `include_str!` exists in the crate; `include_bytes!` is used for
  the icon (`src/main.rs:167`) and fonts (`src/ui/theme.rs:203-211`), so
  the pattern for embedding assets is established. There is no `web/`
  directory. Nothing in `Cargo.toml` serves static files; the HTTP layer
  is hand-rolled (`tungstenite` 0.30 for frames, `rustls` 0.23, `rcgen`
  0.14.10). Any header handling the page needs is hand-written in the
  adapter.
- CI runs clippy on Linux; the page and its tests have no macOS
  dependency.

### The read models the page draws

- `SessionView` (`control/src/lib.rs:637-688`): `id`, `project`, `name`,
  `kind`, `cwd`, `notes`, `liveness`, `card` (the card's word), `waiting`,
  `waiting_reason`, `trust_question`, `prompt_refusal`, `resume_id`,
  `last_stop_at_ms`, `last_activity_ms`, `pending`, `held_until_ms`.
  **There is no `last_message` or excerpt field** on the wire: the spec's
  §4.3 sentence that the card's excerpt is "`last_message` from the Stop
  hook on the record, a field of the session's metadata" does not match
  this tree; `grep last_message control/src/lib.rs` finds nothing. The
  egui card computes its one line in `agent_excerpt`
  (`src/ui/cards.rs:558-574`) from the parsed conversation (first line
  of the last turn's `final_text`, else the last activity line, else
  "You: <prompt>", 160 chars), and for non-agents from the pane caption
  or tmux title (`cards.rs:177-183`). So "each with … the one-line
  excerpt the card shows" is not reachable from the sessions collection
  today: the page would have to fetch `last-response` per Claude session
  (a content read through the worker, one key per session) or the plan
  adds an additive, bounded field to `SessionView`, which is a wire crate
  change plus a fixture update. The issue's "no new read model unless
  the page cannot do without one" meets this here.
- `GET /api/v1/waiting`, `projects`, `projects/{id}/sessions`, `sessions/
  {id}/last-response`, `sessions/{id}/screen`, `POST sessions/{id}/send`
  with `{op, text}` (16 KiB cap), and `/api/v1/events` with `changed
  {session|sessions|waiting|conversation|screen}` are all served
  (`serve_remote` arms at 3043-3100 and `remote_content_route`). The
  page's "waiting ones first" is the `waiting` flag and `card` word; the
  grouping is by `project` id against the `projects` collection.
- `/info` `capabilities` currently lists `read, control, draft, scope,
  tickets, evidence, create` (3579); a `web` capability is the natural
  place to say the host serves a page.

### Docs the plan amends

- `docs/remote-client.md` §4.1 bounds (lines 124-131), §4.3 table
  (157-219, "Everything else on the wire is refused … by an allowlist",
  line 194), §4.4 WebSocket (221-267), §5 (302-320: "Require pairing
  before anything but `/info` and `/pair`", line 306; the token "is sent
  only as a bearer header over the pinned connection, never in a URL",
  line 312; the QR carries the JSON, line 309).
- `docs/design.md` remote status section (3107-3584) ends with "Known
  gaps at the first merge" (3570-3583); the trust-boundary sentence at
  246-254 says "as redacted text on the remote port … the port's key and
  tokens never reach a record or a log".
- README dev aids at 267-279 describe `curl --insecure` and
  `switchboard-remote pair "$(pbpaste)"`; the Layout table lines 383
  (adapter), 401 (app.rs), 423 (ui/remote.rs), 437 (tests/remote.rs),
  446 (client/) all need the page added; a `web/` row is new.
- `CHANGELOG.md` Unreleased is empty at 0.4.1.

## What I did not check

- I did not run anything: no test, no build, no browser. The claim that
  `respond`'s hardcoded content type is the only place a header is
  written is from reading `remote.rs`, not from a run.
- I did not read `remote_content_route` or `remote_command` in detail
  beyond their signatures and the arms in `serve_remote`.
- I did not read the visionOS app entry for how `SWITCHBOARD_INVITE` is
  passed, only its consumer in `BrowserView.swift`.
- I did not look for existing egui UI tests of the QR texture beyond the
  one dialog test.

## Open questions

- Does Safari on iOS (and Chrome on Android) let a user proceed past the certificate warning for a self-signed certificate that has no subject alternative name and a `not_after` in the year 4096, and does the exception then persist for that `ip:port` origin across tabs and relaunches? [source]
- Does an iOS browser's click-through exception extend to the WebSocket connection on the same origin, or does `new WebSocket("wss://ip:port/…")` fail silently where `fetch` succeeded? [source]
- If the plan offers the certificate for download, does installing a profile on iOS trust a leaf certificate with no SAN for an IP address origin, and does it need the certificate to be marked a CA or carry the IP as a SAN? [source]
- Does a browser's `fetch` on a page served from `https://<ip>:<port>` send a `Cookie` set by `/pair` with `SameSite=Strict; Secure` on an IP-address origin with a self-signed certificate, and does the WebSocket handshake carry it too? [source]
- Does `Sec-WebSocket-Protocol` carrying a 43-character base64url token survive every browser's token-syntax check for subprotocol names, and does the browser fail the connection when the server's 101 does not echo it? [source]
- Does a browser retry a `GET` whose connection the host closed under its pool cap, or does the page load fail outright, given browsers open several parallel connections per origin and `MAX_UNAUTHENTICATED` is four? [source] [run]
- Does the phone camera on iOS and Android open an `https://<ip>:<port>/?…` URL from a QR in the default browser without a prompt, with the query intact? [owner]
- Does `history.replaceState` run early enough that the URL with the code never enters the browser's history or its sync, including the camera app's own URL preview and the tab list? [source]
- Does `qrcode` 0.14.1 fit a URL of about 140 characters at the error-correction level the dialog uses, and is the resulting module count readable at `QR_SCALE = 4` by a phone camera at arm's length? [tree] [owner]
- Does the headless harness in `tests/ui.rs` (7474-7489) have the `code` and `spki_sha256` assertions as the only places the invite's shape is asserted, or do the Swift tests assert the clipboard shape too? [tree]
- Which address does `local_address` return on a Mac with both Wi-Fi and a VPN or Tailscale interface up, and is it the one a phone on the same Wi-Fi can reach? [run]
- Does rustls 0.23 with no ALPN configured accept a browser's ClientHello that offers `h2` and `http/1.1` and fall back to HTTP/1.1, or does a browser then speak HTTP/2 framing the hand-rolled reader cannot parse? [source] [run]
- Does a browser send `Connection: keep-alive` requests with a body on `POST /api/v1/pair` in a shape `read_request` accepts (no chunked transfer encoding, `Content-Length` present), and does a `fetch` with `credentials` include a `Cookie` header longer than the 8 KiB `MAX_HEAD`? [source]
- Does a `Content-Security-Policy` with `default-src 'self'` and no inline script permit the page's `new WebSocket` to the same origin, or does `connect-src` need the `wss:` origin named explicitly in each browser? [source]
- How does the host tell a stranger's page load from a revoked device's page load, given `authenticate` answers both 401 and the page needs to say "revoked" plainly? [tree]
- Does the existing `test_stalled_strangers_fill_their_own_pool` scenario (tests/remote.rs:3226) hold when a browser's page files are also unauthenticated requests on the same pool? [run]
