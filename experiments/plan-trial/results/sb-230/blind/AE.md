# Plan for #230: a web control surface on the remote port

The host serves a small page from the remote port's own listener. A
browser on the local network opens it from a URL the pairing window
shows as a QR code, pairs, and lists, reads and answers sessions
through the `/api/v1/*` endpoints the headset already uses. One commit.
Every claim about the tree below was checked in this worktree (HEAD
`7fe8949`); every claim about browsers cites the facts file
(`facts.md`, numbered `1`-`11` for Settled, `U1`-`U7` Unresolved,
`O1`-`O3` For the owner).

## Where this departs from the issue

- **"The one-line excerpt the card shows" is not on the wire.** The
  card's excerpt is computed in the UI from the parsed transcript
  (`src/ui/cards.rs:558-574`, caption or host title at `:176-180`).
  `SessionView` (`control/src/lib.rs:637-678`) has no such field, and
  the `last_message` that `docs/remote-client.md` §4.4 names does not
  exist in the tree. The list row shows what the wire has: name, kind,
  the card's state word, `waiting_reason`, and quiet time. The excerpt
  needs a core-computed field (logic does not go under `ui/`, and a new
  field moves the fingerprints in `core/revision.rs`); that is a
  follow-up, not part of this cut.
- **"The host answers with the device's scope" (`docs/remote-client.md:316`)
  is not true in the tree.** `remote_info` (`src/app.rs:3568-3599`) is
  unauthenticated and carries no scope, and no authenticated endpoint
  does. The page needs the scope to show or hide the reply box, so the
  plan adds `GET /api/v1/me`.
- **A revoked device cannot be told from a never-paired one.** Both get
  `401 unauthorized` (`src/app.rs:3129-3148`), and a browser script
  learns nothing from a refused WebSocket upgrade (facts 3). The plan
  answers a *cookie-carried* unknown token with the code `revoked`, and
  the page learns it from a `fetch`, never from the socket. Bearer
  clients keep `unauthorized`.
- **"The Safari click-through covers the page" does not extend to the
  socket** (facts 8). Safari's `wss://` does not honour a certificate
  click-through, so REST polling is the page's floor and the events
  socket is an upgrade used where it opens. The acceptance step "moves
  to the top without a reload" is met by polling in every browser and
  by `changed` events where the socket opens.
- **The pairing window's address does not match the listener when LAN is
  off.** `bind_remote` listens on `127.0.0.1` unless
  `identity.listen.lan` (`src/app.rs:2962-2966`), yet `pairing`
  (`src/ui/remote.rs:157`) always prints `local_address()`. The URL
  uses `127.0.0.1` when LAN is off and says so, so the window never
  shows an address the listener does not answer.
- **The certificate has no subject alternative name**
  (`src/adapters/remote.rs:102`), which Chrome rejects for a bare IP
  (facts 7). The plan adds IP SANs; it does not change the key or
  `spki_sha256`.
- **Open question 3 (one QR or two)**: one URL. The JSON invite stops
  being produced; the Swift parser keeps reading it for text already
  copied (see the record and the tests).

## Decisions the issue left open

**1. Trust in the self-signed certificate: the click-through is the
trust step.** The download route is not built. Reasons: (a) a download
only helps where the owner then trusts the certificate in the device's
settings, which is a larger step than the issue's "nothing to install";
(b) its main payoff is a working Safari socket (facts 8), and the page
already works without the socket; (c) whether one click-through is
acceptable for a `control` phone is the owner's question (O3). The
plan's default works whichever way O3 goes: the pairing window keeps
its current default of `read` scope (`src/ui/remote.rs:33`), and
`control` is the owner's explicit choice in the same window. If the
owner answers unfavourably, certificate download is the follow-up named
at the end.

What the click-through and the `key` parameter buy and do not: the page
refuses to post the code to an address whose `/info` key differs from
the URL's. That guards against a stale or mistyped address. It is not
authentication, because an impostor serves `/info` and the page (U3).
On a hostile local network an impostor that the owner clicks through
can take one code. That code is single-use, open five minutes, three
wrong attempts (`docs/remote-client.md` §5), and grants the scope the
owner chose. The dialog says this in two sentences. §5's rule stands:
before pairing, a non-loopback bind serves `/info`, `/pair` and the
page's static files, and nothing else.

**2. The token is an `HttpOnly` cookie; the page's script never holds
it.** The safer place is the one script cannot read (the issue's own
reasoning), so the plan takes it and builds no `localStorage` fallback
and no `Sec-WebSocket-Protocol` path (the subprotocol also needs a 101
that echoes it, facts 2, and puts the token in script). U1 (whether a
`Secure; HttpOnly` cookie set from a click-through IP origin is stored
and sent) is unresolved, so it is the **first thing the implementer
tests**, before the rest is written: a stub `GET /` plus a `/pair` that
sets the cookie, opened in Chrome and Safari on this Mac at
`https://127.0.0.1:<port>/`, checking that a later `fetch` and a
WebSocket upgrade carry it. If the cookie is not stored in some
browser, the fallback (a bearer in `localStorage`) is a follow-up for
the owner, not a second transport built now.

Facts 5 says a cookie is not isolated by port and `SameSite` alone is
not the defence, so the host adds its own checks, applied only when the
token came from the cookie (a bearer request is unchanged):
- a non-upgrade request must carry the header `X-Switchboard-Page: 1`.
  It is not a CORS-safelisted header, so a page on another port makes a
  preflight the host never answers (`OPTIONS` reaches the allowlist's
  404);
- an upgrade request must carry `Origin: https://<Host header>`;
  `Origin` is on every browser handshake (facts 4) and checking it is
  the server's job. A missing or different `Origin` is refused 403
  `origin`.

Cookie: `__Host-switchboard=<token>; Secure; HttpOnly; SameSite=Strict;
Path=/; Max-Age=34560000` (400 days, the longest Chrome keeps). The
`__Host-` prefix bans a `Domain` and demands `Secure` and `Path=/`.
`/pair` sets it when the body says `"cookie": true` and then leaves the
token out of the JSON body; without that field `/pair` answers as
today, so the CLI and the headset are unchanged.

**3. One URL.** `https://<address>:<port>/?code=<code>&key=<spki_sha256>&id=<server_id>`.
The headset pairs from Bonjour and a typed code, and the CLI takes the
URL (below), so nothing still needs the JSON from the host. The Swift
`PairingInvite.parse` (`client/Sources/SwitchboardProtocol/Info.swift:114`)
accepts a URL and keeps accepting JSON, so an invite already on a
clipboard, and the inline JSON in `FixturesTests.swift:158-164`, still
parse. The URL has one address. The owner's question O2 (does
`local_address()` pick the one the phone reaches) has a default that
works either way: opened without a code, the page shows a pairing form
(host name, the host's key in the short form the window prints, a field
for the code), so the owner can open `https://<right address>:<port>/`
by hand and compare the key by eye, as the headset's Bonjour path does.
Listing every interface address would need an interface-enumeration
dependency and is not built.

**4. Static paths.** Served by the connection thread in
`adapters/remote.rs` before the request reaches the frame, so a page
load costs no frame pass (today a stray `GET /` costs one,
`src/app.rs:3023`). `GET` only; query ignored; exactly:

| Path | `Content-Type` |
|---|---|
| `/` | `text/html; charset=utf-8` |
| `/app.js` | `text/javascript; charset=utf-8` |
| `/app.css` | `text/css; charset=utf-8` |
| `/favicon.ico` | `image/svg+xml` (the page's one icon) |

Every other path outside `/api/v1/` still reaches the frame and gets
today's 404 `not_found`; `the_allowlist_refuses_everything_else_with_no_side_effect`
(`tests/remote.rs:682`) keeps passing. Headers on all four:
`Content-Security-Policy: default-src 'none'; script-src 'self';
style-src 'self'; img-src 'self'; connect-src 'self' wss:; base-uri
'none'; form-action 'none'; frame-ancestors 'none'` (`wss:` is there
because `'self'` has not matched `wss:` in Safari, facts 9, and the
host name is unknown at compile time); `Cache-Control: no-store`;
`Referrer-Policy: no-referrer` (U6); `X-Content-Type-Options: nosniff`.
JSON replies also gain `Cache-Control: no-store`, so authenticated data
is not left in a browser cache.

Pools. The accept loop counts a connection against the four
unauthenticated slots before it knows the path
(`src/adapters/remote.rs:701-706`), so a separate pool chosen by path
is not possible at accept time (U4 asked for one). Instead a static
reply carries `Connection: close` and the thread ends, so the slot is
held for one request and not for the 30 s `IDLE_TIMEOUT`
(`:573`). A fifth concurrent connection is dropped at once as today;
the page retries its API calls with backoff and says "host busy" (U5).
The page is three files and an icon, so one page load stays inside
four. Rate: every non-`/api/v1/` path counts against a new
`STATIC_PER_MINUTE = 120` window beside `pair` and `info`
(`Limits::limited`, `:631`), answering 429.

Without a code and without a token the page shows the pairing form
above, with a line on how to open the pairing window on the host.

**5. Where the page lives and how it is tested.** `web/index.html`,
`web/app.js`, `web/app.css`, `web/icon.svg`, embedded by `include_str!`
in a new `src/adapters/web.rs` that holds the table above. No
dependency, no build step. Tests are under "The tests".

**6. The device record.** A browser is a `PairedDevice` like any other
(`src/ports/remote.rs`, `devices.json`); the page sends its own
`device_name` ("Safari on iPhone", "Chrome on Mac"). The name is built
in the page's JS, not in Rust, so no user-agent crate is added; the
page sends "iPad" when `navigator.maxTouchPoints > 1` on a Mac user
agent (facts 10). The host trims it to 64 characters as today
(`src/app.rs:3650-3655`). Scope is the window's, listing and Revoke
are the existing dialog.

## The record

- `devices.json`: nothing new. It is a bare `Vec<PairedDevice>` with no
  version (`src/adapters/remote.rs:80-82`); a browser is one more row,
  and the token is never stored (`src/core/remote.rs:166`).
- `identity.json`: one defaulted field, `addresses: Vec<String>`, the
  IPs the certificate was issued for. At `bind_remote`
  (`src/app.rs:2936`), if the addresses it would serve (`127.0.0.1`,
  and `local_address()` when `listen.lan`) are not all in
  `identity.addresses`, the certificate is reissued over the same key
  (already read from the Keychain there), with those IP SANs, and
  `Effect::SaveIdentity` saves it. `spki_sha256` and the device list
  are untouched. An older `identity.json` reads `addresses` as empty,
  so its first bind reissues; no migration is needed. Bump
  `IDENTITY_SCHEMA_VERSION` (`src/ports/remote.rs:17`, 1 today) to the
  value at implementation time plus one, because a ticket landing in
  parallel may take the next number first. `SCHEMA_VERSION`
  (`src/core/model.rs:12`, 16 today) and `VIEWS_SCHEMA_VERSION`
  (`:612`, 7 today) do not change; nothing in `model.rs` is touched.
- A changed network address after bind is not followed until the next
  bind (a relaunch or toggling LAN); Chrome still offers "Proceed" for
  a certificate that lacks the new address (facts 7). Named as a known
  gap in the status section.
- The page keeps `{server_id, spki_sha256}` in `localStorage` after a
  pairing. These are public values; the page uses them only to say
  "identity changed, pair again". A page that cannot read storage
  (private window) works without that one message. The token is not in
  storage.
- Rehydration: a reopened page runs `GET /info`, `GET /me`, then the
  reads, and opens the socket if it can; the page's state is UI state
  only (`docs/remote-client.md` §12.6).

## The signals

No new event kinds. The page consumes `hello`, `changed` (kinds
`session`, `sessions`, `waiting`, `project`, `projects`, `conversation`,
`screen`), `removed` and `resync_required` as §4.4 defines them, and
sends `watch`/`unwatch` for the open session's content key. Replies are
applied by `revision` as §4.4 says: events buffer from `hello`, a
snapshot's `revision` decides which buffered events still apply.

New refusal codes: `revoked` (401, cookie token unknown), `origin`
(403, upgrade with a missing or foreign `Origin`), `page_header` (403,
cookie request without `X-Switchboard-Page`). `GET /api/v1/me`
(authenticated, `read`) answers `{device_id, name, scope}`; `info`
adds `"me"` and `"web"` to `capabilities`
(`src/app.rs:3580`).

Polling floor: the page refetches the project and session lists and the
open view every 5 s while the tab is visible, and the socket, where it
opens, replaces that with refetch-on-`changed` plus the 30 s refetch
§4.4 already requires. A socket that fails to open (Safari after a
click-through, U2 for Chrome) is not an error state; the page keeps
polling and shows no warning.

## The actions and effects

- `AppAction::CertificateReissued { certificate, addresses }` (core,
  `src/core/remote.rs`): applied through the existing
  `update_identity`, so it emits `Effect::SaveIdentity` only when the
  value changed. `bind_remote` dispatches it *after* it sets
  `remote_bound`, so `run_remote_effect` (the `SaveIdentity` arm) sees the listen preference unchanged and does not
  rebind. No new effect.
- `adapters::remote::make_certificate(identity, key_pem, addresses)`:
  the reissue, built from the existing `rcgen` parameters, with
  `SubjectAltName::IpAddress` entries. `make_identity` calls it too, so
  a new identity starts with the right SANs.
- `adapters/web.rs`: `pub fn lookup(path) -> Option<Asset>`, a pure
  table.
- `adapters/remote.rs`:
  - `RemoteRequest` gains `cookie: Option<String>` (the
    `__Host-switchboard` value), `origin`, `host` and `page: bool`
    (`read_request`, `:444`). The cookie value is taken out of the
    headers like the bearer is, so it is never logged.
  - `RemoteReply` gains `headers: Vec<(String, String)>` (only
    `Set-Cookie` uses it). It is built as a literal about 30 times (28 matches in `src/app.rs`,
    one in `adapters/remote.rs`); each gets `headers: Vec::new()`, a
    mechanical edit.
  - `respond` takes a content type and extra headers; the
    JSON path passes `application/json` and `Cache-Control: no-store`.
  - `connection` (`:785`) answers a static `GET` before the rate check
    for `/api/v1/` paths, applies the `STATIC_PER_MINUTE` window to
    non-API paths, and ends after a static reply.
- `src/app.rs`:
  - `authenticate` (`:3127`) takes the token from the bearer, else from
    the cookie; for a cookie it enforces the `X-Switchboard-Page` /
    `Origin` rules above; an unknown cookie token is `revoked`, no
    token at all is `unauthorized`.
  - `remote_pair` (`:3606`): with `"cookie": true` it builds the
    `Set-Cookie` value, leaves `token` out of the body, and otherwise
    behaves as today. The key check before the code check
    (`:3636`) is unchanged.
  - `remote_me` for `GET /me`, routed beside `info`/`pair` after
    `authenticate`.
  - `bind_remote` does the reissue described under the record.
- Swift: `PairingInvite.parse` accepts the URL (query `code`, `key`,
  `id`, and the authority as the one address) and still the JSON; the
  CLI's one-argument `pair` (`main.swift:110`) needs no change because
  it calls `parse`. The headset's paste (`BrowserView.swift:106, 883`)
  gets the URL form from the same `parse`.

## The UI

- The host's pairing window (`src/ui/remote.rs`, `pairing` at `:141`):
  the QR and "Copy invite" carry the URL. The text beside it keeps the
  short key and code, and says "Scan with a phone's camera, or open
  `https://<address>:<port>` and type the code." With LAN off the
  address is `127.0.0.1` and the line adds "this Mac only; allow
  devices on the local network above for a phone." Two sentences state
  the trust step from decision 1, including that the code stays in the
  camera app's history (U6) and that live updates in Safari may need the
  certificate trusted on the device (facts 8).
- The page (`web/`), plain DOM and `textContent` only, with no
  `innerHTML`, no `eval`, no inline script or style, and no
  secure-context-only API (`crypto.randomUUID` is replaced by
  `crypto.getRandomValues` for the send `op`, since secure-context
  status of an IP origin is unknown, U7).
  1. First statement: read `location.search`, then
     `history.replaceState(null, '', location.pathname)`, before any
     request.
  2. `GET /info`. With an invite, compare its `key` and `id` with
     `/info`; on a mismatch say "This address is not the host whose
     code was scanned" and post nothing. On a match
     `POST /pair {code, spki_sha256, device_name, cookie: true}`; keep
     `{server_id, spki_sha256}` in storage.
  3. Without an invite: `GET /me`. 200 is the list. `unauthorized` is
     the pairing form. `revoked` is "This device was revoked; pair
     again". A stored key that differs from `/info`'s is "Identity
     changed, pair again", and nothing authenticated is sent.
  4. The list: `GET /projects`, then `GET /projects/{id}/sessions` for
     each, grouped by project, waiting sessions first and marked, each
     row showing name, kind, state word, `waiting_reason` and quiet
     time. Header: host name from `/info`, device scope from `/me`.
  5. A session: a Claude Code session (`kind: "claude"`) shows
     `GET /sessions/{id}/last-response`; any other kind shows
     `GET /sessions/{id}/screen?lines=60`, in a monospace block. A
     reply box is shown only with `control` scope; it posts
     `POST /sessions/{id}/send {op, text}` with one `op` per message,
     re-sent with the same `op` only after a timeout (`docs/remote-client.md`
     §4.5, `/ops/{op}` for the status). A 403 `scope` re-reads `/me` and
     hides the box.
  6. Layout: one column at phone width, no horizontal scroll, light and
     dark by `prefers-color-scheme`.

Not in the page: tickets, decisions, conversation history, drafts,
dictation, runs, pinned files.

## The tests

- `tests/remote.rs` (raw TLS client; `http()` at `:245` extended to
  return headers and take an extra header list, since the cookie flow
  needs them):
  - each static path answers 200 with its `Content-Type`, the CSP,
    `Cache-Control: no-store`, `Referrer-Policy` and `nosniff`; it
    answers with a query string; `POST /` and `GET /nothing` still 404
    JSON; the connection is closed after a static reply.
  - the embedded `index.html` references only paths in the table, the
    JS contains no `innerHTML`, `eval` or `localStorage` read of a token,
    and every `/api/v1/...` literal in `app.js` is in an allowed list.
  - the pairing flow as the CLI does it: `GET /info`, then `POST
    /pair` with `"cookie": true` and the key from `/info`; the reply has
    `Set-Cookie` with `__Host-`, `Secure`, `HttpOnly`,
    `SameSite=Strict`, `Path=/`, no `Domain`, and no `token` in the
    body; the window is single use as today.
  - with that cookie: `GET /me` with `X-Switchboard-Page` answers the
    scope; without the header 403 `page_header`; a bearer request is
    unchanged; an upgrade with a foreign or missing `Origin` is 403
    `origin`, with the right `Origin` it gets `hello`; a send under
    `read` scope is 403 `scope`; after Revoke the same cookie gets 401
    `revoked` and the socket closes; no cookie gets `unauthorized`.
  - the static rate window answers 429 past `STATIC_PER_MINUTE`, and
    ten page loads in a row leave the unauthenticated pool empty.
- `src/adapters/remote.rs`: a unit test that a reissued certificate
  carries the `subjectAltName` extension and the address bytes, and
  that its `spki_sha256` equals the original's.
- `src/core/tests.rs`: `CertificateReissued` emits `SaveIdentity` once
  and nothing for an identical value, and does not close a pairing
  window or touch devices.
- `tests/ui.rs`: the pairing window's "Copy invite" copies the URL, the
  text names the address, and with LAN off the window says so.
- Swift (`swift test` in `client/`): `PairingInvite.parse` of the URL,
  of the URL with an IPv6-free authority and percent-escapes, and of the
  JSON as before. No fixture changes: no Swift decoder reads `/me`.
- No test drives a browser. The implementer runs the page by hand in
  Chrome and Safari on this Mac against `https://127.0.0.1:<port>/`
  with `SWITCHBOARD_DATA_DIR` and `SWITCHBOARD_TMUX_SOCKET` on test
  locations: cookie stored and sent (U1), the socket after the
  click-through (U2), the code gone from the address bar and history,
  a session turning waiting moving to the top, a reply arriving, and
  Revoke showing "revoked" on the next request. The phone steps (O1)
  are the owner's.

Docs in the same commit: `docs/remote-client.md` §4.1 (the static
paths, the extra pool behaviour and rate window), §4.3 (`/me`, the
`/pair` `cookie` field, the new codes), §5 (the cookie, the Origin and
header checks, the page's place in "before pairing", the line at `:316`
corrected, and the bearer-only wording at `:312` changed to say "or the
host-set cookie, which script cannot read"); `docs/design.md` "Trust
boundary" (`:219`) and the "Remote client" status section (`:3107`),
with the known gaps; `README.md` Layout table (`web/`,
`src/adapters/web.rs`), the Development line about "Copy invite"
(now a URL; `switchboard-remote pair "$(pbpaste)"` still works), the
data-directory entry for `identity.json`'s `addresses`; `CHANGELOG.md`
under Unreleased.

## For the owner, after this lands

- **O1** (iPhone: does the Camera open the URL in Safari, does the page
  load after the warning, does the socket open). The page polls either
  way; the answer decides whether live updates in Safari need more.
- **O2** (does `local_address()` pick the address the phone reaches).
  The manual route works either way; listing every interface needs a
  dependency and is a follow-up.
- **O3** (is one click-through enough for a `control` phone). If not,
  the follow-up is serving the certificate for download so it can be
  trusted on the device first. That follow-up also gives Safari its
  socket (facts 8).
- A follow-up if U1 fails in a browser the owner uses: a bearer in
  `localStorage` with the dialog saying so.
- A follow-up for the card's excerpt as a core-computed wire field.
- Rebinding or re-issuing the certificate when the host's address
  changes while running.

## Commit message

Serve a web control page from the remote port, paired from a URL in the pairing window

## Contract additions

- `GET /api/v1/me` (authenticated, `read`): `{device_id, name, scope}`;
  `info.capabilities` gains `"me"` and `"web"`.
- `POST /api/v1/pair` accepts `"cookie": true`; the reply then sets
  `__Host-switchboard` (`Secure; HttpOnly; SameSite=Strict; Path=/`)
  and omits `token`. Without the field the reply is unchanged.
- Cookie-carried requests: `X-Switchboard-Page: 1` required (non-upgrade),
  `Origin` equal to `https://<Host>` required (upgrade); refusals
  `page_header` (403), `origin` (403), and `revoked` (401, unknown
  cookie token). Bearer requests unchanged.
- Static `GET` paths `/`, `/app.js`, `/app.css`, `/favicon.ico` served
  by `adapters::web`; `Cache-Control: no-store` on JSON replies.
- `Identity.addresses: Vec<String>` (`#[serde(default)]`);
  `IDENTITY_SCHEMA_VERSION` from the value in
  `src/ports/remote.rs` at implementation time, plus one.
- `AppAction::CertificateReissued { certificate, addresses }`.
- `adapters::remote`: `RemoteRequest { cookie, origin, host, page }`,
  `RemoteReply { headers }`, `make_certificate`, `STATIC_PER_MINUTE`.
- The pairing invite is a URL
  `https://<address>:<port>/?code=&key=&id=`; Swift
  `PairingInvite.parse` accepts it and still accepts the JSON.
