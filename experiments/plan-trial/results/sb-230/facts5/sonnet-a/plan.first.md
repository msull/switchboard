# Plan: #230, a web control surface on the remote port

Line numbers are from the worktree at HEAD 7fe8949. "Facts" is
`facts/1/repo/facts.md`; a decision that rests on one of its lines says
which. Nothing here names a client, a customer or their repositories.

## Where this departs from the issue

- **"The one-line excerpt the card shows."** No wire field carries it.
  `SessionView` (`control/src/lib.rs:637-688`) has `card`,
  `waiting_reason`, `last_stop_at_ms` and `pending`, but no excerpt:
  `last_message` exists only on hook events (`src/ports/events.rs:24`,
  `src/adapters/hooks.rs:71`), and `grep last_message` over `src/app.rs`,
  `control/src` and `client/Sources` finds no reader. docs/remote-client.md:207
  and the Settled line in Facts that repeat it describe a field the tree does
  not have. The first cut shows the card's word (`card`) and `waiting_reason`
  on each row. The excerpt is a follow-up: an additive `SessionView` field,
  masked like `last_response` (§4.3), plus a Swift decode.
- **"The device's scope in the header."** No endpoint returns it after
  pairing. `remote_pair` answers it once (`src/app.rs:3658-3663`) and the
  sentence in §5 ("the host answers with the device's scope") has no reader.
  With the token in an `HttpOnly` cookie (decision 2) the page cannot keep a
  copy to reread after a reload, so the plan adds `GET /api/v1/me`.
- **"Removes it from the address bar ... The code never stays in the address
  bar or the history."** True of the bar and of Back (Facts, Settled,
  `history.replaceState`). Not true of the first request, which carries
  `?code=` to the host and to whatever scanned it. Also, same-origin
  subresource requests send the full page URL as `Referer`, so the plan sends
  `Referrer-Policy: no-referrer` as a header and a `<meta>`, so `app.js` and
  `app.css` are fetched with no `Referer`. The code is treated as exposed to
  the host's request line and the device's launcher, and limited by single use
  and the five-minute window. The static route never logs a query.
- **"The browser then trusts that certificate for that address."** Not
  established (Facts, Unresolved 1, 2, 5), and the certificate cannot satisfy
  a browser as made: `make_identity` builds it with no subject alternative name
  (`src/adapters/remote.rs:102`), and the page is opened at a bare IP. The plan
  gives the certificate IP and `localhost` SANs over the same key (decision 1),
  which works whether or not CN-only certificates are accepted.
- **"A name from the user agent."** The host keeps no `User-Agent`
  (`RemoteRequest`, `src/adapters/remote.rs:360-372`). The page derives the name
  (`Safari on iPhone`) and sends it as `device_name`, which `remote_pair`
  already takes (`src/app.rs:3650`).
- **Facts' suggestion of bearer-in-`localStorage` as the base** (Unresolved 2)
  is not taken. See decision 2.

## Decisions the issue asks for

### 1. Browser trust in the certificate

**The click-through is the trust step; the host does not offer its certificate
for download.** What it buys: the `key` in the URL comes from the QR on the
owner's own screen, so a genuine page refuses to send the code to a host whose
`/info` key differs (a stale QR, another Switchboard on the network). The
host also refuses a `/pair` whose `spki_sha256` is not its own
(`src/app.rs:3636`). What it does not buy: a page served by an impostor is not
a genuine page and can skip the comparison, and a browser cannot pin a key the
way the native clients do. So the page shows the short key it verified
(`short_hash` form, `src/ui/remote.rs:303`) and the pairing window says to
compare it with the Key line before accepting the warning's consequence; the
paired-devices dialog states plainly that a browser trusts the network it was
opened on.

Why not the download: it adds an unauthenticated route that serves the
trust anchor over the same untrusted channel, and on iOS it is a profile
install the owner must then enable in Settings. It is the documented fallback
if the owner's phone refuses the socket after a click-through (Facts,
Unresolved 1): the SANs added here are what it would need, and nothing else in
the plan changes. It is a follow-up, not built now.

§5's rule stands, widened by exactly one class: before pairing, a non-loopback
bind serves `/info`, `/pair`, and the three page files. Nothing else, and none
of the page files carries host data.

**SANs.** `Identity` gains `sans` (the names the stored certificate carries).
At bind (`bind_remote`, `src/app.rs:2936`, where the key is already read for
`tls_config`) the wanted set is `127.0.0.1`, `localhost`, and, when
`identity.listen.lan`, `remote::local_address()`. If it differs from
`identity.sans`, the certificate is remade over the same key
(`remote::reissue`, using `SanType::IpAddress` and `DnsName`) and saved with
`save_identity`. The SPKI hash is unchanged, so no client re-pairs (§5: "a
certificate reissued from the same key still verifies"). It is kept across
launches on purpose: a browser's click-through is remembered per certificate,
so a certificate remade on every launch would show the warning again each
time. A change of the Mac's address remakes it at the next bind and shows the
warning once more; the plan says so in the pairing window's text.

### 2. How the token reaches the WebSocket

**An `HttpOnly; Secure; SameSite=Strict` cookie.** Where the credential lives
is the security decision, and a cookie the page's script cannot read is the
safer store than `localStorage` (readable by any script on the origin) and
keeps the token out of every URL. `fetch` and `new WebSocket()` both carry it,
so one carrier serves REST and the socket and no `Sec-WebSocket-Protocol`
echo in `respond_upgrade` is built.

Facts Unresolved 2 says nobody has shown a `Secure` cookie is stored and sent
by Safari or Chrome on a click-through origin, upgrade included. That is the
first thing the implementer tests, before any other work: **step 0, a spike
`spikes/NN-web-cookie`** (next free number at implementation time; 22 is the
last today), a standalone crate copied from `spikes/22-remote-port`'s server
that serves a page over a self-signed certificate with the SANs above, sets the
cookie on a `fetch`, echoes whether the cookie and `Origin` arrived on a second
`fetch` and on a `wss://` upgrade, and a README with what each browser did. The
implementer runs it on the desktop browsers available; the owner repeats it on
the phone (an owner step, not a gate on the build's tests). **If the cookie is
not stored or not sent, stop and report it. The fallback (bearer in
`localStorage`, the token offered as a `Sec-WebSocket-Protocol` value, which
Facts' RFC 6455 line shows is legal when hex or base64url) is a follow-up, not a
second transport built behind this one.**

Mechanics:

- `POST /pair` takes an optional `"carrier": "cookie"`. With it the reply omits
  `token` (the page must never see it), keeps `scope`, `server_id`,
  `device_id`, and carries
  `Set-Cookie: sb-<first 8 hex of server_id>=<token>; Path=/api/v1/; Secure; HttpOnly; SameSite=Strict; Max-Age=34560000`.
  Without it the reply is exactly today's, so the native clients and fixtures
  are untouched. The name carries the server id because cookies are not
  isolated by port: two installations on one address (the dev aid
  `SWITCHBOARD_REMOTE_PORT`) must not overwrite each other. `Path=/api/v1/`
  keeps it off the page's own fetches.
- `authenticate` (`src/app.rs:3127`) reads the bearer header first, else the
  cookie. It adds the **Origin guard** Facts calls for (Settled, "Origin on the
  handshake"): when the credential came from the cookie and the request is not
  a plain `GET`, or is an upgrade, `Origin` must equal `https://` + `Host`.
  `SameSite=Strict` does not stop a same-site page on another port of the same
  IP. Anything else is `403 origin`. A bearer-header request is never checked
  (a browser cannot forge that header cross-site, and native clients send no
  `Origin`). `/pair` with `carrier: cookie` gets the same guard.
- `read_request` (`src/adapters/remote.rs:444`) keeps three more headers:
  `cookie`, `origin`, `host`. Same exposure as `bearer`: `RemoteRequest`
  derives `Debug`, so nothing may format a request; `refuse` logs method, path
  and code only (`src/app.rs:3754`).
- Revocation needs nothing new: `RevokeDevice` already drops the subscriber
  (`src/app.rs:486-498`) and the row, so the next request is `401`.

### 3. One QR or two

**One URL.** `https://<address>:<port>/?code=<code>&key=<spki_sha256>&id=<server_id>`
for the QR, the "Copy invite" text, the browser, `switchboard-remote pair` and
the headset's paste. The JSON is no longer emitted. `PairingInvite.parse`
(`client/Sources/SwitchboardProtocol/Info.swift:114`) keeps decoding JSON (a
copy made earlier, the three-argument CLI form is separate) and gains a branch
for `https://`: host and port become `addresses`, `code`, `key`, `id` the other
fields. Its callers are `switchboard-remote` (`main.swift:111`), the headset's
`BrowserView.swift:106` and `:883`, and `ConnectionManager.swift:474-482`
(Facts, Settled), so the one branch covers all of them. Every field is
Crockford, hex or a UUID, so nothing needs percent-encoding.

### 4. Static paths

- **Exactly** `GET /` (`text/html; charset=utf-8`), `GET /app.js`
  (`text/javascript; charset=utf-8`), `GET /app.css` (`text/css; charset=utf-8`).
  Other methods on them are 405; any other non-`/api/v1/` path keeps today's 404
  (`src/app.rs:3023`); `/favicon.ico` is that 404.
- Headers on each: `Content-Security-Policy: default-src 'none'; script-src 'self'; style-src 'self'; connect-src 'self'; base-uri 'none'; form-action 'none'; frame-ancestors 'none'`,
  `X-Content-Type-Options: nosniff`, `Cache-Control: no-store` (a page cannot
  outlive its binary), `Referrer-Policy: no-referrer`, `Connection: close`. No
  inline script or style, no external resource. `connect-src 'self'` may not
  cover `wss:` on older Safari; the polling default below is why that is not a
  failure. `respond` also gains `Cache-Control: no-store` and `nosniff` on every
  JSON reply, since authenticated data now reaches a browser cache.
- **Served on the connection thread**, like the 429 path
  (`src/adapters/remote.rs:786-794`), before the rate check reaches the frame:
  no frame slot, no `INBOX`, no 8-per-pass drain, so a page load is not queued
  behind requests (Spike 22's one-frame-interval cost). The assets are
  `include_str!` in a new `src/adapters/web.rs` (`asset(method, path)`).
- **Pool and rate.** A page load is three requests; a browser may open up to
  six connections against the pool of four (Facts, Settled, connections per
  page). `Connection: close` frees the slot at once, a connection refused at
  accept is retried by the browser (idempotent GETs), and the page is three
  files, not more. A new `Limits.page` window (120 a minute per listener, over
  the three paths) answers 429 past it; `/pair` and `/info` windows are
  unchanged.
- **Opened with no code and no cookie** the page reads `/info`, shows the host's
  name and key, says whether pairing is open, and says what to do: "On the Mac,
  open Remote devices and choose Allow pairing, then scan the code." No form for
  the code: typing is the headset's path, the browser's is the link.

### 5. Where the page lives and how it is tested

`web/index.html`, `web/app.js`, `web/app.css`, embedded at compile time. No
build step, framework or dependency. See Tests.

### 6. The device record

A browser is a row in `devices.json` like any other; its scope is chosen in
the pairing window as today, and it is listed and revoked in the same dialog.
No new field on `PairedDevice`.

## The record

- `devices.json`: no change.
- `identity.json`: `Identity.sans: Vec<String>`, `#[serde(default)]`, additive.
  `IDENTITY_SCHEMA_VERSION` (`src/ports/remote.rs:17`, 1 today) bumps to its
  value at implementation time plus one, because a ticket landing in parallel
  may take the next number first. The `certificate` field is rewritten when the
  SAN set moves; the key and `spki_sha256` are not.
- `SCHEMA_VERSION` (`src/core/model.rs:12`, 16) and `VIEWS_SCHEMA_VERSION`
  (`src/core/model.rs:612`, 7): not touched, no `model.rs` type changes.
- Rehydration: the cookie lives in the browser, the row on the host; a restart
  reloads `devices.json` and the cookie still hashes to a row. A key rotation
  clears `devices.json` (§5), so the browser gets `401`. The page's own
  `localStorage` holds only a non-secret marker `{server_id, spki_sha256, name}`
  written at pairing, used to word "revoked" and "identity changed"; it is a
  cache and may be missing (private window), in which case the page says "not
  paired".

## The signals

None new. The page consumes the existing ones (§4.4):

- Always starts polling `GET /api/v1/waiting` every 5 s. Its envelope's
  `epoch` and `revision` (every reply carries them) gate the work: unchanged
  means nothing is refetched or redrawn. Moved means refetch `projects` and each
  project's `sessions` and redraw. A `401` here is also how revocation is found.
- Then opens `/api/v1/events` (the cookie rides the upgrade). On `hello` it
  stops the 5 s poll to a 30 s safety refetch, and on `changed` of kinds
  `session`, `sessions`, `waiting`, `projects` it refetches that collection
  (subscribe first, snapshot, apply by revision, §4.4). On `resync_required`,
  an `epoch` change or a close it refetches everything and reconnects with
  backoff (1, 2, 4 ... 30 s). While the socket is down the header says "live
  updates unavailable, refreshing every 5 s". This is the default design for
  Facts Unresolved 1: a phone whose Safari refuses `wss://` after the
  click-through still works, slower, and the socket is the upgrade.
- An open session sends `watch {kind: "conversation"|"screen", id}`, fetches
  only after `watching`, refetches on that key's `changed`, and also every 30 s.
  Polling mode refetches an open session every 5 s on a revision change.
- A revoked device's open socket is closed by the host; the page's next request
  is `401` and it says "revoked". No new event.

## The actions and effects

No `AppAction` or `Effect`: pairing, revocation and `DeviceSeen` are the
existing actions, and the new routes answer from frame state.

- `src/core/remote.rs`: a pure `invite_url(identity, window, address) -> String`
  (the string `ui/remote.rs` builds today moves into the core, per "logic under
  `ui/` goes to the core"). IPv6 is bracketed by building a `SocketAddr`.
- `src/adapters/remote.rs`: `RemoteRequest` gains `cookie`, `origin`, `host`;
  `RemoteReply` gains `set_cookie: Option<String>` (the 28 `RemoteReply {` sites
  in `src/app.rs` and `:829` here take `set_cookie: None`; a
  `with_cookie` builder serves the one site that sets it); `respond` takes extra
  headers; `Limits.page`; the static branch in `connection`; `reissue`.
- `src/app.rs`: `authenticate` (cookie, Origin guard), `remote_pair` (the
  carrier), a `("GET", ["me"])` arm after `authenticate` in `serve_remote`
  (after the `authenticate` call at `src/app.rs:3032`) answering `{device_id, name, scope}` in the usual
  envelope, and the SAN check in `bind_remote`. `me` is read scope and not a
  `wire::Request`; it is added to §4.3's table, so the allowlist stays a list.
- The spec's `GET /api/v1/me` also gives the native clients the reader §5
  already promised; the Swift side does not call it in this commit.

## The UI

The pairing window (`src/ui/remote.rs:141-217`):

- The QR and "Copy invite" carry `invite_url`. The label stays "Copy invite"
  (hover: "The QR code's link, to open in a browser or paste into a client").
- The meta text reads: "Scan with a phone's camera, or open the link in a
  browser. Accept the certificate warning, then check the Key the page shows
  against this one. Grants {scope} scope. Open for five minutes or three wrong
  codes." Facts, Unresolved 4: the text says "open it in your browser" because
  whether a scanner app's webview keeps its storage is unknown; a page with no
  stored token just shows the unpaired page.
- When `identity.listen.lan` is false the address is `127.0.0.1:<port>` and a
  line says the port listens on this Mac only; turn on the local network for a
  phone. (Today it offers the LAN address regardless.)
- The paired-devices dialog (`devices()`, `:230`) gains a sentence: a browser
  cannot pin the host's key, so pairing one trusts the network it was opened on;
  revoke it here. A device named by the page appears with its name and scope as
  any other.

The page (`web/`): a header with the host's name, the device's scope and the
live state; the sessions grouped by project, waiting first and marked, each row
the session's name, kind, `card` word and `waiting_reason`; a session view
(`last-response` for Claude Code, else `screen?lines=200`) with a reply box
that posts `send` only when `me` says `control` and is otherwise absent. The
`op` is 16 random bytes in hex from `crypto.getRandomValues` (`randomUUID` is
absent on an origin the browser does not treat as secure); a retry reuses its
`op`. A `403 scope` removes the box and re-reads `me`. Plainly worded states:
unpaired, "identity changed, pair again" (the `/info` key differs from the
marker; nothing is sent), "revoked" (`401` with a marker), "cannot reach the
host" (a transport error; Facts, Unresolved 3). The device name is built from
`navigator.userAgent` and `maxTouchPoints` (an iPad presents as a Mac).
Dark and light follow `prefers-color-scheme`; the layout is one column.

## The tests

Order of work, with the app running at each step: the spike (step 0); adapter
fields and headers; static route; app (cookie, guard, carrier, `me`); SANs;
core `invite_url` and the UI; the page; Swift parse; docs.

`tests/remote.rs` (a sibling of `http()` at `:245` returns headers too, and
`http()` calls it):

- The three paths answer 200 to an unpaired client with the exact
  `Content-Type`, CSP, `nosniff`, `no-store`, `no-referrer`, `Connection: close`;
  `POST /` is 405; `/nothing` and `/favicon.ico` are 404; four sequential page
  loads never fill the unauthenticated pool; the 121st load in a minute is 429.
  No `serve_pending` is needed, since the connection thread answers.
- The embedded page is self-consistent: `index.html` references only `/app.js`
  and `/app.css`, none of the three files contains an inline `<script>` body, an
  `on…=` attribute or an `http(s)://` URL, and every `/api/v1/…` literal in
  `app.js` (ids substituted for the seeded ones) is a route the host answers
  (a GET returns anything but `not_found`; the `send` literal is checked against
  the control-scope test). This is the only check on the script's paths: the
  tree has no JS runner and the issue forbids one.
- Pairing end to end with the host's own HTTP client, as the CLI does: open the
  window, `POST /pair` with `carrier: cookie`; the body has no `token`; the
  `Set-Cookie` has `Secure`, `HttpOnly`, `SameSite=Strict`, `Path=/api/v1/` and
  the name from the server id; `GET /api/v1/me` with that cookie is 200 with
  the scope, without it 401; a bearer pairing (no carrier) is unchanged.
- Origin guard: a cookie `POST send` with no `Origin`, `null`, or another port's
  origin is `403 origin` and typed nothing; the matching origin sends once; a
  cookie upgrade with a foreign `Origin` is refused and with the right one is
  accepted; a bearer request with a foreign `Origin` is unaffected.
- Revoke the cookie's device: the next cookie request is 401 and the open
  socket closes (extends the existing revocation test).
- Unit: the cookie header parse (several cookies, a prefix of the name, none).
  In `adapters/remote.rs`: `reissue` keeps `spki_sha256`, carries the `87 04`
  IP SAN, and is a no-op when `sans` already match.
- The allowlist test (`:690-700`) still passes unchanged; a new case asserts
  `GET /api/v1/me` is not 404 and `GET /api/v1/me/x` is.

`src/core/tests.rs`: `invite_url` exact string, IPv6 bracketed, loopback.
`tests/ui.rs:7474-7492`: the "Copy invite" test parses a URL instead of JSON
(`code`, `key`, `id` equal the core's), plus the no-LAN line. Swift
(`client/Tests/SwitchboardProtocolTests/PairingInviteTests.swift`, new): the
URL and the old JSON parse to equal invites; a URL missing a field throws.
`swift test` is not in CI or the hook; README's count at line 18 ("81 tests")
is updated. No fixture is added: `me` has no Swift reader yet, so its shape is
asserted inline.

Acceptance, by the owner on a phone (nothing here claims it was run):
scan, accept the warning, see the sessions; a session that starts waiting moves
up (socket) or within 5 s (polling); a reply sent from the phone arrives;
revoke in the dialog and the page says "revoked". The owner also answers Facts'
"For the owner" 1-3 (socket or polling only, whether the cookie survives closing
the browser, which browsers).

Docs: `docs/remote-client.md` §4.1 (page files served on the connection
thread, the page rate window), §4.3 (the four rows), §5 (the invite is a URL,
the cookie carrier, the Origin guard, the widened pre-pairing list, "the token
is never in a URL" kept); `docs/design.md` a bullet in the "Remote client"
status section before "Known gaps", and the gaps (no excerpt, no sign-out
button, no Dispatch tickets); README Layout (`src/adapters/web.rs`, `web/*`, the
spike, the `tests/remote.rs` row), `CHANGELOG.md` Unreleased; no new env var or
`SWITCHBOARD_SCRIPT` line (`pairing` and `remote-listen` already reach the
state).

## Follow-ups for the owner

- The card's one-line excerpt on the wire.
- If the spike shows the cookie is refused on a click-through origin:
  `localStorage` plus bearer, and the token as a `Sec-WebSocket-Protocol`
  value. If the phone refuses `wss://`: the certificate download route.
- A sign-out for the browser (an `HttpOnly` cookie cannot be cleared by script).

## Commit message

Serve a web control page on the remote port, paired from a URL invite with an HttpOnly cookie

## Contract additions

- `RemoteRequest`: `cookie`, `origin`, `host` (`src/adapters/remote.rs`).
- `RemoteReply`: `set_cookie` (`src/adapters/remote.rs`).
- `Identity.sans` (`#[serde(default)]`); `IDENTITY_SCHEMA_VERSION` from the value
  at implementation time, plus one.
- `POST /api/v1/pair`: optional `carrier: "cookie"`; with it, no `token` in the
  reply and a `Set-Cookie` named `sb-<first 8 hex of server_id>`.
- `GET /api/v1/me` (read scope): `{device_id, name, scope}`.
- `GET /`, `/app.js`, `/app.css` served unauthenticated, on the connection
  thread, with the headers above; `Cache-Control: no-store` and `nosniff` on
  every reply.
- Error code `origin` (403).
- The invite text is `https://<address>/?code=&key=&id=`; `PairingInvite.parse`
  accepts it and the JSON.
- `core::remote::invite_url`, `remote::reissue`, `adapters::web::asset`.
- No `AppAction`, `Effect`, `model.rs` or port-trait change.
