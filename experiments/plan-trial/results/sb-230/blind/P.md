# Plan for issue #230: a web control surface on the remote port

One commit. The app runs at every step: the page is served beside the
API and nothing the native clients speak changes. Line numbers are this
worktree's (HEAD 380e4d4). "Facts" is `facts/1/repo/facts.md`; a line
cited as Settled is not re-derived here, and every decision that rests
on an Unresolved or For-the-owner line says so where it is made and
names the fallback.

## Where this departs from the issue

- **"The one-line excerpt the card shows" is not on the wire.** The Mac
  computes it in the UI (`agent_excerpt`, `src/ui/cards.rs:558`);
  `SessionView` (`control/src/lib.rs:637`) has no excerpt, and
  `docs/remote-client.md:207` describes a `last_message` field on the
  session's metadata that the code does not have (Facts, Settled:
  `waiting` and the listings carry no excerpt). The page shows what the
  view has: the card word, `waiting_reason`, kind, liveness, quiet time.
  For the *waiting* sessions only, it also reads `last-response` and
  shows its first non-empty line, for the first ten waiting sessions
through the page's request queue (decision 4), held in memory
  by (session, `last_stop_at_ms`). No new field on `SessionView`. This
  rests on For the owner "the card's excerpt"; if the owner wants the
  line on every card, the implementer builds the other branch: an
  additive, masked `excerpt: Option<String>` on `SessionView` (serde
  default, so older clients decode), computed on the frame from the same
  cache the card reads, which moves the fingerprints and needs the
  Swift fixtures rewritten (`SWITCHBOARD_WRITE_FIXTURES=1`).
- **"The device's scope in the header" has no endpoint.** Only the
  `/pair` reply carries `scope` (`src/app.rs:3660`–`3665`; Facts,
  Settled). A returning browser, and a browser whose scope the owner
  raised in the dialog, cannot learn it. Plan: one additive
  authenticated route, `GET /api/v1/me`.
- **"Revoked" and "never paired" are the same 401** (`authenticate`,
  `src/app.rs:3127`–`3156`). The plan adds no distinct code. The page
  keeps a non-secret marker `{server_id, spki_sha256, name}` after a
  pairing; a 401 with a marker whose key still equals `/info`'s is
  "revoked", a differing key or id is "identity changed, pair again"
  (a missing key remakes the identity and clears `devices.json`, so the
  key moves; `tests/remote.rs:1422`), and a 401 with no marker is "not
  paired".
- **"Compares the key in the URL with the one `/info` presents"** is a
  comparison against the host's own claim (`remote_info`,
  `src/app.rs:3568`); no browser API reads the peer certificate
  (Facts, Settled). It catches a wrong or stale host at the address; it
  does not stop an active relay that terminates TLS and forwards both
  `/info` and `/pair`. The dialog says so (UI, below).
- **"Any path outside `/api/v1/` is refused"** stops being true for
  three exact paths. `docs/remote-client.md:306` ("Require pairing
  before anything but `/info` and `/pair`") and the checkbox hover text
  (`src/ui/remote.rs:113`) gain "and the page".
- **"Revoke the phone and the page says so on its next request"** is
  met by the page's `/me` on load and by a socket close followed by a
  `/me`. A revoked page's open WebSocket is closed by the host
  (`RevokeDevice` drops the device's subscribers); the close carries no
  reason, so the page asks.

## The decisions the issue asks for

**1. Trust in the certificate: the click-through is the trust step.**
Why not the download: Apple's evaluation refuses today's certificate
even as the only trusted anchor, "certificate name does not match
input", because it has no subjectAltName (Facts, Settled, spike 23
`README.md:46`; `make_identity` builds `CertificateParams::new(vec![])`,
`src/adapters/remote.rs:102`). A trusted install therefore needs a
reissue with an IP SAN, bound to one address, so every DHCP change
means a new certificate and a new trust step on every phone (Settled,
`README.md:47`, `:50`). That is a larger feature than this page, and
the click-through needs none of it. What the click buys: the browser
accepts this certificate for this address. What it does not: it proves
nothing about who answered; on a hostile LAN an impostor can serve a
genuine-looking page and ask for the code. The code is single-use, the
window five minutes and three tries, and the owner compares the Key
line the page shows with the dialog's. The dialog says that in one
sentence. §5 is kept: before pairing the port serves `/info`, `/pair`
and the three page paths, nothing else.
Rests on two Unresolved lines: whether Safari offers a click-through for
a CN-only certificate on an IP, and whether the `wss://` socket then
connects (WebKit 41419 suggests it may not). The page is built for the
second regardless (polling fallback, UI). **Fallback if the owner's
device says there is no click-through:** `make_identity` takes the
advertised addresses and writes an IP SAN for each, the serverAuth EKU,
`not_before` kept at 1975 (the 825-day rule binds certificates issued
after 2019-07-01, Settled), the same key so the SPKI pin holds;
`identity.json` keeps the certificate, reissued when
`remote::local_address()` changes; a fourth page path `/switchboard.cer`
serves it; the dialog tells the owner to trust it under Certificate
Trust Settings (Settled, support.apple.com/en-us/102390).
Also Unresolved: whether the click-through exception survives a reload,
a host restart (same certificate), a reissued certificate and private
mode. The 400-day cookie only helps if the page can load again, so both
decisions rest on it. If the exception is lost between visits but holds
within one, the owner clicks through on every visit; that is accepted
and the dialog says "the browser may ask again on each visit". If it is
lost within a visit (a reload asks again), the page cannot work and the
same SAN-reissue fallback applies. It joins the not-yet-run list.

**2. The credential is a cookie, by default; the owner decides.** This
answers For the owner "Cookie or `Sec-WebSocket-Protocol` for the
token?". The trade, whole: a cookie keeps the token out of every script
on the origin (an `HttpOnly` cookie is unreadable to the page, which is
the XSS argument for a page that can type into an agent) and survives
Safari's seven-day wipe of script storage (Settled). Its cost is that
cookies are not isolated by port (Settled, RFC 6265 §8.5), so every
HTTPS server the browser reaches on that IP, on any port, receives
`__Host-sb-<id>=<token>` in its `Cookie` header: a second Switchboard
host, and any HTTPS dev server or proxy on the Mac that logs request
headers, would hold a paired device's credential. The bearer branch
below has neither problem (storage is per origin, port included) and
pays with a script-readable token and the seven-day wipe. The plan
builds the cookie unless the owner picks the bearer branch, which is
written out below in full so the choice can go either way. `/pair`, when the body carries
`"credential": "cookie"`, sets
`__Host-sb-<server_id>=<token>; Path=/; Secure; HttpOnly; SameSite=Strict; Max-Age=34560000`
(400 days, the cap, Settled) and leaves `token` out of the JSON.
Native clients do not send the field and get the token as before. The
name carries the server id because cookies are not isolated by port and
`__Host-` does not change that (Settled, RFC 6265 §8.5), so two hosts on
one IP do not overwrite each other. `SameSite=Strict` does not stop a
request from another port on the same IP (Settled, "site" ignores the
port), so the host checks `Origin` itself: a cookie-authenticated
request is refused 403 `origin` unless `Origin` equals
`https://<Host header>`, and a cookie-authenticated non-GET request or
upgrade without an `Origin` is refused too (the Fetch standard's "append
a request `Origin` header" always adds one when the mode is "websocket"
or the response tainting is "cors", and the default `fetch` mode is
"cors"; a plain GET may omit it and is side-effect free). The rule has
a condition on the page: it calls `fetch` with the default mode and
never sets `mode`, because the page sends `Referrer-Policy:
no-referrer` (decision 4) and under it a non-GET request that is not
"cors" carries `Origin: null`, which every send would then fail on. The
host does not accept `null`. A request
carrying a bearer header skips the check. The WebSocket handshake
carries the cookie and an `Origin` (Settled, WHATWG §2.2; RFC 6455
§10.2) and needs no subprotocol, so `respond_upgrade` is unchanged.
Rests on the Unresolved line "whether a `Secure` cookie on a
click-through origin is stored and sent". **Alternative (the bearer
branch), built if the owner picks it or the device shows the cookie is
not stored:** the `/pair` reply keeps `token` for the page,
the page sends `Authorization: Bearer` on `fetch` and offers
`["sb1", <token>]` to `new WebSocket`; `read_request` keeps
`sec-websocket-protocol`, and `respond_upgrade` echoes `sb1` (a base64url
token fits RFC 6455's token rule, Settled); the token lives in
`localStorage`, which Safari deletes after seven days without use
(Settled), so the marker then reads "not paired" and the owner pairs
again.

**3. One QR, two texts.** The QR becomes the URL
`https://<ip>:<port>/?code=<code>&key=<spki_sha256>&id=<server_id>` (a
smaller QR than the JSON: version 8 against 9, Settled, `README.md:61`).
"Copy invite" stays the JSON, so `switchboard-remote pair "$(pbpaste)"`,
the headset's paste field and the per-line scan
(`client/Sources/switchboard-remote/main.swift:111`,
`client/Sources/SwitchboardUI/BrowserView.swift:106`, `:883`) and the
assertion at `tests/ui.rs:7474`–`7492` keep working, and the Swift tests
(not in CI) are not touched. A "Copy link" button copies the URL. This
is the "Keep JSON" branch of For the owner; if the owner wants URL only,
the implementer teaches `PairingInvite.parse`
(`client/Sources/SwitchboardProtocol/Info.swift`, `parse`) the URL with
`URLComponents` (an IPv6 literal arrives in brackets and is an
Unresolved parsing question), changes the assertion at
`tests/ui.rs:7486`, and adds a Swift fixture test.

**4. The static paths.** Exactly `GET /` (`text/html; charset=utf-8`),
`GET /app.css` (`text/css; charset=utf-8`), `GET /app.js`
(`text/javascript; charset=utf-8`); the query is ignored, so
`/?code=…` is `/`. Any other method or path falls through to the frame
and is refused 404 as today (`/favicon.ico`, `/index.html`, `HEAD /`).
The page's icon is `<link rel="icon" href="data:,">` so no browser asks
for `/favicon.ico` unprompted. Headers on the three:
`Content-Security-Policy: default-src 'none'; script-src 'self'; style-src 'self'; connect-src 'self'; img-src data:; base-uri 'none'; form-action 'none'; frame-ancestors 'none'`
(`'self'` covers `wss://` on the same host and port, Settled; the policy
is a header because `frame-ancestors` is ignored in a `<meta>`, Settled),
`Cache-Control: no-store` (the files change with the binary and are
small), `X-Content-Type-Options: nosniff`, `Referrer-Policy: no-referrer`,
`Connection: close`. These are answered **on the connection thread**,
before `RemoteIncoming` is built, so a page load costs no frame pass, no
inbox slot (`FRAMES`/64) and none of the eight drains per pass
(`DRAIN_PER_PASS`, `src/adapters/remote.rs:580`), and it works with the
app hidden. `Connection: close` ends the thread and so frees the
connection's slot in the four-connection unauthenticated pool at once.
Their own rate window, `PAGE_PER_MINUTE = 120`, beside `PAIR_PER_MINUTE`
and `INFO_PER_MINUTE` (`:583`–`:584`), through `Limits::limited`
(`:631`), 429 as the others. A page opened with no code and no cookie
shows "This browser is not paired" and how to pair (Remote devices…,
Allow pairing, scan the QR); a code whose window has closed shows the
host's `pairing_closed` text.
Rests on the Unresolved line about Safari's parallel connections against
`MAX_UNAUTHENTICATED = 4`. This binds after pairing too: the accept loop
counts every new connection as a stranger until the frame answers one of
its requests as authenticated (`src/adapters/remote.rs:702`–`711`,
`:840`–`:855`), so the page limits itself everywhere, not only at load.
The HTML, CSS and JS are `Connection: close`. After that the page sends
**every** API request through one queue, `MAX_IN_FLIGHT = 2`, and the
first request through it, `/info` or `/me`, goes alone; the events socket
is opened only after the first authenticated reply, so at most two
connections are unauthenticated at any moment and the socket is the
third, already paired. Every API call, not only `/info` and `/pair`,
retries a network error three times 300 ms apart. The excerpt reads of
departure 1 go through the same queue, the first ten waiting sessions in
order, and fill in as the queue allows. **Fallback if the owner's device
still sees a refused connection:** raise `MAX_UNAUTHENTICATED` to 6 and
amend §4.1's first bound; the pools stay separate.

**5. Where the page lives and how it is tested.** `web/index.html`,
`web/app.css`, `web/app.js` at the repo root, embedded by
`src/adapters/web.rs` with `include_str!`. No build step, no
dependency, no `build.rs`. Tests below.

## The record

Nothing changes on disk. A browser is a `PairedDevice` (`src/ports/remote.rs:96`)
like any other: `name` is the `device_name` `/pair` already takes
(`src/app.rs:3650`, cut to 64 characters by `new_device`), `scope` comes
from the pairing window, it is listed and revoked in the dialog as today.
This answers For the owner "the name for a browser device": the page
offers a name field, defaulted from the user agent ("Safari on iPhone",
"Chrome on Android"; an iPad reads as "Safari on Mac", Settled), which
the person edits before pairing, so the host parses no user agent. The
alternative is the host-side UA table with no field; the implementer
builds it only if the owner asks. Nothing marks a device as a browser.
Scope answers For the owner "Should a browser get control scope": yes,
the scope picker as today (`Read` by default). The other branch, if the
owner wants it: `/pair` with `credential: "cookie"` grants `read`
whatever the window's scope, and the owner raises it in the dialog
(`SetDeviceScope`); one line in `remote_pair`, and test 3 asserts it. Rehydration is today's: the
hash of the cookie's token goes through `device_by_token`, so a browser
paired yesterday is authenticated today.
Schema versions today: `SCHEMA_VERSION` 16 (`src/core/model.rs:12`),
`VIEWS_SCHEMA_VERSION` 7 (`:612`), `IDENTITY_SCHEMA_VERSION` 1
(`src/ports/remote.rs:17`). No serialized type changes, so none bumps.

## The signals

- `RemoteRequest` (`src/adapters/remote.rs:360`) gains `cookie`,
  `origin` and `host`, taken in `read_request` (`:444`) beside
  `authorization`, which today is the only credential kept. Like the
  bearer they are never logged; `refuse` (`src/app.rs:3754`) logs method
  and path only and no log line prints a query or token (Facts, Settled).
  `RemoteRequest::cookie_origin_ok()` is the Origin rule of decision 2.
- `RemoteReply` (`:389`) gains `set_cookie: Option<String>`.
- `respond` (`:530`), which hard-codes `Content-Type: application/json`,
  becomes a wrapper over `respond_with(w, status, content_type, extra
  headers, body)`; the page and the `Set-Cookie` use the second.
- `authenticate` (`src/app.rs:3127`): bearer first; else the cookie
  named for this host's server id, then the Origin rule. A refusal is
  403 `origin` ("this request did not come from the page the host
  served") or the same 401s as today.
- The events socket: nothing new. `changed` with `session`, `sessions`,
  `projects`, `waiting`, `conversation` and `screen` kinds (the
  spec's §4.4) is all the page reads; it sends
  `{"type":"watch","resource":{"kind":"screen"|"conversation","id":…}}`
  for the open session (`src/app.rs:1720`).

## The actions and effects

None. No `AppAction`, no `Effect`, no port method: everything is
adapter, `serve_remote` and the page. In `serve_remote` (`:3017`) `/pair`
reads the `credential` field and builds the `Set-Cookie`; a new
authenticated arm `("GET", ["me"])` answers `{device_id, name, scope}`
in the usual envelope. `core/remote.rs` gains one pure function,
`pairing_url(addr: Option<SocketAddr>, server_id, spki_sha256, code) -> Option<String>`
(`SocketAddr`'s display brackets an IPv6 literal; the code's alphabet
and hex need no percent-encoding). `None` in, `None` out: `remote::local_address()`
is `None` with no route and drops loopback (`src/adapters/remote.rs:962`–`970`).

## The page

`app.js`, plain ES2020, no `eval`, `innerHTML` or `document.write`
(text through `textContent`, which is the whole XSS argument for a page
that can type into an agent). In order:

1. First statement: read `location.search`, then
   `history.replaceState(null, "", location.pathname)`; the code lives
   in a variable only, never in storage. (Whether the browser's global
   history or iCloud tab sync kept the first URL is Unresolved; the
   dialog's text does not promise it did not.)
2. `GET /api/v1/info`. With an invite: if `spki_sha256` or `server_id`
   differs from `key`/`id`, say "this is not the host the code was made
   for" and send nothing. Else show the host name, the short key (four
   groups of four, as the dialog does), the name field and a Pair
   button; the tap posts `/pair {spki_sha256: key, code, device_name,
   credential: "cookie"}`. `wrong_code`, `wrong_key`, `pairing_closed`
   and `rate_limited` are worded plainly.
3. Without an invite: `GET /me`; 200 shows the list, 401 per the
   marker (departures, above).
4. The list follows §4.4: open the socket, wait for `hello`, fetch
   `projects`, `waiting` and each project's `sessions` (the listing
   has no global form), apply buffered events by revision (an event
   past a snapshot's revision refetches that resource). A "Waiting on
   you" band comes first, then projects, waiting sessions first in each
   and marked; each row shows kind (`claude`, `codex`, `shell`,
   `command`, `service`), the card word, `waiting_reason` and the
   excerpt of departure 1. The header shows the host name from `/info`,
   and this device's name and scope from `/me`.
5. A session view: `claude`/`codex` read `last-response`, falling back
   to `screen` when it is empty; the rest read `screen`; a session not
   running shows its liveness and exit code (run output is a later
   issue, per "Not in this cut"). It watches the key and refetches on
   `changed`, and on its own every thirty seconds (§4.4).
6. The reply box exists only when `/me` says `control`. A send posts
   `{op: <uuid>, text}` to `/sessions/{id}/send`; the op is kept in
   `sessionStorage` until the reply is read, a timeout is followed by
   `GET /ops/{op}`, never a resend (§4.5), and a reload with a pending
   op asks `/ops/{op}` first. A 403 `scope` hides the box again. The
   op id is 16 bytes from `crypto.getRandomValues`, hex (the Unresolved
   line on `crypto.randomUUID()` on certificate-error pages is thereby
   not depended on).
7. Live updates: if `hello` does not arrive within five seconds, or the
   socket closes before `hello` twice running, the page says "live
   updates unavailable, refreshing every 5 s" and polls `waiting` and the
   open session every five seconds, projects and sessions every
   fifteen. This is the answer to the `wss://` Unresolved line and
   stays in whichever way the owner's run goes.

## The UI

`src/ui/remote.rs`: the QR (`qr_texture`, `:280`) encodes
`pairing_url(...)`; `payload` stays the JSON for "Copy invite"; a "Copy
link" button beside it. With no network address (`pairing_url` is
`None`; the dialog already sends an empty `addresses`, `:156`–`:163`)
the QR is the JSON as today, "Copy link" is not drawn, and the scan line
is the old one. The URL-versus-JSON choice is a pure function of the
address, tested in `src/core/tests.rs`; `tests/ui.rs` cannot seed it,
because the address comes from the machine's real route. Address, port and the cache key (`draft.qr`
keyed by code, `:168`–`:172`) as today. The scan line becomes: "Scan with a
phone camera to open the page, or paste the invite into a client.
The browser will warn about the certificate; accepting it is the trust
step, so check that the page shows this Key. Grants {scope} scope. Open
for five minutes or three wrong codes." The checkbox hover (`:113`)
ends "only paired devices get past /info and the page". With the port on
loopback the address in the URL is still the LAN address (existing
behaviour, now inherited); listed as a known gap, not fixed here.

## The tests

- `src/core/tests.rs`: `pairing_url` for an IPv4 and an IPv6 address,
  parameter order, no encoding needed.
- `src/adapters/remote.rs` unit: `read_request` keeps `cookie`,
  `origin` and `host`; `respond_with` writes the headers and
  `Content-Length`; `cookie_origin_ok` over GET/POST/upgrade with
  and without `Origin`.
- `tests/remote.rs`, with a new `http_full` beside `http`
  (`:245`, which keeps only status and body and matches
  `Content-Length:` by exact case) returning headers, a `Cookie` and
  `Origin` option:
  1. The three paths answer unpaired, **with the test never driving
     `serve_pending`** (proving the connection thread served them), with
     the exact headers above; `HEAD /`, `POST /`, `/index.html`,
     `/favicon.ico` stay 404 JSON.
  2. The embedded page references only paths the host serves: every
     `src`/`href` in `index.html` is one of the three paths or `data:`;
     every `/api/v1/…` literal in `app.js`, with ids filled in from the
     seeded records, answers something other than `not available on this
     port` for a paired device. `index.html` has no inline `<script>`,
     `<style>` or `style=`; no file contains `eval(`, `innerHTML` or
     `document.write`; `app.js` contains no `mode:` (decision 2's
     Origin condition); `history.replaceState` precedes the first
     `fetch(` in `app.js`.
  3. Pairing end to end as the CLI does, with the host's own client:
     `/pair` with `credential: "cookie"` returns no `token`, and
     `Set-Cookie` equals the exact string of decision 2; `GET /me` with
     only that cookie answers the name and scope; a raised scope shows
     on the next `/me`; a wrong cookie 401; a bearer-only pair is
     unchanged (the existing pairing tests stay as they are).
  4. The Origin rule: cookie plus a foreign `Origin` 403 `origin`; a
     cookie `POST …/send` with no `Origin` 403, with the right one runs
     once per op and a repeat is answered from the log; a cookie `GET`
     with no `Origin` 200; a bearer request ignores `Origin`.
  5. The socket: `subscribe_at` gains a cookie-and-origin variant;
     `hello` arrives with the right `Origin`, the handshake is refused
     without one or with another port's; a revoke closes it and the
     next `/me` is 401 (`RevokeDevice`).
  6. `PAGE_PER_MINUTE`: the 121st page request in a minute is 429,
     while `/info` still answers. Three page loads in sequence leave
     the unauthenticated pool empty.
  7. The pool bound the page's queue is built around: five cookie
     connections opened at once, each sending a request, with no
     `serve_pending` yet, so all are strangers: the fifth is closed
     before its request is read and four are served; once the four
     replies are authenticated they are in the paired pool, and a
     sixth connection is then served.
- `tests/ui.rs:7424`: "Copy link" puts an `https://` URL with `code`,
  `key` and `id` on the clipboard (the code equal to the window's, the
  key to the identity's); "Copy invite" still copies the JSON
  (`:7474`–`7492` unchanged).
- No browser is driven (nothing in the tree, no keystroke injection).
  What needs a device is listed in the status section as not yet run:
  the acceptance sequence (scan with a phone camera, accept the
  warning, see the sessions, a session that starts waiting moves to the
  top, a reply arrives, revoke and the next request says so) on a
  default set, for the owner to confirm or replace (For the owner,
  "which devices"): an iPhone on current iOS with Safari, and a Mac
  with Safari. Chrome or Firefox on iOS are apps and raise their own
  Local Network prompt (Settled), so they are not in the default set.
  Plus the Unresolved lines: click-through for the CN-only
  certificate, whether that exception survives a reload, a host
  restart, a reissued certificate and private mode, the `wss://`
  socket, the cookie's storage, the connection fan-out, the camera
  banner. No new spike directory:
  nothing here is runnable without a device, and the certificate
  question has spike 23; the owner's results go into spike 23's
  `README.md`.

## Documents

- `docs/remote-client.md`: §4.1 (page paths answered on the connection
  thread, `PAGE_PER_MINUTE`, the page's two-request queue), §4.3 (rows for `/`, `/app.css`, `/app.js`
  and `/me`), §5 (the pre-pairing bullet at `:306`, the token bullet "sent
  only as a bearer header" to say a browser's is a cookie, the pairing
  bullet to say the QR is a URL, the Origin rule, what the key
  comparison does and does not prove, and one sentence that a cookie
  goes to every HTTPS server on the host's IP whatever its port, so a
  paired browser's credential can reach a co-hosted server's logs).
- `docs/design.md`: the "Remote client" status section (ends before
  `:3585`): what is built, and the not-yet-run list above, and the
  known gap about the loopback address.
- `README.md`: Layout rows for `web/` and `src/adapters/web.rs`, the
  edited `adapters/remote.rs` row (page paths, the page rate window),
  and a Development line: with `SWITCHBOARD_REMOTE_PORT` set and
  `pairing <code>` / `remote-listen` script lines, the page is at
  `https://127.0.0.1:<port>/`. No new `SWITCHBOARD_SCRIPT` line: the
  state is reachable with the existing ones.
- `CHANGELOG.md` under Unreleased: a browser page on the remote port,
  paired by scanning the dialog's QR.

## Commit message

Serve a browser page on the remote port, paired from a URL that carries the code

## Contract additions

- Routes: `GET /` (`text/html`), `GET /app.css`, `GET /app.js`, answered
  on the connection thread before pairing; `GET /api/v1/me`
  (`{device_id, name, scope}`, any paired device).
- `/pair` request field `credential: "cookie"`; with it the reply
  carries `Set-Cookie: __Host-sb-<server_id>=…` and no `token`. The
  host accepts that cookie as a credential, with the Origin rule,
  beside the bearer header.
- Error code `origin` (403).
- `RemoteRequest` fields `cookie`, `origin`, `host`; method
  `cookie_origin_ok`; `RemoteReply` field `set_cookie`.
- `src/adapters/web.rs` (new): the embedded assets and their headers;
  `web/` (new): `index.html`, `app.css`, `app.js`.
- `core::remote::pairing_url`; constant `PAGE_PER_MINUTE = 120`.
- The pairing dialog's QR is the URL form; "Copy invite" is the JSON as
  before; "Copy link" is new.
- No change to `model.rs`, `AppAction`, `Effect`, any port trait or any
  serialized type; no schema bump.
