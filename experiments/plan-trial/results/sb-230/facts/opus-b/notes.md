# Issue #230 — a web control surface on the remote port: investigation notes

Read first: `CLAUDE.md`, `docs/design.md` (core idea, Durable store,
Trust boundary, "Remote client (2026-10-12)" status at line 3107 on),
`docs/lessons.md`, `docs/remote-client.md` §4.1–§5 and §9–§11. Line
numbers are as they are in this worktree (branch
`dispatch/230-a-web-control-surface-the-remote-port-se`, at 7fe8949).
There is no `web/` directory yet.

## Record and rehydration first

**What is on disk today.**

- `<data dir>/identity.json` (`ports/remote.rs:26` `Identity`,
  `IDENTITY_SCHEMA_VERSION = 1` at :17): server id, display name, the
  self-signed certificate (DER, base64), its `spki_sha256`, `listen
  {lan, port}`. The private key is a Keychain item `remote-key:<id>`
  (:49).
- `<data dir>/devices.json` (`ports/remote.rs:96` `PairedDevice`): `id,
  name, scope, token_sha256, created, last_seen` per device. It is a
  bare JSON array written by `RemoteFiles::save_devices`
  (`adapters/remote.rs:83`) through `JsonStore`; it carries **no
  `schema_version` of its own** (only `identity.json` does), and
  `last_seen` is already a `#[serde(default)]` field, which is the
  precedent for additive fields here.
- The pairing window is core state only, never saved:
  `PairingWindow {until, code, scope, failures}` (`core/action.rs`
  around :290, `PAIRING_WINDOW` 5 min and `PAIRING_ATTEMPTS` 3 at
  :972–973), set by `OpenPairing` (`core/remote.rs:46`), spent by
  `Paired` (:64), counted by `PairingFailed` (:56).

**What a browser device adds to the record.** Nothing new is
required: a browser is a `PairedDevice` like the headset and the CLI.
`/pair` already takes `device_name` from the body (`app.rs:3650`) and
`new_device` (`core/remote.rs:166`) takes the window's scope, caps the
name at 64 chars, and keeps only the token hash. A "kind" field
(browser vs native) or a stored user-agent string would be an
additive `#[serde(default)]` field on `PairedDevice`; whether that
needs a version on `devices.json` is open (below). The token hash is
the same whether the token travels as a bearer header or a cookie, so
the record does not depend on decision 2.

**Rehydration.** Host side: unchanged. On start `IdentityLoaded`
brings back identity and devices; a browser token stays valid across
host restarts because only its hash is checked
(`AppCore::device_by_token`, `core/remote.rs:146`). Revocation removes
the record (`RevokeDevice`, :84) and drops that device's subscribers
(`app.rs:486–498`). Key rotation clears `devices.json`, which
invalidates a browser's token the same as any other.

Device side: there is no record on the device except what the browser
keeps for the origin `https://<address>:<port>`. That is the whole of
a browser's "rehydration": the token (cookie or storage), and anything
the page chooses to keep (server id, scope, last-known key). The
origin includes the address, so **a host whose DHCP address changes is
a different origin to the browser**: the cookie/storage and the
accepted certificate exception do not follow it, and the device has to
pair again from a fresh QR. The native clients do not have this
problem because they pin the key and re-resolve via Bonjour (§6.1).
Page content itself is never durable on the device beyond the
browser's cache, which decision 4 wants off.

The page's own files are not a record at all: they are compiled into
the binary (`include_str!`/`include_bytes!` — precedent at
`src/main.rs:167` and `src/ui/theme.rs:203–211`), so a host's version
and its page's version can never drift.

## Where the code is

### Listener: `src/adapters/remote.rs`

- `RemoteRequest` (:360–372): `method`, `path` (query stripped),
  `query` (pairs, **undecoded**, :497–504), `body`, `bearer`,
  `upgrade`. No `Cookie`, no `Origin`, no `Host`, no `User-Agent`, no
  `Sec-WebSocket-Protocol`, no `Accept` is kept; every other header is
  parsed into a local `headers` vec and thrown away (:456–480).
- `read_request` (:444): bearer read only as `Authorization: Bearer `
  (case-sensitive prefix, :487–490); upgrade recognised from `Upgrade:
  websocket` + `Sec-WebSocket-Key` (:491–494). Head cap 8 KiB, body
  cap 64 KiB (:437–438).
- `respond` (:530–553) **hard-codes `Content-Type: application/json`**
  and `Connection: keep-alive`, and has no way to add a header
  (`Set-Cookie`, `Content-Security-Policy`, `Cache-Control`,
  `X-Content-Type-Options`, `Referrer-Policy`). `RemoteReply`
  (:389–393) is `{status, body, authenticated}` with no content type
  or headers. Reason phrases for 101 is separate (`respond_upgrade`,
  :519–527); 304/302 are not in the reason table.
- `respond_upgrade` (:519): writes the 101 with
  `Sec-WebSocket-Accept` only; it does **not** echo a
  `Sec-WebSocket-Protocol`, which matters if decision 2 picks the
  subprotocol route (a browser fails the handshake if it offered a
  protocol and the server does not select one — to be confirmed,
  below).
- Pools (:564–568): `MAX_UNAUTHENTICATED = 4`, `MAX_PAIRED = 12`. The
  accept loop closes a fifth unauthenticated connection at once
  (:699–707). A connection moves to the paired pool only when the
  frame answers `authenticated: true` (:834–848).
- Rate windows (:582–587, :623–640): `/api/v1/pair` 6/min, `/api/v1/info`
  60/min, per listener, sliding (`RateWindow`, :596). Any other path is
  never limited (`Limits::limited` returns false, :634).
- Deadlines (:570–577): 5 s handshake and first line, 30 s idle between
  requests, 10 s write, 60 s for the frame's answer.
- `connection` (:770): one request in flight, the 429 and 503 answered
  without the frame, then hands to the frame over the 64-deep channel.
- `subscriber` (:878): the WebSocket, `tungstenite::from_raw_socket`
  after the hand-written 101.
- `make_identity` (:95–127): `CertificateParams::new(vec![])` — **no
  subjectAltName at all**, CN `Switchboard <server_id>`, rcgen's
  default validity (comment says `not_after` year 4096), P-256 key.
  `tls_config` (:144): rustls with ring, safe default versions.
  `local_address()` (:962): the routed outward IP, v4 or v6.

### Frame side: `src/app.rs`

- The pump: `app.rs:1586–1597` drains `DRAIN_PER_PASS` requests and
  calls `serve_remote`.
- `serve_remote` (:3017–3123): `strip_prefix("/api/v1/")` at :3023 —
  anything else is `404 not_found "no such endpoint"`; this is where
  `GET /` and the page's files would be matched (or before it, in the
  adapter). `info` and `pair` before auth (:3027–3031); everything
  else through `authenticate` (:3127), which reads only
  `request.bearer`. Events at :3036; the allowlist :3044–3116.
- `authenticate` (:3127–3153): no bearer → 401 `unauthorized` "pair
  this device first"; unknown token → 401 `unauthorized` "the token is
  not known; pair again". A **revoked** token and a **rotated** host
  both land on the second message; nothing in the reply tells
  "revoked" from "never paired here" from "host key changed".
- `remote_subscribe` (:2723–2758): accepts the upgrade, queues `hello`.
- `remote_info` (:3568–3600): `server_id`, `server_name`, `protocol`,
  `capabilities`, `spki_sha256`, `version`, `limits`, `pairing_open`.
  Note `spki_sha256` here is a **self-report in the body**, not
  something the page can check against the TLS key it is talking to.
- `remote_pair` (:3606–3670): requires the window, `spki_sha256 ==
  identity.spki_sha256` (:3633), the code (case-insensitive, :3643);
  reply `{token, scope, server_id, device_id}` in the envelope.
- `refuse` (:3754–3772) logs `method` and `path` only. Because `path`
  excludes the query, a `GET /?code=…&key=…` would not log the code;
  but `request.query` keeps it in memory and any future log of the
  target would leak it.
- Revocation closing sockets: `dispatch_inner` :486–498.
- `Subscriber` (:242–249) keys subscribers by device id, so a browser's
  socket closes on Revoke like any other — provided the browser's
  socket authenticated as that device.

### Pairing window: `src/ui/remote.rs`

- `pairing` (:141–227): the QR payload is built **in the UI** with
  `serde_json::json!({server_id, spki_sha256, code, addresses})`
  (:160–166) and `remote::local_address()` is called **on every frame
  while the window is open** (:157), which is I/O from `ui/`. CLAUDE.md
  says logic under `ui/` moves to the core; the URL form is a natural
  moment to make the invite a pure function (core or a small helper)
  with a test, and to cache the address.
- `addresses` is `"{ip}:{port}"` (:157–159); for an IPv6 address that
  string is not a valid URL authority — a URL needs `[ip]:port`.
- "Copy invite" (:201–207) copies the same JSON.
- The texture cache is keyed by the code (:167–172); a payload that
  changes for another reason (address, port) while the code holds is
  not redrawn.
- Scope radio (:212–218): the hover on Control still says "later the
  commands", stale since C3.
- Paired devices list (:230–276): name, scope combo, paired, seen,
  Revoke. No column says what kind of device it is.
- Redaction sentence (:72) is where a "the browser trusted this
  certificate by your click" sentence would go if decision 1 picks the
  click-through.

### Core: `src/core/remote.rs`

As above; `new_device` (:166) and the window. No change needed for a
browser beyond a possible name/kind.

### The text pairing path

- `client/Sources/SwitchboardProtocol/Info.swift:91–117`
  `PairingInvite` is `Decodable` with `server_id`, `spki_sha256`,
  `code`, `addresses`; `parse(_:)` (:114) is JSON only.
- Callers of `PairingInvite.parse`: the CLI
  (`client/Sources/switchboard-remote/main.swift:111`, one argument;
  the three-argument form :113–123 needs no invite), the headset's
  paste field (`client/Sources/SwitchboardUI/BrowserView.swift:883`),
  the launch-time dev aid `SWITCHBOARD_INVITE`
  (`BrowserView.swift:106`, one invite per line;
  `client/Apps/SwitchboardVision/SwitchboardVisionApp.swift:56`;
  `client/README.md:180–190`), and the Swift test
  `FixturesTests.swift:157` (a JSON literal, not a Rust-written
  fixture). README.md:277 documents `switchboard-remote pair
  "$(pbpaste)"`.
- A URL carries one address; the JSON carries a list. The CLI's device
  name is `"<hostname> (cli)"` (`main.swift:33`).

### Tests

- `tests/remote.rs` (44 tests): `http(addr, config, method, path,
  bearer, body)` at :245 is the host's own pinned client — it builds
  the request with a bearer only, no cookie or arbitrary headers, and
  returns `(status, body String)` without response headers. A test of
  `Content-Type`/CSP/`Set-Cookie` needs it to return headers. `pair`
  helper :406. The fixture writer :1579–1700 writes
  `client/Fixtures/*.json` (`SWITCHBOARD_WRITE_FIXTURES=1`).
- The allowlist test (:682) asserts non-`/api/v1` refusal behaviour
  that the page paths will change.
- The stalled-strangers test (README layout line for tests/remote.rs)
  depends on the 4-slot pool.

## What the page needs that the wire does not have

These are findings from reading the tree, each checked against the
code:

1. **The card's one-line excerpt is not on the wire.** The issue asks
   for "the one-line excerpt the card shows". §4.3 (spec) says that is
   `last_message` from the Stop hook, "a field of the session's
   metadata", but `SessionView` (`control/src/lib.rs:637–688`) has no
   `last_message`; it has `card` (the card's word), `waiting_reason`,
   `notes`. So either an additive field on `SessionView` (a wire
   change touching the Swift fixtures and `revision.rs`'s fingerprint),
   or a fetch of `last-response` per listed session (a content read
   each), or the page drops the excerpt.
2. **The device's own scope has no read endpoint.** It comes back only
   in the `/pair` reply (`app.rs:3660–3665`). The Swift client keeps it
   from there (`ServerConnection.swift:232`) and learns a lowered scope
   from a `403 scope` refusal. A page reloaded later (and with an
   `HttpOnly` cookie, which the script cannot read) cannot show "the
   device's scope" or decide to hide the reply box without either
   keeping scope in page storage, or a new tiny endpoint (who am I:
   device name and scope), which the issue says to avoid unless needed.
   Raising scope in the dialog is invisible to the page until it asks.
3. **"Revoked" and "identity changed" are not distinguishable from the
   replies.** Both are the same 401 `unauthorized`. The native client
   learns "identity changed" from the TLS pin failing, which the
   browser cannot see; a rotated host serves a new certificate, so the
   browser shows a certificate warning again before the page even
   loads, and after the click-through the page sees `/info`'s new
   `spki_sha256` and the same 401. The page can only say "identity
   changed" if it kept the old key/server id per origin (page storage)
   and compares.
4. **Screen for a stopped session.** `session.screen` refuses a
   session that is not running (spec §4.3), and only Claude Code has a
   last response; so a stopped shell/command/Codex session has nothing
   to show except `runs/{n}/output` for a command or service. The
   issue's "the screen for anything else" needs a plan for that case.
5. **Grouping by project costs N+1 reads.** Sessions are listed per
   project (`projects/{id}/sessions`); `waiting` is global. The page
   would read `projects` then each project's sessions, refetching a
   project's list on `changed {sessions, project}` — the headset does
   the same; fine at this scale but worth naming.
6. **Unauthenticated page loads use the 4-slot pool.** A page load
   (`/`, CSS, JS, `/info`) is unauthenticated until a request carries
   the token; browsers commonly open several parallel connections to
   one origin, and a fifth is dropped at accept. Static files also do
   not move a connection to the paired pool (`authenticated` is set
   only by the frame for a known token). Whether the page's assets
   should be served from the connection thread (no frame, like the
   429) and whether a cookie-bearing static request should count as
   paired are plan decisions.
7. **The key check in the URL protects less than it looks.** The page
   cannot read the TLS key it is connected to; it can only compare the
   URL's `key` with `/info`'s self-reported `spki_sha256`. An impostor
   that relays will report the genuine key in `/info` and pass that
   check, and the host's `wrong_key` check (`app.rs:3633`) also passes
   because the page sends the genuine key from the URL. So for a
   browser, §5's "key verified before the code is sent" is replaced by
   whatever the certificate trust step gives; the URL key only catches
   a QR pointing at the wrong host by mistake, or a stale QR. Decision
   1 has to say this plainly.
8. **Cookie or subprotocol auth needs adapter changes either way.**
   `read_request` must keep the cookie (or `Sec-WebSocket-Protocol`)
   value next to `bearer`, never in logged fields; `respond` must be
   able to send `Set-Cookie` (or `respond_upgrade` must echo the
   selected subprotocol). With a cookie, the API becomes reachable by
   ambient credentials, so `Origin` (and probably `Host`) checking on
   state-changing requests and on the WebSocket upgrade becomes
   necessary; today nothing reads `Origin` because no browser could
   authenticate.
9. **Logic in `ui/remote.rs`.** The invite payload and the address
   lookup live in the dialog; the URL form should be a pure, tested
   function, with IPv6 bracketing.

## Pane last

Only after the above: a static page (HTML + one CSS + one JS, no
framework) served at the port root, sessions grouped by project with
waiting first, a session view (last response or screen), a reply box
under control scope, header with host name and scope, explicit
"identity changed, pair again" and "revoked" states. On the Mac side
the dialog's QR changes to the URL, possibly the device list gains a
kind, and a sentence about the browser's trust step. The UI test for
the dialog (`tests/ui.rs`) would assert the invite text through "Copy
invite" or the core; I did not look for an existing test of the
dialog.

## What I did not check

- I did not run anything: no build, no tests, no browser, no phone.
- I did not read the Swift transport/core beyond the pair path and the
  scope handling lines named above, nor `client/README.md` beyond the
  invite lines.
- I did not read `tests/ui.rs` for an existing Remote devices dialog
  test, nor `src/script.rs` for the `pairing` script line's shape
  beyond the README mention (README.md:318).
- I did not read the conversation/last-response handlers
  (`app.rs:1947–2125`) in detail, nor `revision.rs`.
- I did not check `Cargo.toml` features of `rcgen` (SAN/EKU/validity
  options) or `tungstenite`'s subprotocol handling.
- Nothing outside the tree was consulted; every browser and platform
  claim in the issue is listed below as a question.

## Open questions

- [owner] Decision 1: is the click-through on the local network the trust step (said in the dialog), or does the host offer its certificate for download to trust on the device first?
- [source] Does iOS/iPadOS Safari let a page whose self-signed certificate was clicked through open a `wss://` WebSocket to the same origin, or does the socket fail where `fetch` succeeds?
- [source] Does Safari on iOS offer a click-through at all for a certificate with no subjectAltName (the current cert has none), and for a certificate whose validity runs to year 4096 (Apple's TLS server certificate requirements: SAN required, at most 825 days, serverAuth EKU)?
- [source] If the certificate is downloaded and trusted on the device instead, what must it carry (SAN with the IP address, validity limit, EKU, basicConstraints for a self-signed root) and how is it installed (profile, then Certificate Trust Settings)?
- [tree] Can the certificate be reissued over the same Keychain key with a SAN and a shorter validity without rotating pairings (the pin is the SPKI, so §5 says yes) — and where would reissue run (`make_identity` today only makes new keys)?
- [owner] If the certificate names an IP address, what happens when the Mac's address changes: reissue on every bind, or accept the mismatch as part of the click-through?
- [source] Does a browser treat a page served over a clicked-through self-signed certificate as a secure context (needed for `crypto.randomUUID`, `Secure` cookies, and some APIs)?
- [owner] Decision 2: `HttpOnly; Secure; SameSite=Strict` cookie, or the token as a `Sec-WebSocket-Protocol` value (with the token then held in page storage)?
- [source] Does a browser send a `SameSite=Strict` cookie on a same-origin `new WebSocket()` handshake, in Safari and Chrome both?
- [source] If the subprotocol route is chosen: what characters may a `Sec-WebSocket-Protocol` token carry (the bearer is base64url), and does a browser fail the handshake when the server's 101 does not echo a selected protocol?
- [source] Does a cookie set by a response on `https://<ip>:<port>` with no `Domain` stay scoped to that host and port, and does Safari's tracking prevention or ITP expire it?
- [owner] With a cookie: should the host check `Origin` on every POST and on the WebSocket upgrade, and `Host` against its own addresses (a DNS rebinding guard), and refuse when absent or foreign?
- [owner] Should a cookie-authenticated browser be able to read every endpoint the bearer can, or only the subset the page uses (tickets, drafts, dictation, new session are out of this cut)?
- [owner] Decision 3: one QR (the URL) read by the browser, the CLI and the headset's paste field, with the JSON dropped — or keep the JSON form as well, and for how long?
- [tree] Do any other consumers of the JSON invite exist besides the CLI, the headset paste field, `SWITCHBOARD_INVITE`, and the Swift `FixturesTests` literal?
- [owner] The URL carries one address while the JSON carries a list: which address goes in the URL when the Mac has several, and how is an IPv6 address written (`[addr]:port`)?
- [owner] Should the QR URL use the Bonjour `.local` hostname instead of an IP, so the origin survives a DHCP change?
- [source] Does iOS resolve `<name>.local` in Safari reliably, and would a certificate naming it be acceptable to the click-through?
- [owner] Decision 4: the exact static paths (`/`, `/app.js`, `/app.css`, an icon?), and whether `/index.html` or unknown non-API paths 404 or fall back to `/`.
- [owner] Decision 4: the headers on each static file: `Content-Type` per file, the exact `Content-Security-Policy` (`default-src 'self'; connect-src 'self' wss:`…?), `Cache-Control: no-store` or a versioned path, `X-Content-Type-Options`, `Referrer-Policy: no-referrer`, `frame-ancestors 'none'`.
- [source] Does `connect-src 'self'` cover a same-origin `wss://` WebSocket in Safari, or must `wss://<host>:<port>` be named?
- [owner] Should static files be answered on the connection thread (never reaching the frame, like the 429), or through the frame like every other request?
- [owner] Which pool and rate window do static page loads count against; is the 4-slot unauthenticated pool enough for a browser opening several connections, and does a token-bearing static request move its connection to the paired pool?
- [run] How many parallel connections does iOS Safari open to one origin when loading the page, and does the 4-slot pool drop any of them?
- [source] Do browsers keep a 30 s idle keep-alive connection and reuse it, or open fresh TLS handshakes per request (each counting against the pool and the 5 s handshake deadline)?
- [owner] What the page shows when opened with neither a code nor a token: instructions to scan the QR, the host name from `/info`, `pairing_open`?
- [owner] Decision 4: should the query be removed by `history.replaceState` before the first `fetch`, and should the code also be refused if it arrives on any path other than `/`?
- [source] Does a QR-opened URL reach the browser's history, the camera app's history, or iCloud tab sync before the page's `replaceState` runs, so that the code is kept somewhere the page cannot remove?
- [source] Is the query string of the first `GET /?code=…` sent in a `Referer` to anything, and does `Referrer-Policy: no-referrer` on the first response cover it?
- [tree] Does anything in the host log the request target or query (today `refuse` logs the path only); is there any other `log::` call in the remote path that includes `request.query`?
- [owner] Should the host refuse a `GET /` whose query carries a code when pairing is closed, or ignore the query entirely (the page alone reads it)?
- [owner] Decision 5: is the page tested only by `tests/remote.rs` (paths referenced, headers present, pairing end to end with the host's own client sending a cookie or subprotocol), with no browser in CI?
- [tree] How should the embedded page's referenced paths be extracted for the test (a fixed list, or parsing `src=`/`href=` from the HTML)?
- [owner] Should a small JS test harness exist for the page's revision/merge logic, given "no build step, no dependency"?
- [owner] Decision 6: device name from `navigator.userAgent` on the page or from the `User-Agent` header on the host; what wording (`Safari on iPhone`), and may the owner rename it?
- [owner] Should `PairedDevice` gain an additive field saying it is a browser (for the dialog), and does that warrant a `schema_version` on `devices.json`, which has none today?
- [tree] Is `devices.json` covered by the CLAUDE.md rule on serialized `model.rs` types, or only by `#[serde(default)]` convention like `last_seen`?
- [owner] The card excerpt: add `last_message` (or similar) to `SessionView` additively, read `last-response` per session, or drop the excerpt from the list?
- [tree] Where does the Mac's card take its one-line excerpt from today (record field name and its source), so a wire field would match it?
- [owner] The device's scope on the page: a new `whoami`-style endpoint, keep it in page storage from `/pair`, or infer it from a `403 scope`?
- [owner] How does the page tell "revoked" from "identity changed" from "never paired": should the 401 carry distinct codes, or should the page keep the last server id and key per origin and compare?
- [owner] What the session view shows for a stopped non-Claude session (screen is refused when not running): the last run's output, nothing, or a note?
- [owner] Should the reply box use `POST sessions/{id}/send` (plain) or the draft path, and should Resume and Trust be on the page in this cut?
- [owner] Should the page follow §4.4's subscribe-then-snapshot and per-key revision rule fully, including the 30 s refetch of a visible content view and `watch`/`unwatch` for the screen?
- [source] Does iOS Safari keep a WebSocket open when the tab is backgrounded or the screen locks, and what must the page do on `visibilitychange` to reconnect and resync?
- [owner] Does "a session that starts waiting moves to the top without a reload" rely on `changed {waiting}` / `changed {session}` alone, or also on a periodic refetch?
- [owner] The Linux CI rule: the static files and their routing must compile on Linux; is there anything macOS-specific in serving them (no, as far as read) — and should the page be in the README Layout table under `web/`?
- [owner] Should the spec (§4.3 table, §5, §4.1 bounds) and design.md's Remote client status gain a new section, and does this change warrant a spike directory (browser trust of a self-signed cert on iOS is a risky mechanism by CLAUDE.md's rule)?
- [run] Acceptance on a real phone: scan, click through, see sessions, watch a session move to waiting, send, revoke — needs a device on the same network and the signed bundle.
- [owner] Should Bonjour TXT or `/info` advertise a `web` capability so native clients and the dialog know the page is served?
- [owner] Does serving HTML on the port change §5's rule text to "nothing but `/info`, `/pair` and the page's static files before pairing", and does the page's static content count as "identity only" (it must contain nothing host-specific)?
