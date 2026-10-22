# Plan for #230: a web control surface on the remote port

One commit. The app runs at every step of the order at the end. "Facts"
below is `facts.md` for this ticket, cited as `facts:<line>`. Tree
references were checked in the worktree at "Release 0.4.1".

## Where this departs from the issue

1. **The excerpt is not on the wire.** The issue gives each session "the
   one-line excerpt the card shows". `SessionView`
   (`control/src/lib.rs:637`) has no excerpt field and `last_message`,
   which `docs/remote-client.md` §4.3 (`session.last_response` bullet)
   calls "a field of the session's metadata", exists nowhere in the
   wire crate (facts:8). The card's line is drawn on the Mac by
   `caption_of` (`src/app.rs:4105`). Owner question 6 (facts:91) is
   unanswered, so the plan builds its "no" branch: a row shows the
   session's name, kind, card word and waiting reason. The branch
   the other way is an additive `excerpt: Option<String>` on
   `SessionView` with `#[serde(default)]`, a `changed {session}` that
   already follows the fingerprint, and a regenerated
   `client/Fixtures/session*.json`; it is not built here.
2. **The page cannot learn its scope from anything.** The issue puts
   "the device's scope" in the header and hides the reply box without
   `control`. No endpoint says it (facts:9), though `docs/remote-client.md`
   §5 ("the host answers with the device's scope") says every reply
   does; the envelope carries only `epoch`, `revision` and `body`
   (`src/app.rs:3730`). Owner question 7 (facts:94) is unanswered; the
   plan builds its "yes": `GET /api/v1/device`. The "no" branch (the
   page stores the scope from the `/pair` reply and learns of a change
   from a 403) is worse for a page, whose cookie it cannot read, so
   it is not built.
3. **"Revoked" cannot come from the host.** A revoked token and an
   unknown one get the same 401 and `devices.json` keeps no tombstone
   (facts:10; `authenticate`, `src/app.rs:3127`; `RevokeDevice`,
   `src/core/remote.rs:86`). Owner question 5 (facts:88) is
   unanswered; the plan builds the "no" branch with a twist that
   needs no record: the page remembers, in `localStorage`, that it
   paired with this `server_id` and key. A 401 on a browser that
   remembers pairing says "this device was revoked on the Mac, or the
   Mac forgot every pairing; pair again", and a 401 on one that does
   not says "not paired". The tombstone branch (a durable
   `revoked.json` of token hashes, beside `devices.json`, written
   through `RemoteFiles::save`) is not built.
4. **The `key` check is against a field the answering server writes.**
   The issue has the page "compare the key in the URL with the one
   `/info` presents". A page cannot read the TLS certificate or its
   SPKI (facts:67, unresolved; the plan assumes it cannot), so the
   comparison is with the `spki_sha256` field of `/info`'s JSON. It
   stops a page reaching the wrong genuine Switchboard by address; it
   stops no impostor. See decision 1.
5. **There is no all-sessions read.** "Sessions grouped by project"
   costs `GET /projects` and one `GET /projects/{id}/sessions` per
   project (facts:9); the page does them one at a time (see the
   signals). No new read model.
6. **"The host's own HTTP client as the CLI does".** The CLI
   (`client/Sources/switchboard-remote/main.swift:106`) is Swift. The
   Rust test side is the hand-rolled `http` in `tests/remote.rs:245`,
   which reads only the status and `Content-Length` (facts:19). The
   plan extends that helper; it adds no client to `src/`.
7. **Device names.** The issue wants "Safari on iPhone" from the user
   agent. The host reads no `User-Agent` and does not need to: `/pair`
   already takes `device_name` from the body (`src/app.rs:3650`,
   facts:14), so the page builds the name. An iPad's Safari sends a
   Mac's user agent (facts:59), so the page tells it apart with
   `navigator.maxTouchPoints > 1`, a heuristic the plan states in the
   code's comment; a wrong guess only mislabels a row the owner can
   see. Owner question 10 (facts:101) is open; its other branch (the
   page asks for a typed name) is a field on the pairing view and is
   not built.
8. **A correction to the investigation notes, not to the issue.** The
   notes count about 28 `RemoteReply { .. }` literals; there are ten
   (`src/app.rs:1664, 1679, 2365, 2371, 2376, 2754, 3570, 3723, 3736,
   3767`), the rest of the matches are signatures.

## The record

Nothing on disk changes. A browser is a `PairedDevice`
(`src/ports/remote.rs:96`) like any other: name from the page, scope
from the pairing window, listed and revoked in the existing dialog. No
field is added to `PairedDevice` or `Identity`, so no
`IDENTITY_SCHEMA_VERSION` change (1 today, `src/ports/remote.rs:17`)
and none to `SCHEMA_VERSION` (16, `src/core/model.rs:12`) or
`VIEWS_SCHEMA_VERSION` (7, `:612`); `model.rs` is not touched. If an
owner answer flips a branch above and a field is needed after all
(owner question 9's separate web switch would put `web: bool` with
`#[serde(default)]` on `Listen`), the bump is the value in
`src/ports/remote.rs` at implementation time, plus one, because a
ticket landing in parallel may take the next number first; there is no
migrate step for either file (`devices.json` and `identity.json` load
with plain serde, facts:15).

What the page keeps, per device, none of it from a host's data:
the cookie (set by the host, `HttpOnly`, so script cannot read it) and
one `localStorage` entry `sb.host = {server_id, spki_sha256}` written
after a successful pairing, which is how decision 1's check and
revoked-versus-never-paired are said. `localStorage` is wrapped in
`try`/`catch`; without it the page works and says "not paired" instead
of "revoked". Everything else is refetched on every load, as the Swift
client does (`docs/remote-client.md` §7, "What the client keeps").

## The signals

The wire gains three static paths, one route, one request field, and
one authentication source. `REMOTE_PROTOCOL` stays 1: every addition is
additive.

**Decision 1: browser trust in the self-signed certificate. Option A: the
click-through on the local network is the trust step.**
This rests on two lines. The certificate has no SAN, no `serverAuth`
EKU and a year-4096 expiry (facts:16), and iOS trusts only a certificate
that names the server in the SAN, carries the EKU and lives 825 days or
less (facts:55), so option B (the host offers its certificate) is a new
certificate per LAN address, a profile install and Certificate Trust
Settings on each device (facts:54) for a thing the owner does once per
phone anyway. And whether Safari lets a WebSocket through after the
page's click-through is unresolved (facts:64, bug 145819); only a device
settles it. That is owner question 1 (facts:76), and it decides
nothing about the build: **the page falls back to polling if the
socket fails to open** (below), which is the fallback facts:78 names
for Safari. Option B stays unbuilt; if the owner's device run wants it,
the work is `make_identity` (`src/adapters/remote.rs:95`) issuing the
certificate with the address in its SAN and the EKU, from the same key
so every native pin holds (facts:55, the pin hashes the key), and a
download path.
What option A buys: the connection is encrypted against anyone
watching the LAN, and the `key` in the URL stops a page that reached
the wrong genuine Switchboard (a reused DHCP address, two Macs) from
sending the code. What it does not buy: a page served by an active
impostor is not a genuine page, and an impostor can relay `/pair` to
the real host inside the window and keep the cookie the real host sets.
Doing that needs a man in the middle of the Mac's address and the owner
clicking through the warning for it within five minutes. The dialog
says so (see the UI). `docs/remote-client.md` §5's "the client verifies
the pin on every connection" gains "a browser cannot; see §5.1".

**Decision 2: the token reaches the socket in a cookie. Cookie, with the
Origin rule below; the bearer header stays for the native clients.**
The owner's question 2 (facts:79) is open; the plan takes the cookie
branch because it keeps the token out of script and out of the storage
Safari prunes after seven days (facts:58), and the browser sends it on
the WebSocket handshake (facts:30), which a subprotocol would need
`respond_upgrade` (`src/adapters/remote.rs:519`) to echo (facts:34).
- `POST /api/v1/pair` takes a new body field `web: true`. The
  reply for a web pairing carries no `token` in the body (script never
  sees it) and sets
  `sb-<server_id>=<token>; Path=/api/v1; Max-Age=34560000; HttpOnly;
  Secure; SameSite=Strict`. The name carries the `server_id` because
  cookies are not isolated by port (facts:24), so two instances on one
  address (the dev aid `SWITCHBOARD_REMOTE_PORT`, the tests) must not
  share one. `Path=/api/v1` keeps the cookie off the static requests.
  Without `web` the reply is today's, byte for byte, and no
  `Set-Cookie` is sent.
- `authenticate` (`src/app.rs:3127`) reads the bearer if present and
  ignores any cookie then; with no bearer it reads the cookie named
  for this host's `server_id`. A request whose credential came from the
  cookie must pass the **Origin rule**: for a non-`GET` request or an
  upgrade, `Origin` must equal `https://` plus the request's `Host`
  header, else 403 `origin`. The rule is needed because `SameSite`
  ignores the port, so a page on another port of this IP is same-site
  (facts:25), and the browser sends `Origin` on every POST and every
  socket handshake (facts:31, 32). A `GET` carries none (facts:32),
  and the rule does not cover it: a cross-origin page cannot read a
  `GET`'s reply (the port never sends a CORS header, and answers no
  `OPTIONS`), and no `GET` route changes a record (the route table,
  `src/app.rs:3027-3110`). The bearer path needs no rule: a
  cross-origin page cannot attach the header.
- A 401 for a cookie credential the host does not know also sends
  `sb-<server_id>=; Path=/api/v1; Max-Age=0`, so a revoked device's
  browser stops sending a dead cookie on every poll.
- The cookie's `Secure` and `HttpOnly` on a click-through origin are
  stored and sent per the standard (facts:27) but no vendor says
  Safari or Chrome do (facts:65). If a device does not keep it, the
  page's first call after pairing (`GET /device`) is a 401 and the page
  says "this browser would not keep the pairing; open the pairing window
  and try again", having spent the code. The fallback the implementer
  builds if the owner's device run shows this is a follow-up, not this
  commit: the `web` field takes `"token"`, the reply carries the token,
  the page holds it in `localStorage` (and so loses it after seven
  days idle, facts:58), sends `Authorization: Bearer` on fetch, and
  polls because the socket cannot carry it. No host change beyond the
  field's third value. This is stated so the owner can pick it with
  the device result in hand.

**New route** `GET /api/v1/device`, authenticated, any scope: body
`{ "device_id", "name", "scope" }` of the caller, in the usual envelope
(`authenticated: true`). It is added to the route table in
`serve_remote` after `authenticate`, answered by `remote_device(&self,
&PairedDevice)`. The scope is read from the core on every call, so a
change in the dialog shows on the page's next request, as §5 says.
Not added to `info.capabilities`: that list is in `client/Fixtures/info.json`
and the Swift tests decode it.

**Decision 4: static paths** — `GET /`, `GET /app.js`, `GET /app.css`
and nothing else. `/index.html`, `/favicon.ico` and every other path
outside `/api/v1/` stay `404 not_found` from `serve_remote`
(`src/app.rs:3023`), so `tests/remote.rs:464` holds. A query string on
the three is ignored (the pairing link is `/?code=...`). A method other
than `GET` is 405 with `Allow: GET`. Each answer is `200` with
- `Content-Type`: `text/html; charset=utf-8`,
  `text/javascript; charset=utf-8`, `text/css; charset=utf-8`;
- `Content-Security-Policy: default-src 'none'; script-src 'self';
  style-src 'self'; connect-src 'self'; img-src data:; base-uri
  'none'; form-action 'none'; frame-ancestors 'none'`. No inline
  script, no inline style, no external resource. `default-src` does not
  cover the last three (facts:41) and `frame-ancestors` works only as
  a header (facts:42), so they are listed. `img-src data:` is for the
  page's `<link rel="icon" href="data:,">`, which saves a
  `/favicon.ico` request. `connect-src 'self'` covers `wss:` to the
  page's own host and port in the spec (facts:39); older Safari builds
  are unresolved (facts:68). If one blocks it the socket fails and the
  poll fallback runs, the same path as a Safari certificate failure;
- `Referrer-Policy: no-referrer`. The page's own subresource
  requests are made while the URL still carries the code, and a same-origin
  `Referer` is the full URL by default (facts:48); this header on the
  HTML is what keeps `/app.js` and `/app.css` from sending it;
- `Cache-Control: no-store`, so a page never outlives its version, and
  `X-Content-Type-Options: nosniff`;
- `Connection: close`.

*Where they are answered.* On the connection thread, before the frame
hand-off in `connection` (`src/adapters/remote.rs:770`), as the 429 and
503 already are. The files are compile-time bytes, so no frame state is
read, the frame's 64-deep inbox and 8-per-pass drain are not spent on
them (`docs/remote-client.md` §4.1: "the frame does core state only"),
and the code in a `/?code=` request line never reaches the frame's
memory or a `log::` call. The adapter's only `log::` is the
connection-error line (`src/adapters/remote.rs:714`, which prints the
`io::Error`, not a request), and `refuse` logs method, path and code
only (`src/app.rs:3754`); the page paths reach neither with a query.
*Pool and rate.* A page load is an unauthenticated connection like
`/info`'s, so it sits in the 4-slot pool (`MAX_UNAUTHENTICATED`,
`src/adapters/remote.rs:567`); `Connection: close` returns the slot
as soon as the file is written, so a load holds a slot for a few
milliseconds rather than the 30-second idle timeout, and the
HTTP/1.1-without-ALPN fan-out (facts:17, 71) cannot wedge the pool.
How many connections a given browser opens is unmeasured (facts:71);
the spike records it. A new `Limits.page` window, 60 a minute per
listener like `/info` (`src/adapters/remote.rs:623`), counts the
three paths together and answers 429 beyond it. The paired pool is 12
shared with the headset and the CLI, and a tab holds one slot for its
socket; the page therefore fetches **one request at a time** (a small
queue in `app.js`), so a tab holds at most a socket and one fetch
connection.
*With no code and no token* the page shows a static "Pair from the Mac"
panel and no host name; the same bytes go to everyone and the page
makes no request to find out who answers. This is the "a page" branch
of owner question 8 (facts:97). The "nothing" branch is `asset("/")`
returning `None`, one line. *Both binds serve it.* The files carry no
host data, so there is nothing to withhold on a loopback bind, and the
LAN rule of §5 becomes: before pairing a non-loopback bind serves
`/info`, `/pair` and these three files, which hold no host data. Owner
question 9 (facts:100) is open; the branch of a separate web switch is
a `web: bool` on `Listen` (see the record) and an arm in `connection`.

**The page's protocol** is the one the headset speaks, `docs/remote-client.md`
§4.4, re-implemented in `web/app.js`:
1. Open with `?code&key&id`: before any `await`, read the three,
   then `history.replaceState(null, "", location.pathname)`
   (allowed for the path and query of an https URL and it replaces the
   entry rather than adding one, facts:45, 46). The browser's global
   history, a camera app's prefetch and a link scanner may see the URL
   first and are unresolved (facts:69, 70); the code is single-use
   (facts:11), so what they keep is dead once the device pairs. The
   query form is the issue's; owner question 4 (facts:85) is open, and
   the fragment branch is `location.hash` in the page and a `#` for
   `?` in the one function that builds the link: the plan keeps the
   query and adds `Referrer-Policy: no-referrer` as facts:87 asks.
2. `GET /api/v1/info`; refuse and send nothing if `spki_sha256` or
   `server_id` differs from the link's ("this is not the Mac whose code
   was shown"). Then `POST /api/v1/pair` with `{code, spki_sha256,
   device_name, web: true}`. `403 pairing_closed` is "the pairing window
   on the Mac is closed; open it again". Write `sb.host`.
3. Opened without a link: read `sb.host`; `GET /info`; if its
   `server_id` or key differs from `sb.host`, say "identity changed,
   pair again" and stop (the spec's wording, §5) before sending
   anything else.
4. `GET /api/v1/device`: 401 is "revoked" if `sb.host` is set, else
   "not paired"; 503 `full` is "the Mac has too many connections open".
5. Open `wss` at `/api/v1/events` by a relative URL (resolved against
   the page, facts:36). Wait for `hello`, then snapshot (`projects`, then
   each `projects/{id}/sessions`), then apply the buffered events, per
   "Subscribe first, then snapshot". One held revision per key
   (`projects`, `sessions:<project>`, `session:<id>`, `waiting`,
   `conversation:<id>`, `screen:<id>`); a reply is applied only at or
   past the held revision; a new `epoch` drops everything;
   `changed` past the held revision refetches that key; `removed` drops
   it; `resync_required` or a close reconnects with 1 s doubling to 30 s,
   and a close followed by a 401 is the revoked state, which is how a
   revocation in the dialog (it drops the subscriber's queue,
   `src/app.rs:486`) reaches the page at once.
6. The open session sends `{"type":"watch","resource":{"kind":
   "conversation"|"screen","id":<record id>}}` (the frame format at
   `src/app.rs:1704-1735`), fetches after `watching`, and sends
   `unwatch` when closed. A visible content view also refetches every
   thirty seconds, as §4.4 says.
7. **Poll fallback** (the cost of owner question 1's other answer): if
   the socket gives no `hello` within five seconds on two attempts in a
   row, or `connect-src` reports a violation, the page says "Live
   updates are off; refreshing every 3 seconds" and refetches `waiting`,
   the project lists and the open view every three seconds, applying by
   revision as before. It retries the socket on a reload only.
8. Send: `POST /api/v1/sessions/{id}/send` with `{op, text}`, `op` a
   fresh UUID per attempt held until the reply; after a timeout the page
   reads `GET /api/v1/ops/{op}` and never resends blind (§4.5). Shown
   only when `device.scope` is `control`; a 403 `scope` hides it and
   re-reads `/device`. The 16 KiB cap is `info.limits.send_bytes`.

## Actions and effects

No `AppAction`, no `Effect` and no port trait changes. The core is not
involved except for one pure function: pairing is an adapter and frame
matter, and `AppAction::Paired` and `RevokeDevice` already do the whole
record. Everything below is adapter and frame.

- `src/adapters/remote.rs`
  - `RemoteRequest` gains `cookie`, `origin`, `host`: `Option<String>`,
    read in `read_request` (`:444`) from the lowercased headers beside
    `authorization`. `read_request` is the only place that builds one.
  - `RemoteReply` gains `set_cookie: Option<String>`; the ten literals
    in `src/app.rs` and the one in the adapter (`:829`) get
    `set_cookie: None`.
  - `respond` (`:530`) becomes a call to a new
    `respond_with(w, status, content_type, headers, body)`; the 429,
    503, 504 callers are unchanged. The reason table gains 405's
    `Allow` through `headers`.
  - `cookie_name(server_id: Uuid) -> String`,
    `cookie_value<'a>(header: &'a str, name: &str) -> Option<&'a str>`,
    `same_origin(origin: Option<&str>, host: Option<&str>) -> bool`:
    pure, with unit tests.
  - `Limits` gains `page`; `limited` (`:631`) maps the three paths onto it.
  - In `connection` (`:770`), after the rate check and before the frame
    hand-off: if `web::asset(&request.path)` is `Some`, answer 200 for
    `GET` or 405 otherwise through `respond_with` with `web::HEADERS`
    and `Connection: close`, and `return Ok(())`.
- `src/adapters/web.rs` (new): `include_str!("../../web/index.html")`,
  `app.js`, `app.css`; `pub struct Asset { content_type, body }`,
  `pub fn asset(path: &str) -> Option<Asset>`, `pub const HEADERS`.
  `web/` at the repo root reaches `include_str!` in every build
  without a `Cargo.toml` change (facts:18).
- `src/app.rs`
  - `authenticate` (`:3127`): the credential source and the Origin
    rule, with the clearing cookie on an unknown cookie.
  - `remote_pair` (`:3606`): `web` field; token out of the body and
    `set_cookie` on the reply for a web pairing.
  - `serve_remote`: `("GET", ["device"])` after `authenticate`;
    `remote_device`.
- `src/core/remote.rs`: `AppCore::pairing_url(&self, address: &str) ->
  Option<String>`: `https://<address>/?code=<code>&key=<spki_sha256>&id=<server_id>`
  from the identity and the pairing window as last set, `None` when
  either is missing or the address is empty. All four parts are in
  the 32-letter alphabet, hex and a UUID, so nothing is percent-encoded.
  This is formatting and belongs in the core, as the JSON payload
  should have; the JSON stays where it is in the UI (leaving it alone
  keeps the commit small).
- `web/index.html`, `web/app.js`, `web/app.css` (new): plain files, no
  build step, no dependency. `app.js` is split into a few small
  functions (`pair`, `connect`, `reconcile`, `render`) so its length
  stays readable; no framework.
- `client/Sources/SwitchboardProtocol/Info.swift:113`: `PairingInvite.parse`
  accepts the link as well as the JSON (`https`, a host and port, query
  `id`, `key`, `code`; `addresses = ["host:port"]`), throwing on a
  missing part. `switchboard-remote pair '<link>'`
  (`main.swift:108`) and the headset's pasted line
  (`BrowserView.swift:106, 883`) call `parse` and so need no change. The
  JSON form still parses, so an installed headset build is
  unaffected. Owner question 3 (facts:82) is open; this is both of its
  branches at once (the QR is the link, "Copy invite" stays JSON, and
  the native parser takes either), at the price of a Swift edit the
  hook and CI do not run (`docs/design.md` A2 status), so it is
  run by hand with `swift test` and said so in the status paragraph.

## The UI

`src/ui/remote.rs` `pairing` (`:141`): the QR texture is made from
`core.pairing_url(&address)` when it returns a link, and from the JSON
as today when it returns `None` (no network address: a link has
nothing to be a link of). The URL is 153 bytes against the JSON's
189, so it fits a smaller QR version (facts:21; the exact version
unrun, facts:72). "Copy invite" still copies the JSON; a new "Copy
link" beside it copies the link, shown only when there is one. Beside
the code, one sentence states the trust step:
"A browser opens this over a certificate it cannot check: accept the
warning for this address on a network you trust. The key in the link
guards against a wrong Switchboard, not an impostor on the network."
The devices grid (`:229`) needs no change: a browser is a row with the
name the page chose, a scope, "seen", Revoke.

The page (`web/`) is a header with the host's name (from the paired
`/info`) and the device's name and scope from `/device`; a banner for
the live, polling, revoked, identity-changed and unreachable states
(each plain text); sessions grouped by project, the waiting ones
(`SessionView.waiting`) first within each project and marked, then the
host's own order, each row its name, kind, card word and waiting
reason; a session view with the last response (Claude Code kind) or
the screen (anything else) as pre-wrapped text, the screen's
`lines` set to 60 for a phone (the host's cap is 200, `info.limits.screen_lines`);
a reply box that sends and is hidden without `control`. Layout is a
single column that works at phone width.

## The tests

- `src/adapters/remote.rs` unit tests: `cookie_value` (several
  cookies, a missing one, a name that is a prefix of another),
  `same_origin` (match, port differs, scheme differs, missing),
  `web::asset` (the three paths, nothing else), and `respond_with`
  writing the headers.
- `src/core/tests.rs`: `pairing_url` with a window open, closed,
  and an empty address; the code upper-cased as `OpenPairing` stores it.
- `tests/remote.rs`: extend `http` into `http_full` returning the
  response headers, and `call` into a form taking extra request headers
  (the existing `http`/`call`/`get` stay as wrappers, so no test
  changes). The old client sends `Host: localhost`; the new tests send
  `Origin: https://localhost` to match it. New tests:
  - the page: `GET /`, `/app.js`, `/app.css` answer 200 with the
    types, the CSP (no `unsafe-inline`, contains the five directives
    above), `Referrer-Policy`, `Cache-Control: no-store`, `nosniff`,
    `Connection: close`; `POST /` is 405 with `Allow: GET`;
    `/index.html` and `/favicon.ico` are 404; the bodies contain
    neither the server id nor the key; six sequential page loads on a
    full-looking pool all succeed (`Connection: close` frees the slot);
    the 61st in a minute is 429. The existing
    `info_is_served_to_anyone...` test (`:426`) is unchanged and still
    holds.
  - the page references only what the host serves: every `src` and
    `href` in `index.html` is `/app.js`, `/app.css` or `data:,`, and
    every `/api/v1/...` string in `app.js`, with `{}` for ids,
    is in a list of routes the test requires the port to answer
    (`info`, `pair`, `device`, `projects`, `projects/{}/sessions`,
    `waiting`, `sessions/{}/last-response`, `sessions/{}/screen`,
    `sessions/{}/send`, `ops/{}`, `events`); `app.js` contains no
    `eval`, no `innerHTML` assignment and no `http://` or `https://`
    literal.
  - web pairing end to end, as the page does it: `GET /info`, `POST
    /pair` with `web: true` and the host's key and code; the reply has
    no `token` and a `Set-Cookie` with the name, `Path=/api/v1`,
    `HttpOnly`, `Secure`, `SameSite=Strict`; `GET /device` with the
    cookie returns the device's name and scope; the window is closed by
    it (single use). A non-`web` pairing still returns the token and
    sets no cookie (the existing pairing tests at `:477, :582, :635` are
    unchanged).
  - the cookie's rules: `POST .../send` with the cookie and no `Origin`
    is 403 `origin`, with a foreign `Origin` 403 `origin`, with the right
    one it is 200 for a `control` device and 403 `scope` for `read`; a
    `GET` with the cookie and no `Origin` is 200; a socket upgrade with
    the cookie and the right `Origin` receives `hello`, without it is
    refused; a bearer needs no `Origin` and wins over a wrong cookie;
    a cookie named for another `server_id` is ignored (401).
  - revocation: `RevokeDevice`, then `GET /device` with the cookie is 401
    `unauthorized` with `Max-Age=0` for the cookie, and an open socket
    closes, using the existing revocation test's subscriber helper
    extended to send the cookie and `Origin`.
  - fixtures: `device` is added to `write_fixtures` and to the
    decode list; `client/Fixtures/device.json` is generated with
    `SWITCHBOARD_WRITE_FIXTURES=1`. No Swift model reads it yet.
- `tests/ui.rs:7474`: unchanged for the JSON ("Copy invite"). Added:
  the trust sentence is on screen with the window open; when the
  machine has an address (`remote::local_address()` is `Some`), "Copy
  link" puts a string starting `https://` with `code=<code>` on the
  clipboard, as the existing test reads it. The test is conditional
  because CI may have no routable address; the core test is the
  unconditional check.
- Swift `FixturesTests.swift:157`: a sibling of `testInviteParsesTheQRText`
  for the link, and a link missing its code throws. By hand.
- **Not tested by this commit, and said so:** the page's script
  runs in no test; no browser or JS runtime is in the hook or CI, and the
  plan adds none. The checks above pin what the host serves and accepts;
  the page's behavior is checked by hand against the issue's acceptance
  (open the pairing window, scan, accept the warning, see the sessions,
  watch a session start waiting move to the top, send a reply, revoke
  and see the page say so) on the owner's phone and a second Mac.
- `spikes/23-browser-page/README.md` (new, one row in `spikes/README.md`):
  CLAUDE.md asks for a spike before building on a risky mechanism and
  there is none for browsers (the native pin's spike is
  `spikes/22-remote-port`). It lists the device checks of facts:76 and
  facts:64-71 (click-through, `fetch`, the socket, the cookie stored and
  sent, `connect-src` on `wss`, camera prefetch, connections held by a
  page load) with every result "not run". The commit lands with the
  checks unrun, as the visionOS pinning delegate is; the owner's run
  settles decisions 1 and 2 and may call for the follow-ups named
  there.

## Docs

`docs/remote-client.md` §4.1 (the page's place in the pools: static
replies close the connection, answered on the connection thread, a
`page` window), §4.3 (the three static paths and `GET /device`, and that
"everything else on the wire is refused" is the `/api/v1` allowlist
plus these three), §5 (the cookie, the Origin rule, `?code=` link,
decision 1 as §5.1, the lines at 306, 309, 312, 313, 317 amended); the
`## Remote client (2026-10-12)` section of `docs/design.md` (`:3107`)
gets a dated status paragraph with the known gaps (no tombstone, no
excerpt, the device checks unrun, the Swift edit unrun); `README.md`
(Layout rows for `src/adapters/web.rs` and `web/`, the changed rows for
`src/adapters/remote.rs`, `src/ui/remote.rs`, `tests/remote.rs`, the
`pbpaste` line at `:277` now also saying the link pastes); one
`CHANGELOG.md` line under Unreleased. The commit carries no tool
attribution trailer: CLAUDE.md overrides a harness reminder, and a
body is checked for `claude.ai` before any `gh pr create`. Nothing in
the page, the tests or the docs names a client or a client's
repositories.

## Order inside the commit

Each step compiles and the app runs:
1. `RemoteReply.set_cookie`, `respond_with`, the three request fields,
   `cookie_*` and `same_origin` with their unit tests.
2. `adapters/web.rs`, `web/` with a stub page, the `Limits.page`
   window, the static arm in `connection`, the page tests.
3. `authenticate`, `remote_pair`'s `web` field, `GET /device`, and the
   cookie, Origin and revocation tests.
4. `app.js` and `app.css` in full; the references test.
5. `pairing_url`, the dialog's link, sentence and "Copy link";
   the core and UI tests; the Swift parse and its test.
6. The spike README, the fixtures, the docs, the changelog.
Then `cargo test --locked --workspace`, `cargo clippy --locked
--workspace --all-targets -- -D warnings`, `cargo fmt --all`.

## Commit message

Serve a browser page from the remote port, paired from a link, with a cookie credential and an Origin rule

## Contract additions

- `adapters::remote`: `RemoteRequest.cookie`, `.origin`, `.host`
  (`Option<String>`); `RemoteReply.set_cookie` (`Option<String>`);
  `cookie_name`, `cookie_value`, `same_origin`; a `page` rate window
  (60 a minute); `respond_with`.
- `adapters::web` (new): `Asset`, `asset`, `HEADERS`; files under `web/`.
- Wire, additive, `REMOTE_PROTOCOL` still 1: static `GET /`,
  `GET /app.js`, `GET /app.css` outside `/api/v1/`; `GET /api/v1/device`
  returning `{device_id, name, scope}`; `POST /api/v1/pair` accepts
  `web: true` (reply without `token`, with `Set-Cookie`
  `sb-<server_id>`); a request with no bearer is authenticated by that
  cookie, and a cookie-authenticated non-`GET` or upgrade needs
  `Origin` equal to `https://<Host>` or is refused 403 `origin`; a 401 for
  an unknown cookie clears it.
- `AppCore::pairing_url(&self, address: &str) -> Option<String>`.
- `PairingInvite.parse` (Swift) accepts the link as well as the JSON.
- No change to `model.rs`, `AppAction`, `Effect`, `ports/`, `control/`,
  `SCHEMA_VERSION` (16), `VIEWS_SCHEMA_VERSION` (7) or
  `IDENTITY_SCHEMA_VERSION` (1).
