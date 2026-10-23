# Notes for #230: a web control surface on the remote port

Line numbers are from /Users/sbgragent/.dispatch/worktrees/7b32951e (branch
dispatch/230-…, HEAD 7fe8949). I read CLAUDE.md, docs/lessons.md (generic, nothing
bearing on this), docs/remote-client.md §4.1, §4.3, §4.4, §5, §9, and the code named
below. I did not read all of docs/design.md (3678 lines); I read only its remote status
section header (line 3107) and README grep hits. I ran nothing.

## On-disk record and rehydration (first, per the brief)

No new record type is needed.

- A browser is one more row in `<data dir>/devices.json` (`PairedDevice`,
  src/ports/remote.rs:96-107: id, name, scope, `token_sha256`, created, `last_seen`).
  The token's hash is stored, never the token. Revoke and scope change already work
  against that row (src/core/remote.rs:86 `RevokeDevice`; `device_by_token` at :145).
  Nothing in the issue needs a `SCHEMA_VERSION` bump unless the record gains a field
  (e.g. a "kind: browser" tag or a cookie/session id distinct from the token hash).
  That would be an additive field with `#[serde(default)]`.
- Host identity (`identity.json`, key in the Keychain) is unchanged.
- The pairing window is `never saved` state (`core.pairing()`); the URL it produces is
  derived each frame from identity + window + address, not stored.
- Rehydration is the existing path: restart reloads `devices.json`; a token held by the
  browser (cookie or storage) still hashes to a row. Key rotation clears `devices.json`
  (§5), so a browser then gets 401 and must say "revoked / pair again". Per §5, a changed
  key for a known `server_id` is "identity changed"; a page cannot see the key except via
  `/info` over a connection it can't pin (see Open questions).
- Browser-side state: only the token's carrier (cookie, or `localStorage`) and maybe the
  pinned `server_id`/key. That is a cache; the host can always re-pair it.

## Where the code is

Transport (src/adapters/remote.rs, 1088 lines)
- `read_request` :444-512 parses the head. It keeps only `authorization` (bearer),
  `upgrade`/`sec-websocket-key`, `content-length`. It does NOT keep `cookie`, `origin`,
  `user-agent`, `host`, `sec-websocket-protocol`, `accept-encoding`, `if-none-match`.
  `RemoteRequest` (defined near :360-380) would need new fields for any of them.
  Body cap `MAX_BODY` 64 KiB, head cap `MAX_HEAD` 8 KiB (:437-438). A cookie plus a
  long user-agent fits in 8 KiB.
- `respond` :530-552 hard-codes `Content-Type: application/json`, `Connection: keep-alive`,
  and no other headers. Static files need a content-type/headers path (CSP, nosniff,
  cache-control, `Set-Cookie`), and a status list has no 302/304/405 headers beyond 405.
- `respond_upgrade` :519 writes a 101 without `Sec-WebSocket-Protocol`; selecting a
  subprotocol would need it echoed.
- Pools: `MAX_UNAUTHENTICATED = 4`, `MAX_PAIRED = 12` (:567-568). Accept loop :691-719
  closes a stranger past 4. A connection moves to the paired pool only when a reply has
  `authenticated: true` (:835-848). An idle connection may sit `IDLE_TIMEOUT` 30 s
  (:575). Rate windows: `Limits` :608-628 only throttles `/api/v1/pair` (6/min) and
  `/api/v1/info` (60/min) by exact path (`PAIR_PATH`/`INFO_PATH` :586-587). Any other
  path is unthrottled, so static paths are unthrottled today.
- `tls_config` :144-155: no ALPN set, so a browser negotiates HTTP/1.1 (fine; no h2).
- `make_identity` :95-: certificate made with `CertificateParams::new(Vec::new())`, i.e.
  NO subject alt names, CN `Switchboard <server_id>`, never expires in practice
  (rcgen default `not_after` year 4096). Relevant to browser acceptance.

Routing (src/app.rs)
- `serve_remote` :3017-3150: `request.path.strip_prefix("/api/v1/")` else 404 (:3023).
  `GET info`, `POST pair` are served before `authenticate` (:3028-3032). `authenticate`
  :3153-3185 reads only `request.bearer`. `GET events` -> `remote_subscribe` :2723-2752
  (requires `request.upgrade`; sends `hello`). Sessions list = `projects/{id}/sessions`,
  `waiting`; per-session `sessions/{id}`; `sessions/{id}/screen|last-response` via
  `remote_content_route` :1947 (last-response is the Conversation cache key,
  :1967); `POST sessions/{id}/send` -> `remote_command` (:3190-) which needs
  `Scope::Control`, a body `{op, text}`, op 1-64 chars is the idempotency key, text
  1 B-16 KiB.
- `remote_info` :3568-3600: `server_id, server_name, protocol, capabilities, spki_sha256,
  version, limits, pairing_open`. Does not report the caller's scope (a page needs the
  device's scope for its header and for hiding the reply box; check whether any
  authenticated endpoint returns it — §5 says "the host answers with the device's
  scope" but I did not find which endpoint).
- `remote_pair` :3606-3670: checks window open, `spki_sha256` matches (wrong_key counts
  as a failure), code (case-insensitive), `device_name` (defaults to "Device"), mints
  the token with `remote::bearer_token()`, returns it in the JSON body
  (`token, scope, server_id, device_id`). A `Set-Cookie` would need `RemoteReply` to
  carry headers (`RemoteReply { status, body, authenticated }`, adapters/remote.rs:389).
- `refuse` :3754 logs method, path, code only. `envelope` wraps `{epoch, revision, body}`.
- `serve_remote` runs on the frame (8 requests per pass). Serving static files through
  the frame costs a frame slot per asset; serving them on the connection thread (like
  the 429 path at remote.rs:786-794) would skip the frame, the channel (`INBOX` 64) and
  the 8-per-pass drain. That is a design choice for the plan.
- No `Origin` or `Host` check exists anywhere (grep for "origin" in remote.rs and app.rs
  was empty).

Pairing UI (src/ui/remote.rs, 317 lines)
- `pairing()` :141-: builds the QR payload as JSON
  `{server_id, spki_sha256, code, addresses:[ "ip:port" ]}` (:160-166), address from
  `remote::local_address()` (UDP-connect trick, remote.rs ~:920) + `cx.remote_port` or
  `identity.listen.port`. Makes the QR texture (`qr_texture` :280, cached per code at
  :168-172), shows "Copy invite" copying the same JSON (:201-208), a code in two halves
  (`spaced` :314), `short_hash` :303.
- `devices()` :230-: paired list with scope and Revoke (where a user-agent-derived name
  would show).
- Only one address is ever offered; a loopback-only bind yields an address but the host
  may not be on the LAN (`remote_bound.lan`, app.rs ~:2998).

Core (src/core/remote.rs, 176 lines): `OpenPairing` :46, `ClosePairing` :55,
`PairingFailed` :56 (three closes), `RevokeDevice` :86, `pairing_at` :152,
`new_device` :166. All three actions are in the non-replayable/exempt list at
src/core/action.rs:1251-1256. Core tests for the window: src/core/tests.rs:11878-11915.

Native text pairing path that must keep working
- `PairingInvite.parse` (client/Sources/SwitchboardProtocol/Info.swift:94-116) decodes
  ONLY JSON `{server_id, spki_sha256, code, addresses}` via `Wire.decode`. Giving it a
  URL needs a new parse branch (and the same in the Rust side if any Rust code parses an
  invite — I found none; the host only emits it).
- `switchboard-remote pair` (client/Sources/switchboard-remote/main.swift:107-130) takes
  the JSON as one argument or `<host:port> <code> <spki>` as three. The visionOS app's
  paste path was not read (I did not open client/Apps). README.md:277 documents
  `switchboard-remote pair "$(pbpaste)"`, which a URL change must keep or amend.
- The protocol spec is amended in docs/remote-client.md §4.1 (bounds), §4.3, §5 (the
  "QR carrying {…}" sentence, "never in a URL" for the token, "Require pairing before
  anything but /info and /pair") and §9; docs/design.md "Remote client (2026-10-12)"
  at line 3107 is the status section to update. README Layout table (line ~383-423 hits)
  gets new `web/` files and any new `src` file; README dev-aid list for any new env or
  script line; CHANGELOG Unreleased gets a line.

Tests
- tests/remote.rs (3592 lines): harness `Port`/`port()` :37-66; `http()` :245 is a raw
  rustls HTTP/1.1 client that sends `Authorization: Bearer` and reads only
  `Content-Length`-framed bodies and the status line, returning `(u16, String)`; it
  neither returns headers nor sends other headers. Asserting headers (CSP,
  Content-Type, cache control, Set-Cookie) needs it extended or a sibling. `call()`
  :299 runs the client on a thread while the test thread calls `port.app.serve_pending()`
  (so static paths served on the connection thread would not need that).
  `pair()` :406; the allowlist test `the_allowlist_refuses_everything_else…` :682;
  `info_is_served_to_anyone…` :426; pair rate limit :1406; pools/handshake (Spike 22 /
  tests near :1395); fixtures `write_fixtures` :1587 and
  `the_client_fixtures_decode_on_both_sides` :1829; events helpers :1920-2150
  (`subscribe` :1955 builds an upgrade with the bearer header).
- Existing tests that assert a non-`/api/v1/` path is a 404 would change (the allowlist
  test :682 is the one to read).
- A page's script has no test runner in the tree (no node/package manager, and the issue
  forbids one); the only "browser" test is the host's own HTTP client.

## What the page needs from existing endpoints (checked)

- List: `GET /api/v1/projects`, `projects/{id}/sessions`, `waiting` (the issue says
  "waiting first and marked, with the card's one-line excerpt"). I did not locate the
  field name the card excerpt travels as in `SessionView`; `rg 'struct SessionView'`
  failed in my shell, so I did not read it. [tree] below.
- Live: `/api/v1/events` upgrade, `hello` with `epoch`/`revision`, then `changed
  {kind,id}`; kinds `session`, `sessions` (with project id), `waiting`, `screen`,
  `conversation`. Reconnect rule (§4.4): on `resync_required` or an `epoch` change,
  refetch everything; subscribe first, then snapshot, apply by revision. A page must
  re-implement this (the Swift side is the reference).
- `watch`/`unwatch` text frames exist for content keys (§4.4); content `changed` for
  `screen`/`conversation` only arrives for watched keys, so the session view must send
  `watch` and wait for `watching`.

## Things I noticed that bear on the decisions (facts, not answers)

- Decision 1/§5: the cert has no SAN and CN `Switchboard <uuid>`. The address in the URL
  is an IP. How a browser treats that during click-through is not in the tree.
- Decision 2: nothing in the tree reads cookies or `Sec-WebSocket-Protocol`; both need
  parser changes in `read_request` and (for cookies on a WebSocket upgrade, plus CSRF)
  an `Origin` check, since the cookie rides along on cross-site requests and an
  unchecked `Origin` exists today only because a bearer header can't be forged
  cross-site. `SameSite=Strict` is what the issue relies on. This is new attack surface.
- Decision 4: unauthenticated pool is 4 connections, each kept alive up to 30 s idle
  (:575, :801), and a page that loads HTML + CSS + JS in parallel while another tab
  or phone is also on the page can fill it. `/pair`+`/info` are throttled by path; static
  paths are not.
- Decision 4: §5 says before pairing only `/info` and `/pair`; static paths must be in the
  same "no token needed" class, and `authenticate` is not applied to them.
- `device_name` today is whatever the client sends; a browser has no user agent in
  `RemoteRequest` unless the page derives the name from `navigator.userAgent` and puts
  it in the `/pair` body (the page, not the host, can do that).
- The pair reply today carries the token in JSON, which suits `fetch` plus
  `localStorage`; the cookie route also changes `/pair`'s reply for native clients
  unless the cookie is set only on request.
- The QR image is built from the same `payload` string that "Copy invite" copies
  (ui/remote.rs:160-208); changing the string changes both at once; a URL of ~150
  characters fits in the same QR size class.
- Revoked device: `remote_subscribe` subscribers are closed on revoke (§5), so the
  page sees a socket close then a 401 on its next REST call.

## Not checked
- docs/design.md beyond line 3107 area, client/Apps (visionOS paste path),
  `SessionView`'s fields, the Swift `HostClient`/`Address` parsing, other test files
  that call non-`/api/v1/` paths, `scripts/`, `src/script.rs` pairing lines, and
  whether any authenticated endpoint reports the caller's scope.

## Open questions

- [tree] Which field of `SessionView` (and `waiting`) carries the card's one-line excerpt, the kind and the state, and does a list response carry enough to group by project without N+1 calls?
- [tree] Which authenticated endpoint tells a device its own scope and its server name, or does the page have to learn the scope from `/pair`'s reply and a 403 on `send`?
- [tree] Does the visionOS app's paste path (client/Apps) or any Rust code parse the invite string, beyond `PairingInvite.parse` and `switchboard-remote pair`?
- [tree] Which existing tests (tests/remote.rs:682, tests/ui.rs, src/script.rs, fixtures) assert the exact invite JSON or that a non-`/api/v1/` path is 404, and so change with this issue?
- [tree] What does `RemoteRequest` (adapters/remote.rs ~:360-380) look like in full, and does anything else construct it (fakes, app tests) that new header fields would break?
- [tree] Does `remote::local_address()` and `remote_bound` give an address a phone can reach when several interfaces exist (VPN, Thunderbolt bridge), and what does the URL carry when the port is loopback-only?
- [source] Safari on iOS and Chrome on Android: a self-signed certificate with no subjectAltName and a bare IP in the URL; "visit this website" click-through, and whether a `fetch` and `new WebSocket("wss://…")` to that origin work after it (the failure is that the page loads but the WebSocket or `fetch` is refused with a certificate error).
- [source] Safari/iOS: whether a certificate valid until year 4096, or a leaf certificate with a validity longer than 398 days, changes the warning or blocks the click-through (certificate lifetime limits, "HSTS", "TLS certificate trust" rules for self-signed leafs).
- [source] Whether the page's `history.replaceState` before any script runs removes the URL from the browser's session history and from a QR scanner's in-app browser / camera link preview (the failure: the code stays in a history entry, a preview URL, or the Referer on the first request).
- [source] Whether an `HttpOnly; Secure; SameSite=Strict` cookie set on a click-through (untrusted) certificate origin is stored and sent by Safari and Chrome, including on the WebSocket upgrade; and whether `Secure` cookies are refused on an origin with a certificate error.
- [source] `Sec-WebSocket-Protocol` as a token carrier: that the browser requires the server to echo exactly one offered subprotocol or fails the handshake, and what a token-bearing value may contain (token characters allowed in a subprotocol name).
- [source] Browser behaviour on a cross-origin / DNS-rebinding request to `https://<lan-ip>:<port>` with a `SameSite=Strict` cookie: whether an `Origin`/`Host` check is needed on the WebSocket upgrade and on `POST`s (Cross-Site WebSocket Hijacking).
- [source] The number of parallel connections a browser opens for one page (HTTP/1.1 per-host limit, speculative preconnects) against the 4-connection unauthenticated pool, and whether it retries a connection the host closes at once.
- [source] iOS/Android camera app behaviour when the QR holds an `https://` URL with a bare IP and a port: whether it offers to open it, and whether it opens in an in-app webview (with a different certificate prompt and no `localStorage` persistence) rather than the default browser.
- [source] Local Network privacy prompt (iOS 14+) and Safari: whether a page loaded from a LAN IP is subject to it, and whether it blocks the WebSocket.
- [run] Whether the frame-thread route for static files (8 requests per pass, one frame interval each, per Spike 22) makes the first page load noticeably slow against a connection-thread route; measure with the host's own client.
- [run] Whether a revoked device's open WebSocket closes promptly enough that the page can say "revoked" on its next request (the close, then a 401 on the refetch).
- [owner] The acceptance run on the owner's phone: scan, certificate warning, see sessions, a session starting to wait moves to the top, send arrives, revoke shows "revoked". Needs a device on the same network and the owner's Switchboard bundle.
- [owner] Which browsers/devices the owner actually uses (an iPhone's Safari vs a Chrome on Android vs an iPad), since the certificate-trust and cookie answers above differ by browser.
