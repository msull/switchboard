# Investigation notes: issue #230, a web control surface on the remote port

Read: CLAUDE.md, docs/lessons.md (general architecture lessons only; nothing
about the remote port), docs/remote-client.md §4.1-4.4 and §5, §9, and the code
below. Line numbers are as in this worktree (branch
dispatch/230-a-web-control-surface-the-remote-port-se, HEAD 7fe8949).
I did not read docs/design.md's remote status section (starts at
`## Remote client (2026-10-12)`, line 3107, through 3584) beyond locating it, and
did not read the Swift code beyond the invite parser.

## On-disk record and rehydration (before the pane)

Nothing new is needed on disk for the page itself; the page is code in the
binary, not data.

- A browser is a `PairedDevice` (`src/ports/remote.rs:96`: id, name, scope,
  `token_sha256`, created, `last_seen`), persisted in `<data dir>/devices.json`
  through `RemoteFiles::save_devices` (`src/adapters/remote.rs:83`), atomic
  write. The token is never stored, only its SHA-256 (`token_sha256`, line 345).
  Nothing in the struct says what kind of client it is; the name is the only
  label. A user-agent-derived name fits the existing field with no schema bump.
- Rehydration of "paired" is the existing path: `StoreLoaded`/identity and
  devices load at start, the bearer hash is looked up by
  `AppCore::device_by_token` (`src/core/remote.rs:145`). A cookie carrying the
  same token would hash to the same record, so revocation
  (`AppAction::RevokeDevice`, handled in `app.rs:486`, which also drops the
  device's subscriber queues and so closes its socket) works unchanged.
- Key rotation deletes devices (§5 of the spec), so a browser's stored token
  stops working; the page needs a distinct "identity changed / revoked" reading
  of a 401 vs a changed `/info` key (the server answers 401 `unauthorized`
  with no distinction between revoked and never paired:
  `authenticate`, `src/app.rs` ~3130-3150).
- Client-side state (token if it is kept in `localStorage`, the pinned key) is
  per-browser, outside the data dir, and cannot be rehydrated by the host. With
  an `HttpOnly` cookie the token is not even visible to the page. A cookie's
  lifetime (session cookie vs `Max-Age`) decides whether a browser restart
  loses the pairing; the host holds the device record either way.
- The pairing window is `PairingWindow` in core (`src/core/remote.rs`), never
  saved (the spec says it is like `ENV_SETUP_WINDOW`).
- Waiting/live state on the page is not stored; it is the existing read model
  (`/waiting`, `/projects/{id}/sessions`, events with `revision`), refetched on
  `changed`. Nothing to rehydrate beyond reconnect + `hello` + snapshot (§4.4
  "Subscribe first, then snapshot").

## Code the issue touches

### Serving paths: `src/app.rs`
- `serve_remote` `src/app.rs:3017-3127`. Line 3023: `request.path.strip_prefix("/api/v1/")`
  else 404 `not_found` via `refuse(..., false)`. This is where `GET /` and
  static paths would branch first. `/info` and `/pair` are matched at
  3028-3033 before `authenticate`.
- `authenticate` ~3131: only reads `request.bearer`. No cookie input exists.
- `remote_info` `app.rs:3568-3603`: returns server_id, server_name, protocol,
  capabilities, `spki_sha256`, version, limits, `pairing_open`. Caveat: the page
  compares the URL's `key` to the `/info` key *over the same connection it then
  posts to*; `/info` is served by whoever answers, so it proves consistency, not
  authenticity (the issue already notes this).
- `remote_pair` `app.rs:3606-3665`: checks `spki_sha256` field equals the
  host's, then the code (case-insensitive), counts `PairingFailed` on either
  failure, names the device from the `device_name` body field (empty becomes
  "Device"), returns JSON `{token, scope, server_id, device_id}` through
  `envelope`. No headers can be set today (see below).
- `remote_subscribe` `app.rs:2723`: requires `request.upgrade`, authenticated
  by the same bearer path; registers a `Subscriber {device, all}`.
- `refuse` ~3754 and `remote_reply` ~3695 shape errors.
- The remote port is only bound on non-loopback after pairing is required;
  `advertise_remote` (3002) / `remote_addr` (3014).

### The HTTP adapter: `src/adapters/remote.rs`
- `RemoteRequest` `:360-385`: method, path (query split off), query pairs
  (undecoded, "ids and numbers only"), body, `bearer`, `upgrade`. Headers are
  dropped after `read_request` (`:444-517`); it reads only
  `content-length`, `authorization`, `upgrade`, `sec-websocket-key`.
  Not read: `cookie`, `origin`, `host`, `user-agent`, `sec-websocket-protocol`,
  `if-none-match`. Query values are not percent-decoded.
- `RemoteReply` `:389`: `{status, body: Vec<u8>, authenticated}`. No headers,
  no content type.
- `respond` `:530-552`: hardcodes `Content-Type: application/json`,
  `Content-Length`, `Connection: keep-alive`; no `Cache-Control`, no CSP, no
  `Set-Cookie`, no `X-Content-Type-Options`. Status reasons cover 200, 400, 401,
  403, 404, 405, 413, 429, 503, 504 only (no 304).
- `respond_upgrade` `:519-527`: 101 reply with `Sec-WebSocket-Accept` only; no
  `Sec-WebSocket-Protocol` echo (a browser that offered subprotocols fails the
  handshake if the server does not echo one of them).
- No `Origin` check anywhere on the upgrade (matters once a cookie
  authenticates: cross-site WebSocket hijack).
- Pools `:567-568`: `MAX_UNAUTHENTICATED = 4`, `MAX_PAIRED = 12`. The accept
  loop (`:700-730`) counts *every* new connection against the unauthenticated
  pool, and moves it to the paired pool only on the first reply with
  `authenticated: true` (`connection`, `:770-875`). A page load before pairing
  (HTML + CSS + JS + `/info` + `/pair`, over however many parallel keep-alive
  connections the browser opens) all sit in the 4. Timeouts: 5 s handshake and
  first request line (`HANDSHAKE_TIMEOUT`), 30 s idle (`IDLE_TIMEOUT`).
- Rate windows `:583-640`: `Limits::limited` matches exactly `PAIR_PATH`
  (6/min) and `INFO_PATH` (60/min); any other path returns false (unlimited).
  A static path would be unrate-limited by default.
- `connection` hands every request to the frame over `INBOX` (64), drained
  `DRAIN_PER_PASS` (8) per pass; static files would go through the frame
  unless short-circuited on the connection thread (a design choice).
- TLS: `make_identity` `:95-127` builds the certificate with
  `CertificateParams::new(Vec::new())`: no subject alternative names, CN
  `Switchboard <server_id>` only; validity is rcgen's default (not examined).
  `tls_config` `:144-157` sets no ALPN, so browsers will speak HTTP/1.1 (fine
  for the hand-rolled reader). The client pin `pinned_client` `:168` ignores
  names; browsers will not.
- `local_address` `:962`: the host's LAN IP via a UDP connect trick; the
  address is an IP literal. An IP-literal URL and a certificate with no IP SAN
  is the case a browser rejects as a name mismatch.

### The pairing window: `src/ui/remote.rs`
- `pairing` `:141-227`; payload built `:156-166` as JSON `{server_id,
  spki_sha256, code, addresses: ["ip:port"]}`; `qr_texture` `:280` encodes the
  payload; `Copy invite` button `:203-208` copies the JSON; the dialog text
  says "Scan, or type the key and the code" (`:194`). `short_hash` `:303`,
  `spaced` `:314`. The address is `ip:port` with the port from
  `cx.remote_port` else `identity.listen.port`.
- Scope is chosen in `draft.scope` (`:213-224`), radio Read-only/Control.
- Devices list/Revoke is `devices` `:230`; the dialog does not yet say anything
  about browsers.
- Core: `src/core/remote.rs` (`remote_action` `:21`, `pairing_at` `:152`,
  `new_device` `:166`).
- Script aids: `src/script.rs:729-737` (`pairing <code>|close`,
  `remote-devices`), README dev-aid list at README.md ~277-278 says the QR text
  is on the clipboard for `switchboard-remote pair "$(pbpaste)"`.

### Native text-pairing path that must keep working
- `client/Sources/SwitchboardProtocol/Info.swift:94-117`: `PairingInvite`
  is `Decodable` JSON with `server_id`, `spki_sha256`, `code`, `addresses`;
  `parse` is `Wire.decode(PairingInvite.self, from: text)`. A URL would need
  `parse` to accept `https://addr/?code=&key=&id=` (the `key` and `id` query
  names differ from the JSON names).
- `client/Sources/switchboard-remote/main.swift:12-13, 105-125`: the CLI takes
  the JSON as one argument or `<host:port> <code> <spki_sha256>`.
- visionOS paste field: `client/Sources/SwitchboardUI/BrowserView.swift:792,
  849, 883`; `ConnectionManager.pair(invite:)`
  `client/Sources/SwitchboardClientCore/ConnectionManager.swift:482`.
- Swift tests/fixtures: `client/Tests/*`, `client/Fixtures` (rewritten by
  `SWITCHBOARD_WRITE_FIXTURES=1`, `tests/remote.rs:1587 write_fixtures`).

### Tests today: `tests/remote.rs`
- Helper `http` `:245-297` is a raw TLS client with a pinned config; it sets
  only `Authorization` and `Content-Length`, parses only `Content-Length`
  from response headers (returns status + body, not headers), so asserting
  `Content-Type`/CSP/`Cache-Control` needs a helper that returns headers.
  `call`/`get` `:299-331`, `pair` helper `:406`.
- Relevant existing tests: `info_is_served_to_anyone_and_every_other_read_is_refused_unpaired`
  `:426`; `the_allowlist_refuses_everything_else_with_no_side_effect` `:682`
  (asserts `/api/v2/projects` is 404 `not_found`; a `GET /` test belongs
  beside it); `a_pairing_with_another_hosts_key_or_the_wrong_code_counts_and_three_close_the_window`
  `:582`; `pair_is_rate_limited_per_listener` `:1406`; stalled strangers filling
  only their pool (listed in README); revoke `:658`.
- No `web/` directory exists. The only embedded assets in the tree are
  `include_bytes!` fonts/icon (`src/ui/theme.rs:203-211`, `src/main.rs:167`);
  `include_str!` is not used anywhere yet.

### Spec and docs to amend
- docs/remote-client.md: §4.1 bounds (line ~126-134, the pool/rate bullets),
  §4.3 endpoints table (line ~159) which says everything else is refused,
  §5 (line ~302-320) pairing: QR JSON text, "bearer header over the pinned
  connection, never in a URL", "Require pairing before anything but `/info`
  and `/pair`", and the "Loopback-only without auth" line; §9 (no relay).
- docs/design.md "Remote client (2026-10-12)" status section (3107-3584).
- README.md Layout table (line ~383 `src/adapters/remote.rs`, ~423
  `src/ui/remote.rs`, ~437 `tests/remote.rs`), the dev-aid note at ~277; a new
  `web/` dir needs a Layout line. CHANGELOG.md Unreleased line (user-visible).
- CLAUDE.md rules that bind the plan: `src/core` stays deterministic; no new
  dependency for the page; secrets never in a URL/log (the invite URL carries
  the one-time code, and the issue's own plan removes it from the address bar);
  never name a client/customer in tests or fixtures; no tool attribution in
  commits/PR bodies; schema bump only if a serialized model type changes (a
  device kind field would; a name-only record would not).

## Facts the tree settles that bear on the issue's decisions
- A cookie cannot be set or read today: no response-header channel exists
  (`RemoteReply` and `respond` above) and `Cookie` is not parsed. Either choice in
  decision 2 changes `RemoteReply`/`respond`/`read_request` (and a
  `Sec-WebSocket-Protocol` echo in `respond_upgrade` if the subprotocol route is
  chosen).
- The frame decides `authenticated: true` in `RemoteReply`; the connection
  thread uses it to move a connection between pools, so a cookie-authenticated
  request would need the same mapping.
- A `GET /` that isn't under `/api/v1/` is currently a 404 JSON `not_found`
  produced at `app.rs:3023` after a frame round-trip; the `Limits::limited`
  and pools sit below that in the adapter.
- `/pair`'s wrong-key and wrong-code paths both count toward the window's three
  failures; a page that compares keys first avoids sending the code to a
  mismatched host, but the rejection is also server-enforced.
- The device name arrives only from the `/pair` body (`device_name`); the User-
  Agent is not read by the host, so "Safari on iPhone" is derived either in the
  page script from `navigator.userAgent` or in `read_request` by adding a
  header read.
- The QR is currently a JSON payload with an address list; a URL carries one
  address, and the address is chosen once by `local_address()`.

## Not checked
- The Swift test files and `docs/design.md` remote status contents in detail.
- Whether `qrcode` crate handling of a longer URL payload changes QR size/version
  (the URL carries a 64-hex key, a UUID, an 8-char code, an address: roughly
  150 characters vs the current JSON of similar length).
- The exact certificate validity rcgen sets by default, and whether anything in
  `Cargo.toml` pins the `rcgen` version/features for SANs or dates.
- How `ControlSocket`-style `serve` handles `last-response` for non-Claude
  sessions on the wire (the page needs the screen for those); `session` view
  fields the card excerpt uses were not enumerated.

## Open questions
- [tree] Which `SessionView` fields give kind, state, waiting reason and the one-line card excerpt (the wire crate under `control/` and `session_view` in `src/core`), and does `/waiting` carry enough to order sessions without a second fetch?
- [tree] Which pairing/connection paths in `docs/design.md`'s remote status section (3107-3584) and `docs/remote-client.md` §11 (tests and failure cases) already state the unauthenticated-pool and rate rules a static page would have to amend?
- [tree] What do the Swift tests and `client/Fixtures` assert about `PairingInvite` and the QR text, so the URL form can be accepted without breaking them?
- [tree] How does `last-response` answer for a non-Claude session (does it refuse, and with what code), so the page knows when to fall back to `screen`?
- [tree] Does `identity.listen` / `remote_port` give a single address the URL can carry, and what happens when `local_address()` returns `None` (the URL has no address)?
- [source] The browser: does Safari on iOS, Chrome on Android and desktop Chrome let the user proceed past a self-signed certificate that has no subject alternative name and a CN only, when the page is loaded by IP literal (ERR_CERT_COMMON_NAME_INVALID vs a click-through warning)?
- [source] The browser: do Safari/iOS and Chrome apply their certificate validity-length limits (Apple's 398-day rule, Chrome's equivalent) to a certificate the user has clicked through, and what validity does `rcgen` give a `CertificateParams::new` certificate by default?
- [source] The browser: after the user accepts a certificate warning, does Safari on iOS remember the exception for the IP and port across tabs, `fetch` and `new WebSocket("wss://...")`, and for how long (it is known to drop the exception on some page reloads)?
- [source] The browser: does `wss://` on a certificate exception the user accepted work for a WebSocket opened by the page, or does the browser fail the socket separately from `fetch` (the failure on a self-signed `wss`)?
- [source] The mechanism: is a `Secure` cookie stored and sent by Safari and Chrome for an `https://<LAN IP>:<port>` origin whose certificate was clicked through, and does the cookie then ride the WebSocket upgrade request (cookies are not scoped by port)?
- [source] The mechanism: `SameSite=Strict` on an `https://<IP>:<port>` page opened from a QR scanner's in-app browser or from Camera's link preview: is the cookie still sent on the first navigation after pairing?
- [source] The protocol: when a browser offers `Sec-WebSocket-Protocol` values and the server's 101 reply omits one, does the browser fail the connection (RFC 6455 §4.1), and does the browser send subprotocol values in a way that appears in logs or intermediaries?
- [source] The mechanism: `history.replaceState` removing the query: does Safari, Chrome or the iOS camera app's in-app viewer keep the original URL (with the code) in browsing history, the camera's "recent" list, or a QR-scanner app's history, before the script runs?
- [source] The browser: does the `Referer` header or an `unload`/prefetch of the first page ever send the full URL (with code) to the same host's static requests, and does `Referrer-Policy: no-referrer` change that for sub-resources of the same origin?
- [source] The mechanism: with `Content-Security-Policy` that has no inline script and no external resource (`default-src 'self'`, `connect-src 'self'` and WebSocket), does `connect-src 'self'` cover `wss://` to the same host in Safari (older WebKit needed `wss:` named explicitly)?
- [source] The browser: how many parallel connections does Safari/Chrome open for a page and its three static files over HTTP/1.1 with keep-alive, and does a connection a browser opens speculatively (preconnect) count against the 4-slot unauthenticated pool?
- [source] The failure: how does the browser behave when the server closes a socket from the accept loop without a response (the pool-full case) and when a keep-alive connection is closed by the 5 s first-line deadline; does it retry silently and thereby hold slots?
- [source] The mechanism: `navigator.userAgent` on iPadOS and recent iOS Safari (desktop-class "Macintosh" UA) and `navigator.userAgentData` on Chromium: can the page derive "Safari on iPhone" vs "Safari on iPad", or only the generic string?
- [source] The protocol: HTTP/1.1 conditional requests and `Cache-Control` for versioned embedded assets: what a browser does with `no-store` vs `no-cache` on an origin with a user-accepted certificate (some browsers refuse to cache any content from a certificate-error origin regardless).
- [run] Does a real iPhone camera opening `https://<ip>:<port>/?code=...` reach the page at all given that the certificate has no SAN: run the host on the LAN with `remote-listen lan <port>` and open the URL.
- [run] Does the hand-rolled `read_request` cope with a browser's request head (Cookie, long User-Agent, Accept-*, Sec-Fetch-*) within `MAX_HEAD` (8 KiB), especially with a cookie added?
- [run] Does an HTTP/1.1 keep-alive page load of HTML + CSS + JS + `/info` through the frame stay within the 5 s first-line and the 8-per-pass drain when `serve_pending` runs only on frames (hidden window case in lessons: `logic` vs `ui`)?
- [owner] Does the owner's phone/iPad accept a click-through on this host's certificate and keep it (the acceptance criterion needs a device); which browsers on which devices must be supported?
- [owner] Does the owner's QR scanner (camera app, third-party) open the URL in the system browser or an in-app view that has a different certificate store?
- [owner] Is a phone's link-preview or messaging app's URL fetch (if the owner shares the invite by message rather than scanning) acceptable, given it would consume or leak the one-time code?
