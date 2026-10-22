# Plan: a web control surface on the remote port (#230)

All file and line references were checked against the worktree at
Release 0.4.1 (branch `dispatch/230-a-web-control-surface-the-remote-port-se`).
Schema versions as they stand: `SCHEMA_VERSION` 16 (`src/core/model.rs:12`),
`VIEWS_SCHEMA_VERSION` 7 (`src/core/model.rs:612`), `IDENTITY_SCHEMA_VERSION` 1
(`src/ports/remote.rs:17`), `REMOTE_PROTOCOL` 1 (`src/app.rs:4252`). This plan
bumps none of them.

## Where this departs from the issue

1. **The card's one-line excerpt is not on the wire, and `last_message` is not
   on the record.** The issue (and `docs/remote-client.md:207`) says the
   excerpt is the Stop hook's `last_message`. The hook helper writes it
   (`src/bin/switchboard-hook.rs:37`) and `EventKind::Stopped` carries it
   (`src/ports/events.rs:24`), but `src/core/events.rs:132` reads only
   `pending` from that event and drops the message; `SessionRecord` has no such
   field and `wire::SessionView` (`control/src/lib.rs:637`) has none either.
   The Mac's card derives its excerpt in the UI from the parsed transcript
   (`agent_excerpt`, `src/ui/cards.rs:558`: the first non-empty line of the
   last turn's final text, cut at 160 characters) or from the pane caption.
   A new field would need either a transcript parse on the frame or a new
   record field, so the plan adds neither. The page shows the card word and
   `waiting_reason` for every session, and for each waiting Claude Code
   session (at most eight) fetches the existing `last-response` and shows its
   first line, cut the way the card cuts it. Other kinds show no excerpt in
   this cut. The spec sentence at `docs/remote-client.md:207` is corrected to
   say this.
2. **The device's scope is not returned by any authenticated read.** The issue
   hides the reply box "when the device has `control` scope", but `/pair`
   returns the scope once (`src/app.rs:3662`) and a page reloaded with only a
   cookie has nothing to read it from. The plan adds `GET /api/v1/device`
   (name, scope, server name), behind auth.
3. **The certificate has no subject alternative name.** `make_identity` builds
   it from `CertificateParams::new(Vec::new())` (`src/adapters/remote.rs:102`),
   so the exception a browser stores covers a name mismatch as well as an
   unknown issuer, and the issue's "accept the certificate warning" is one
   warning with two reasons. Native clients pin the SPKI and never look at
   names, so nothing so far has needed one. The plan does not add SANs (see
   Decisions, 1) and gates on a real-phone check instead.
4. **"Any path outside `/api/v1/` is refused" is enforced on the frame**
   (`serve_remote`, `src/app.rs:3023`), and the page cannot be served there:
   a page load would take slots in the 64-deep inbox and the 8-per-pass drain
   that paired devices' requests need. The static table is answered on the
   connection thread instead, closed at compile time, so `serve_remote` and
   its allowlist do not change. Spec §4.1 ("handlers are thin") and §4.3's
   "everything else is refused" are amended to name the table.
5. **The device's name needs no host work.** The issue lists the user agent as
   an input; `/pair` already reads `device_name` from the body and
   `new_device` already trims and caps it at 64 characters
   (`src/app.rs:3650`, `src/core/remote.rs:169`). The page computes
   `Safari on iPhone` and posts it.
6. **The QR falls back to JSON in one case.** The URL needs a reachable host.
   The pairing window can be open while the port is loopback-only
   (`Listen::lan == false`), where today's payload still lists
   `local_address()` although nothing listens there. The URL is produced only
   when `listen.lan` is set and an address exists; otherwise the JSON form is
   produced as now. The existing UI test (`tests/ui.rs:7424`), which parses the
   copied invite as JSON on a loopback identity, keeps passing unchanged.

## Decisions

1. **Browser trust: the click-through is the trust step; no certificate
   download.** A download means a configuration profile on iOS and a Keychain
   import on macOS, which asks the owner to install a trust anchor on a phone
   (worse than the problem), needs validity under 398 days, `serverAuth` EKU and
   SANs for the address, and has to be served by the unauthenticated port. What
   the click-through buys: the channel is encrypted to whoever answered, and the
   browser remembers the exception for that address. What it does not buy:
   authenticity of the first page. The `key` in the URL makes a genuine page
   refuse to send the code to a host whose `/info` presents another key, but
   an impostor's page can simply omit the check; the protection against that
   is the QR being out of band (only the owner's screen shows it), a 40-bit
   single-use code, three wrong attempts globally, and a five-minute window
   (spec §5). Two things make the owner's eye part of the trust: the dialog
   says so in a sentence under the QR, and the page prints the host key in the
   same four-by-four grouping the dialog uses (`short_hash`,
   `src/ui/remote.rs:303`) so the two can be compared. Spec §5's rule is kept:
   before pairing, a non-loopback bind serves `/info`, `/pair` and the three
   page files, nothing else. No SANs and no reissue: the certificate never
   changes, so the exception survives an address change. **Gate:** before
   building the page, open `https://<LAN address>:43187/api/v1/info` on a real
   iPhone and a laptop browser with the current build. If either refuses to
   offer the click-through because of the name, the first step of the work
   becomes SANs for `localhost`, `127.0.0.1` and the current LAN address,
   reissued from the stored key (the SPKI and so every pin is unchanged;
   `identity.json` keeps schema 1), and the plan's other parts follow.
2. **Token transport: an `HttpOnly; Secure; SameSite=Strict` cookie.** `fetch`
   and `new WebSocket` both send it with no script access, so the page holds no
   token at all and `localStorage` holds none; a `Sec-WebSocket-Protocol`
   token would sit in script memory and in a header proxies log. Costs, all in
   scope:
   - The cookie is named `sb_<first 8 hex of server_id>`. Browsers scope
     cookies by host, not port, so two installations on one machine would
     otherwise overwrite each other's cookie.
   - `Path=/; Max-Age=34560000` (400 days, the longest Chrome keeps). The token
     itself does not expire; revocation is the control; an expired cookie is a
     re-pair.
   - Cookie authentication opens cross-site request forgery and cross-site
     WebSocket hijack that header tokens never had, and `SameSite=Strict`
     does not separate two pages on one host with different ports. So on a
     cookie-authenticated request that is not a `GET`, and on the upgrade, the
     host requires `Origin` to equal `https://<Host header>`; a missing or
     different `Origin` is refused 403 `origin`. A bearer-header request skips
     the check, and bearer wins when both are present.
   - `/pair` sets the cookie only when the body carries `"web": true`, and then
     omits `token` from its JSON so the script never sees it. Without the
     flag the reply is exactly what it is now (native clients, the CLI).
   - **Gate:** after steps 1 to 3 below, pair once from the real phone and
     confirm the cookie is kept and sent over the self-signed exception. If
     Safari drops it, the fallback is `Sec-WebSocket-Protocol` plus the token in
     `sessionStorage`; that is a separate plan, not an in-flight change.
3. **One QR, one URL.** `https://<address>:<port>/?code=<code>&key=<spki_sha256>&id=<server_id>`
   is the QR and "Copy invite" when `listen.lan` and an address exist
   (departure 6). Everything in it is URL-safe: a Crockford code, hex, a UUID.
   `PairingInvite.parse` and the CLI accept the URL or the JSON; JSON parsing
   stays so an old host's pasted text still pairs a new client. An old CLI
   cannot read the new URL; that is the one forward break, said in the
   changelog. The headset's Bonjour and typed-code path needs nothing.
4. **Static paths.** A closed table in the new `src/adapters/web.rs`, matched
   on method and exact path, answered on the connection thread:

   | Path | Content-Type |
   |---|---|
   | `GET /` (any query) | `text/html; charset=utf-8` |
   | `GET /app.js` | `text/javascript; charset=utf-8` |
   | `GET /app.css` | `text/css; charset=utf-8` |

   Any other method on those three is 405; any other path outside `/api/v1/`
   falls through to the frame and is 404 as today; the page links no favicon
   (`<link rel="icon" href="data:,">`). Every static answer carries
   `Content-Security-Policy: default-src 'none'; script-src 'self'; style-src 'self'; connect-src 'self'; img-src 'self' data:; base-uri 'none'; form-action 'none'; frame-ancestors 'none'`,
   `Cache-Control: no-store`, `X-Content-Type-Options: nosniff`,
   `Referrer-Policy: no-referrer` (the URL briefly carries the code) and
   `Connection: close`. `Connection: close` is the pool rule: a static answer
   never leaves a connection parked in the pool of four, and the page's first
   authenticated `fetch` opens a connection that moves to the paired pool as
   it does today. Static requests count against their own sliding window,
   120 a minute per listener (`Limits`, `src/adapters/remote.rs:623`), answered
   429 past it. `connect-src 'self'` should cover the same-origin `wss:`
   socket; Safari before 15.4 did not match it, which no test here can show
   (known risk, checked on the phone). With no code and no cookie the page
   says "Open the pairing window on the host and scan its QR code" and nothing
   else; no request beyond `/info` is made without one of them.
5. **Where the page lives, how it is tested:** `web/index.html`, `web/app.js`,
   `web/app.css`, embedded with `include_str!` in `src/adapters/web.rs`. No
   build step, no dependency. Tests are listed under Tests.
6. **A browser is a paired device.** Name from the user agent, scope from the
   pairing window, listed and revocable in the dialog. No new record.

## The record

No change. `identity.json`, `devices.json` and every workspace file are
unchanged; `model.rs` is untouched, so no `SCHEMA_VERSION` bump and no
`migrate` step (if a record change turns up during the work, the bump is the
value in `model.rs` or `store.rs` at that time plus one, since a ticket landing
in parallel may take the next number first). The host still stores only
the token's SHA-256; the cookie's value is the bearer token itself, so
`authenticate` looks it up by `token_sha256` as it does now and `RevokeDevice`
(`src/core/remote.rs:86`) already kills it on the next request. The page
persists only `{server_id, spki_sha256, paired}` in `localStorage`, a
per-viewer convenience that may come back empty; it lets the page say
"revoked" or "identity changed" instead of a generic error. Everything the page
shows is refetched on load (spec §7: bodies in memory only).

## The signals

None new. The page consumes the existing `changed` and `removed` events and
the revision rule of spec §4.4: open `/api/v1/events` first, buffer events
until the snapshot reply arrives, drop any reply older than the revision
already applied, and refetch everything on a new `epoch`. A session starting
to wait arrives as `changed {waiting}` or `changed {session}`, and the page
refetches `waiting` and the affected project's sessions. It sends `watch` for
the open session's `conversation` or `screen` and `unwatch` on close. A
revocation closes the socket; the page then calls `/device`, gets 401 and
shows "revoked". `/info` is read once per load and backed off on 429 (it
shares the 60-a-minute window with every other load).

## The actions and effects

No `AppAction` or `Effect` variant: no core state moves. The additions are to
the port's HTTP surface and its adapter.

- `RemoteRequest` (`src/adapters/remote.rs:360`, filled by `read_request` at
  `:444`) gains `host`, `origin` and `cookies` (only cookies whose name starts
  `sb_`, so no other cookie is ever kept or logged with the request).
- `RemoteReply` (`:389`) gains `set_cookie: Option<String>` and derives
  `Default`; the ten literal construction sites in `src/app.rs` (1664, 1679,
  2365, 2371, 2376, 2754, 3570, 3723, 3736, 3767) and the one in the adapter
  (829) take `..Default::default()`. `respond` (`:530`) writes the cookie when
  set and adds `Cache-Control: no-store` to JSON answers (bodies are private;
  nothing was setting any cache header).
- `connection` (`:770`) checks `web::asset(method, path)` before
  `shared.limits.limited`: a hit is rate-checked on the new window and answered
  with `respond_static`, then the connection closes. A non-GET to a table path
  is 405.
- `authenticate` (`src/app.rs:3127`) reads the bearer, else the cookie named
  from `identity.server_id`, then applies the `Origin` rule above to a cookie
  request that is not a `GET` or is an upgrade.
- `remote_pair` (`:3606`): `"web": true` sets the cookie and drops `token` from
  the body; the key and code checks are untouched.
- `serve_remote` (`:3017`): one new arm, `("GET", ["device"])` after
  `authenticate`, answering `{name, scope, server_name}` through `envelope`.
  Added to the endpoint table with scope "read".
- `src/core/remote.rs`: a pure `pairing_invite(identity, code, address)` that
  returns the URL (`SocketAddr` display, so IPv6 is bracketed) when
  `identity.listen.lan` and an address exist, else the JSON as now. The UI
  stops building the payload itself (`src/ui/remote.rs:160`).
- Swift: `PairingInvite.parse` (`client/Sources/SwitchboardProtocol/Info.swift:90`)
  accepts `https://host:port/?code&key&id` through `URLComponents` and fills
  `addresses` with `host:port` (re-bracketing a v6 host); JSON stays.
  `main.swift:107` and `BrowserView.swift:106,883` call `parse` already and
  need nothing else.

Order of work, the app running and `cargo test` green at each step:

1. Adapter plumbing: `RemoteRequest` fields, `RemoteReply.set_cookie`,
   `respond` changes, `web.rs` with a placeholder `index.html`, the table,
   `connection`'s static branch and window. Existing tests pass untouched.
2. Host: cookie auth with the `Origin` rule, web `/pair`, `/device`. Pair from
   the phone; this is the cookie gate in Decision 2.
3. The page in `web/`.
4. `pairing_invite`, the dialog sentence, the Swift parse and its fixture.
5. Docs.

## The UI

On the host: the pairing window (`src/ui/remote.rs:141`) draws the QR from
`pairing_invite`, "Copy invite" copies the same text, and the hint under the
code gains a sentence: "A phone's browser warns about the certificate once;
continuing is how you trust this Mac, so check that the key the page shows
matches the key above." No new widget, and no change to the paired-devices
grid (`devices`, `:230`): a browser is a row like any other.

On the page (plain JS, DOM built with `createElement` and `textContent` only,
no `innerHTML`, no inline script or style):

- Header: host name, the device's scope, the host key in four-digit groups.
- Sessions grouped by project, waiting ones first and marked; each row has
  kind, card word, `waiting_reason`, and for a waiting Claude Code session
  the excerpt of departure 1.
- Session view: the last response for a Claude Code session, the screen for
  anything else, both refreshed on `changed`; the reply box posts
  `POST /api/v1/sessions/{id}/send` with an `op` from `crypto.randomUUID()`
  and follows `/ops/{op}`; an `interrupted` op is never resubmitted (spec
  §4.5). The box is absent when `/device` says `read`; a 403 `scope` re-reads
  `/device`.
- States said plainly: not paired, pairing, "identity changed, pair again"
  (same `server_id`, different key; nothing is sent), "revoked or forgotten,
  pair again" (401 after the page had paired), host protocol higher than 1.
- First lines of `app.js`: read the query, `history.replaceState` it away, then
  compare `key` with `/info`'s `spki_sha256`, post `/pair` only on a match.

No UI test drives a browser. `app.js` is kept small, with those first lines
first, and the manual run below covers them.

## The tests

`tests/remote.rs`, over the real TLS listener. The `http` helper (`:245`)
returns status and body only; it gains a sibling that takes extra request
headers and returns response headers, and `http` calls it.

- Each static path answers 200 with its content type, the CSP above,
  `no-store`, `nosniff`, `no-referrer`; a `POST /` is 405; `/index.html` and
  `/favicon.ico` are 404; 130 `GET /` in a minute end in 429 for the window
  while a paired request on another connection is still answered.
- Static requests never reach the frame: a flood of them leaves the 64-slot
  inbox free (a paired metadata request is answered in one `serve_pending`).
  Four strangers holding connections (existing pattern, `:1394` neighbourhood)
  do not stop a page load, because static answers close.
- The embedded page: `web/index.html` and `app.js` reference only `/app.js`,
  `/app.css` and `/api/v1/...`; no `http://` or `https://` literal, no inline
  `<script>`, no `on*=` or `style=` attribute, no `innerHTML`; every
  `api("METHOD", "/api/v1/...")` literal in `app.js`, with `${...}` replaced by
  ids from `seed()`, is requested with a paired token and answers anything but
  the allowlist's 404 `not_found` endpoint.
- Pairing as the CLI does, with the host's own client: open the window,
  `GET /?code=...` (200, the static page), `GET /api/v1/info`, compare keys,
  `POST /pair` with `"web": true`; assert `Set-Cookie` has `HttpOnly`,
  `Secure`, `SameSite=Strict`, `Path=/`, the name `sb_<id8>`, and that the body
  has no `token`; then `GET /api/v1/waiting` with only that cookie. The same
  `/pair` without the flag returns the token in the body and no cookie.
- Cookie authentication: a `POST .../send` with a wrong or missing `Origin` is
  403 `origin` and sends nothing to the pane; with the right one it is typed
  once; the same `GET` needs no `Origin`; the upgrade is refused on a wrong
  `Origin`; a bearer request skips the check; a cookie of another `server_id`
  is ignored; after `RevokeDevice` the next cookie request is 401 and the
  open socket closes; a scope lowered in the dialog makes `send` 403 `scope`.
- `GET /api/v1/device` returns name, scope and server name, 401 unpaired.
- Core test in `src/core/tests.rs`: `pairing_invite` gives the URL on a LAN
  identity with an address, round-trips its three query items, brackets an
  IPv6 address, and gives the JSON on a loopback identity or with no address.
- Fixture: `write_fixtures` (`tests/remote.rs:1587`) writes `invite.json`
  (`{"url": ...}` from fixed values); a Swift test in
  `client/Tests/SwitchboardProtocolTests/FixturesTests.swift` parses it and the
  JSON form. The Swift tests are not in the pre-commit hook or CI (design.md
  A2); run `swift test` in `client/` by hand.
- Not testable here: the `replaceState`-first ordering, the key comparison,
  cookie acceptance on Safari, `connect-src` for `wss:`. Manual run per
  CLAUDE.md "Driving the live app" with `SWITCHBOARD_DATA_DIR`,
  `SWITCHBOARD_TMUX_SOCKET` and `SWITCHBOARD_REMOTE_PORT` on test values and
  `remote-listen lan <port>` plus `pairing <code>` in a script; then the
  issue's acceptance on a real phone: scan, accept the warning, see the
  sessions, watch one start waiting and move up, send a reply, revoke in the
  dialog and see "revoked" on the next request.

Docs in the same commit:

- `docs/remote-client.md`: §4.1 (static answers on the connection thread, the
  120-a-minute window, `Connection: close`), §4.3 (the three page paths, the
  `/device` row, the corrected excerpt sentence at `:207`, the allowlist
  paragraph naming the table), §5 (URL form of the QR, the click-through as the
  trust step with its limits, cookie transport, the `Origin` rule, the token
  line that says "only as a bearer header"), §11 (the tests above), §13 (a
  "Web page" decision row and the Authentication row).
- `docs/design.md`, "Remote client": a new status bullet for the web surface
  with its known gaps (no SANs, Safari `wss:` and cookie behaviour unverified
  in CI, no excerpt for non-Claude kinds, no tickets or conversation).
- `README.md`: Layout rows for `web/` and `src/adapters/web.rs`; the
  `src/adapters/remote.rs`, `src/core/remote.rs`, `tests/remote.rs` rows; the
  dev-aid paragraph at `:267-279` (Copy invite is a URL on the network, and a
  phone opens it).
- `client/README.md`: the CLI takes the URL.
- `CHANGELOG.md` Unreleased: the page, the QR as a URL, and that an older
  `switchboard-remote` cannot read the new invite.

Commit message: `A web page on the remote port, paired from a link in the QR code`

Contract additions (copy into the commit message body):

- `RemoteRequest`: `host`, `origin`, `cookies` (`sb_`-named only).
- `RemoteReply`: `set_cookie: Option<String>`; derives `Default`.
- `src/adapters/web.rs`: the closed static table (`/`, `/app.js`, `/app.css`)
  and its headers; `web/` embedded with `include_str!`.
- Remote HTTP: `GET /`, `/app.js`, `/app.css` answered unauthenticated on the
  connection thread; `GET /api/v1/device` (read); cookie `sb_<id8>` accepted
  beside the bearer, with the `Origin` rule for non-`GET` and upgrade;
  `/pair` accepts `"web": true`; JSON answers carry `Cache-Control: no-store`.
- `core::remote::pairing_invite`; the QR and "Copy invite" are a URL on a
  network bind; `PairingInvite.parse` (Swift) accepts the URL.
- No `AppAction`, `Effect`, `model.rs` type or schema version changes;
  `REMOTE_PROTOCOL` stays 1.
