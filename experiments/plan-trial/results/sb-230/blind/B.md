# A web control surface on the remote port (issue #230)

A page the host serves from the remote port, opened in a browser on the
local network, that lists the sessions with the waiting ones first,
shows one session's last response or screen, and sends a reply. It
speaks the protocol the headset speaks, pairs from the QR code, which
becomes a URL, and holds its token in a cookie the page's script never
sees. One commit; the core contract (`model.rs`, `action.rs`,
`ports/`) is untouched.

## Where this departs from the issue

- **The excerpt is not on the session view.** The issue lists "the
  one-line excerpt the card shows" beside a session's kind and state.
  `SessionView` (`control/src/lib.rs:637`) carries no excerpt: the
  Mac's card derives it from the parsed transcript
  (`agent_excerpt`, `src/ui/cards.rs:558`), and the spec's
  `last_message` field (`docs/remote-client.md` §4.3) was never added.
  Adding one means the app post-processing every session view with a
  transcript parse, which §4.1 keeps off the frame. The page instead
  reads `last-response` for the **waiting** Claude Code sessions only
  and shows its first line, cut at 160 characters as the card cuts it;
  every other session shows its card word and `waiting_reason`. The
  waiting sessions are the few whose gist matters, and the read is the
  content cache's, versioned and shared with the session view.
- **The pairing parameters ride in the URL's fragment, not its
  query.** `https://<address>:<port>/#code=<code>&key=<spki>&id=<id>`
  rather than `/?code=...`. A fragment is never sent to the host, so
  the code is in no request line, no `Referer` on the page's own file
  loads (the HTML's `<script>` fetch happens before any script runs,
  and a query would be in that request's `Referer`), and no refusal
  log. The page still clears it with `history.replaceState` first
  thing, so it leaves the address bar and the history as the issue
  asks. A phone's camera opens it the same way.
- **The token never reaches the page's script, even at pairing.** The
  issue's cookie candidate has `/pair` set the cookie; the reply body
  would still carry the token, which the pairing script would see
  once. The page asks for the cookie (`transport: "cookie"` in the
  `/pair` body) and the reply then omits `token`. Native clients send
  no `transport` and get the body they get today.

Everything else in the issue is borne out: the allowlist's prefix
match (`src/app.rs:3023`), the four-connection unauthenticated pool
(`MAX_UNAUTHENTICATED`, `src/adapters/remote.rs:567`), the `/pair` and
`/info` windows (`src/adapters/remote.rs:583`), the QR payload
(`src/ui/remote.rs:160`), the Swift text path
(`PairingInvite.parse`, `client/Sources/SwitchboardProtocol/Info.swift`,
used by `main.swift:111` and `BrowserView.swift:106` and `:883`).

## Decisions

1. **Trust is the click-through on the owner's own network.** The
   other route, offering the certificate for download, does not work
   here: the certificate is made with no subject alternative names
   (`make_identity`, `src/adapters/remote.rs:102`), so even a browser
   that trusts it as a root rejects it for an IP address, and putting
   the address in it would tie the identity to a DHCP lease. iOS also
   needs a profile installed and trust enabled in two settings
   screens. So: the owner scans, the browser warns, the owner accepts,
   and the browser keeps that exception for the address. What it
   buys: every later visit from that browser is to the certificate it
   accepted. What it does not: the first visit is unverified, and the
   `key` in the URL protects only a genuine page (a page served by an
   impostor at that address is the impostor's, and the host's own key
   check on `/pair` cannot tell a relayed code). The residual risk is
   a machine on the local network answering at the Mac's address
   during the five-minute window; the dialog says so in one sentence,
   and §5's rule stands: before pairing a non-loopback bind serves
   `/info`, `/pair` and the page's three files, nothing else.
2. **The token is a cookie.** `/pair` with `transport: "cookie"`
   answers `Set-Cookie: switchboard-<id8>=<token>; Path=/; Secure;
   HttpOnly; SameSite=Strict; Max-Age=34560000`, where `<id8>` is the
   first eight hex digits of `server_id` (cookies ignore the port, so
   two instances on one machine would otherwise read and clear each
   other's). `fetch` and `new WebSocket()` both carry it, the script
   cannot read it, and it lives in the browser's jar, not
   `localStorage`. The host accepts the bearer header as before; a
   request with both is answered by the bearer. A cookie-authenticated
   request whose `Origin` header names an origin other than
   `https://<Host header>` is refused `403 wrong_origin` (same-origin
   `GET` fetches send no `Origin`; a WebSocket always does), which is
   the cross-site defence `SameSite=Strict` alone leaves to the
   browser. A `401` to a request that carried the host's cookie
   clears it (`Max-Age=0`), so a revoked browser stops presenting a
   dead token.
3. **One QR.** The URL above. The Swift `PairingInvite.parse` takes
   it as well as the JSON, so the headset's paste and
   `switchboard-remote pair` keep working; the JSON form stays only as
   what `parse` still accepts and `SWITCHBOARD_INVITE` may carry.
   "Copy invite" copies the URL.
4. **Static paths.** `GET /` (`text/html; charset=utf-8`),
   `GET /app.css` (`text/css; charset=utf-8`), `GET /app.js`
   (`text/javascript; charset=utf-8`). Each answers with
   `Content-Security-Policy: default-src 'none'; script-src 'self';
   style-src 'self'; connect-src 'self'; img-src data:; base-uri
   'none'; form-action 'none'; frame-ancestors 'none'`,
   `Cache-Control: no-store`, `X-Content-Type-Options: nosniff`,
   `Referrer-Policy: no-referrer`. No inline script or style, no
   external resource; the icon is `data:,` so no `/favicon.ico` is
   asked for. `no-store` is what keeps a page from one app version
   driving the next. Every other path outside `/api/v1/` stays `404`.
   An unpaired page load counts against the unauthenticated pool as
   today, and the three paths share one new window, `PAGE_PER_MINUTE =
   60` per listener, like `/info`; a page load is three requests. With
   no fragment and no cookie the page shows the host's name and short
   key from `/info` and the sentence "Pair from the Mac: Remote devices,
   Allow pairing, scan the code with this device", with a field for a
   typed code that pairs against the key `/info` presented, as the
   headset does.
5. **Where it lives and how it is tested.** `web/index.html`,
   `web/app.css`, `web/app.js` embedded by `include_str!` through
   `src/web.rs`; tests in `tests/remote.rs` against the real listener
   with the test file's own HTTP client, below.
6. **The device record** is unchanged: the page sends `device_name`
   as `<browser> on <platform>` ("Safari on iPhone") worked out from
   `navigator.userAgent`, the scope is the pairing window's, and the
   dialog lists and revokes it like any device.

## The record

Nothing: `identity.json` (`IDENTITY_SCHEMA_VERSION` 1 in
`src/ports/remote.rs`), `devices.json`, and the store's
`SCHEMA_VERSION` (16) and `VIEWS_SCHEMA_VERSION` (7) in
`src/core/model.rs` are untouched; no bump.

## The signals

Nothing new: the page lives on `hello`, `changed`, `removed` and
`resync_required` as the headset does, with `watch`/`unwatch` on
`conversation` and `screen` keys.

## The port

In `src/adapters/remote.rs`:

- `RemoteRequest` gains `cookies: Vec<(String, String)>` (the `Cookie`
  header's pairs), `origin: Option<String>` and `host: Option<String>`,
  read in `read_request` beside `bearer`. `RemoteReply` gains
  `headers: Vec<(String, String)>`; `respond` writes them and writes
  `Content-Type: application/json` only when they name no content
  type. The dozen literal `RemoteReply { .. }` constructions in
  `app.rs` take `headers: Vec::new()`; a `RemoteReply::json(status,
  body, authenticated)` helper keeps that to one line each.
- `Limits` gains a `page` window; `limited` maps `/`, `/app.css` and
  `/app.js` to it. `PAGE_PER_MINUTE = 60`.
- `pub fn invite_url(address: &str, identity: &Identity, code: &str)
  -> String` beside `pairing_code`, so the dialog and the tests build
  the one URL.

`src/web.rs` (new): `pub struct Page { pub path, pub content_type,
pub body: &'static str }`, `pub const PAGES: [Page; 3]` from
`include_str!("../web/...")`, `pub fn page(path) -> Option<&'static
Page>`, and `pub fn headers(page) -> Vec<(String, String)>` with the
list in decision 4.

In `src/app.rs`:

- `serve_remote` (`src/app.rs:3017`): before the `/api/v1/` strip, a
  `GET` for a page answers it `200` with its headers,
  `authenticated: false`, no frame state touched.
- `authenticate` (`src/app.rs:3127`): the token is the bearer, else
  the cookie named `switchboard-<id8>` for this host's identity. A
  cookie token with an `Origin` that is not `https://<host>` is
  `403 wrong_origin` before the device lookup. An unknown cookie
  token's `401` carries the clearing `Set-Cookie`.
- `remote_pair` (`src/app.rs:3606`): reads `transport`; for
  `"cookie"` the reply has the `Set-Cookie` header and no `token`
  field.
- `remote_info` (`src/app.rs:3568`): `"web"` joins the capabilities.
- Nothing else: `remote_subscribe` sees an authenticated device as
  before, revocation drops the device's subscribers as before
  (`src/app.rs:486`).

The page (`web/app.js`, plain ES2020, no dependency, the three files
under 40 KiB together):

1. First statement: parse `location.hash` for `code`, `key`, `id`;
   `history.replaceState(null, "", location.pathname)`.
2. `GET /api/v1/info`; the header shows `server_name` and the short
   key (first sixteen hex digits in fours, as the dialog prints them).
3. With a code: `key` must equal `/info`'s `spki_sha256` and `id` its
   `server_id`, else "This is not the Mac whose code was scanned" and
   the code is dropped. On a match, `POST /api/v1/pair` with `{code,
   device_name, spki_sha256, transport: "cookie"}`; `wrong_code`,
   `wrong_key` and `pairing_closed` are said in those words. The page
   keeps `{server_id, spki_sha256, scope, device_name}` in
   `localStorage` (nothing secret) to tell the states apart later.
4. Subscribe first: `wss://<host>/api/v1/events`, wait for `hello`,
   then `GET /projects`, `GET /projects/{id}/sessions` per project and
   `GET /waiting`; one held revision per resource key, an older reply
   dropped, an event past a snapshot's revision refetches it (§4.4).
   The list is grouped by project, waiting sessions first and marked,
   each with kind, card word, `waiting_reason`, quiet time from
   `last_activity_ms`, and for a waiting Claude Code session the first
   line of `GET /sessions/{id}/last-response`.
5. A session view: `watch {conversation, id}` then `last-response` for
   a Claude Code session, `watch {screen, id}` then
   `screen?lines=100` for anything else; refetched on `changed` of
   that key and every thirty seconds while shown; `unwatch` on back.
6. A reply box when `/pair`'s or the stored scope is `control`,
   hidden otherwise: `POST /sessions/{id}/send` with `{op:
   crypto.randomUUID(), text}`; a timeout asks `GET /ops/{op}` and
   never resends blind.
7. A `401` with a stored pairing: if `/info`'s key differs from the
   stored one, "The Mac's identity changed; pair again", else "This
   browser was revoked on <name>; pair again from the Mac". A closed
   socket reconnects with backoff; a new `epoch` or
   `resync_required` refetches everything.

Swift, so the native text path takes the URL: `PairingInvite.parse`
tries `URLComponents` first (scheme `https`, host and port as the one
address, the fragment's `code`, `key`, `id`) and falls back to the
JSON; `main.swift:12`'s usage says "invite url or json";
`client/README.md`'s `SWITCHBOARD_INVITE` paragraph says the URL also.

## The actions and effects

None new: pairing, `Paired`, `DeviceSeen`, `RevokeDevice` and the
saves are what they are; the core is not touched.

## The UI

`src/ui/remote.rs`, `pairing` (`:141`): the payload is
`remote::invite_url(address, &identity, &window.code)`; the QR, the
text and "Copy invite" carry it. The hint becomes "Scan with a phone's
camera and accept the certificate warning in its browser, or type the
key and the code on the headset. Grants <scope> scope. Open for five
minutes or three wrong codes." Without a local address there is no QR
and a line says the Mac is off the network; when `listen.lan` is off
a line says the port answers this Mac alone and points at the
checkbox. The dialog's closing sentence (`:71`) gains: "A browser
cannot check the key: accept the certificate only on a network you
trust, since a machine answering at this address during the window
would receive the code." No new script line: `pairing <code>` and
`remote-listen lan <port>` already put an instance into the state.

## The tests

`src/adapters/remote.rs`: `a_request_reads_with_its_query_bearer_and_body`
gains the `Cookie`, `Origin` and `Host` lines; a new case checks a
reply with headers is written with them and without a second
`Content-Type`.

`tests/remote.rs` (the `http` helper grows a sibling that takes extra
request headers and returns the response headers; `subscribe_at` takes
a header list):

- `the_page_is_served_unpaired_with_its_headers`: the three paths
  answer `200` without a token with the content type, the CSP, `no-store`,
  `nosniff` and `no-referrer`; `/favicon.ico` and `/other` are `404`;
  the sixty-first load in a minute is `429`; every `href` and `src`
  in `web/index.html` is one of the two files, every `/api/v1/...`
  literal in `web/app.js` is in the test's list of the routes the page
  uses, and each of those answers a paired request with something
  other than `not_found`, so the page names no path the host lacks.
- `a_browser_pairs_from_the_url_and_holds_its_token_in_a_cookie`: the
  URL from `invite_url` is parsed as the page parses it; `/pair` with
  `transport: "cookie"` answers no `token` and a `Set-Cookie` with
  every attribute of decision 2 and the id-suffixed name; `/waiting`
  with the cookie alone is `200`; with `Origin: https://10.0.0.9` it
  is `403 wrong_origin`; the events socket with the cookie gets
  `hello`; the device is listed under the browser's name;
  `RevokeDevice` closes the socket and the next cookie request is
  `401` with a clearing `Set-Cookie`; a bearer request from the same
  device is unaffected throughout.
- `write_fixtures`: the fixture set is unchanged, since no view
  changes; `info.json` is rewritten for the `web` capability.

`tests/ui.rs`, `remote_devices_dialog_opens_pairing_and_revokes_a_device`
(`:7424`): the copied invite parses as the URL with the code, key and
id, not as JSON.

`client/Tests/SwitchboardProtocolTests/FixturesTests.swift`:
`testInviteParsesTheURL` beside `testInviteParsesTheQRText`, and the
JSON test stays.

The acceptance walk (scan, warning, sessions, a session moving to the
top on `changed {waiting}`, a reply arriving, revoke) is done by hand
with `SWITCHBOARD_REMOTE_PORT`, `remote-listen lan` and `pairing` in a
script against a test data directory, and noted in the commit.

## Docs

`docs/remote-client.md`: §4.1 adds the page window; §4.3's table adds
the three paths with scope "none"; §5 adds the cookie transport, the
browser's click-through with what it does and does not buy, the URL as
the QR's payload, and "before pairing: `/info`, `/pair` and the page".
`docs/design.md`, the remote client status: a bullet "A browser page
on the port (2026-10-22)" and the known gaps (no tickets, no
conversation beyond the last response, no drafts or dictation, no runs
or files, nothing off the local network). `README.md`: Layout rows for
`web/` and `src/web.rs`, the `SWITCHBOARD_REMOTE_PORT` dev-aid
paragraph saying "Copy invite" copies a URL a phone's browser opens.
`CHANGELOG.md` under Unreleased, Added: "The remote port serves a
browser page: scan the pairing QR with a phone, accept the certificate
warning, and see the sessions with the waiting ones first, a session's
last response or screen, and a reply box under the control scope."

## Commit

`Serve a browser control surface on the remote port, paired from the QR's URL`

Contract additions (none to `model.rs`, `action.rs` or `ports/`):

- `adapters::remote::RemoteRequest`: `cookies`, `origin`, `host`.
- `adapters::remote::RemoteReply`: `headers`; `RemoteReply::json`.
- `adapters::remote::invite_url`, `PAGE_PER_MINUTE`.
- `web::{Page, PAGES, page, headers}`.
- Wire: `GET /`, `GET /app.css`, `GET /app.js` (no scope); `/pair`
  body `transport: "cookie"` and the `Set-Cookie` reply without
  `token`; cookie authentication with the `Origin` check and
  `wrong_origin`; `info` capability `web`.
- Swift: `PairingInvite.parse` accepts the invite URL.
