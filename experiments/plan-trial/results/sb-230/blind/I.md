# Plan: a web control surface on the remote port (issue #230)

Line references are to the worktree at `7fe8949` (branch
`dispatch/230-a-web-control-surface-the-remote-port-se`, clean).

## Where this departs from the issue

- **"The key in the URL lets a genuine page refuse to send the code to a
  host whose `/info` key differs."** The code does not bear out the
  strength of that. `/info` returns `spki_sha256` as a JSON string
  (`remote_info`, `src/app.rs:3568`) over the same TLS connection the
  page was loaded on, and a page script cannot see the connection's
  actual public key. A host that terminates TLS can serve a page and an
  `/info` that agree with each other. The comparison catches a stale or
  mistyped QR that points at a different Switchboard, and nothing else.
  The plan keeps the comparison for that, and nowhere calls it a check on
  the host. It also means a browser pairing **weakens the binding rule of
  `docs/remote-client.md` §5 and design.md's "Trust boundary", "pairing
  verifies the key before the code is sent"**: the browser sends the code
  (first in the `GET /?code=…`, then in `/pair`) to whatever answered the
  owner's click on the certificate warning, and the token comes back to
  that peer as a `Set-Cookie`. The plan writes that down as a deliberate,
  scoped amendment in §5 and design.md (decision 1) and **does not
  implement it until the owner signs it off**; the native paths keep the
  rule unchanged.
- **"The one-line excerpt the card shows."** The excerpt is not on the
  wire. It is built in the UI from the parsed conversation
  (`agent_excerpt`, `src/ui/cards.rs:558`, called at `:178`) or from a
  pane caption. `SessionView` (`control/src/lib.rs:646` and the fields in
  `client/Fixtures/session.json`) carries `card`, the state label
  (`"starting"`, `"at a prompt (…)"`, `"exited (0)"`), plus
  `waiting_reason` and `notes`. Putting an excerpt in a listing means a
  transcript parse per session, which §4.1 keeps off the frame. The
  first cut shows the `card` label, and `waiting_reason` for a waiting
  session. The last response arrives on opening the session. An excerpt
  is a later, separate read-model item.
- **"A device's scope on the page."** The scope is only in the `/pair`
  reply, and a dialog change takes effect on the next request. After a
  reload the page cannot know it, and it cannot read its cookie. The
  plan adds `GET /api/v1/me`. The issue lists no endpoint, but the
  device's scope, "revoked" and "still paired" all need one.
- **"Any path outside `/api/v1/` is refused."** True today
  (`strip_prefix("/api/v1/")`, `src/app.rs:3023`, and the 404 above it).
  The plan serves exactly three fixed paths outside it, before the
  frame is involved. It does not widen the prefix match.
- **"The `/pair` reply"** is `{token, scope, server_id, device_id}`
  (`src/app.rs:3656`). Decision 2 needs the token kept from script, so a
  browser asks for a cookie and its reply has no `token`. The native
  clients' request and reply are unchanged, so `pair.json` still decodes.

## The record

No change to any record. A browser is a `PairedDevice` (`src/ports/remote.rs:96`:
id, name, scope, `token_sha256`, created, `last_seen`) in `devices.json`,
saved through `RemoteFiles::save_devices` (`src/adapters/remote.rs:83`)
and found by `device_by_token` (`src/core/remote.rs:145`). Its name is
`device_name` from the page ("Safari on iPhone", from `navigator.userAgent`),
which `remote_pair` already takes (`src/app.rs:3650`) and defaults to
"Device". No `kind` field is added, so none of these versions moves:
`SCHEMA_VERSION` (16, `src/core/model.rs:12`), `VIEWS_SCHEMA_VERSION`
(7, `src/core/model.rs:612`) and `IDENTITY_SCHEMA_VERSION` (1,
`src/ports/remote.rs:17`). `devices.json` has no version of its own, only
the array. Should an implementer find a device kind necessary after all,
the bump is the value in `model.rs` or `ports/remote.rs` at that time plus
one, with a step in `adapters::store::migrate`. Another ticket may have
taken the next number first.

**Rehydration.** The host side is nothing new. The page keeps no state on
the host, `devices.json` survives a restart, and the same token hash is
accepted. `epoch` changes on restart and the page refetches (§4.4). The
device side is new, and has two parts:

1. The token, in an `HttpOnly; Secure; SameSite=Strict` cookie that
   script never reads (decision 2).
2. A non-secret `localStorage` entry `{server_id, spki_sha256, name}`
   written at pairing, in a `try`. Without it the page cannot tell
   "never paired" from "revoked" or "identity changed", because it
   cannot read the cookie. A reload does this:
   - read the entry;
   - `GET /info`, and compare `server_id` and `spki_sha256`: a mismatch
     shows "identity changed, pair again" and sends nothing;
   - `GET /me`: 401 with an entry shows "revoked" and clears the entry,
     401 without one shows the unpaired screen.

   With `localStorage` unavailable the page still works for the session
   and shows the unpaired screen after a reload that finds no cookie.

The origin is `https://<ip>:<port>`, so cookie and entry belong to that
address. A Mac whose address changes is a new origin to the browser and
needs pairing again. The spec says so as a known gap, because a phone has
no Bonjour to follow the host.

## The signals

None new. The page is a fourth speaker of the existing protocol (§4.4):
it opens `wss://<host>/api/v1/events`, waits for `hello`, snapshots
`projects`, `projects/{id}/sessions` and `waiting`, applies buffered
events by revision, and refetches on `changed {session | sessions |
waiting | projects}`. A session that starts waiting changes the `waiting`
collection, so it moves to the top without a reload. For the open session
it sends `watch {conversation | screen}` when the view opens and
`unwatch` on leaving, and refetches on `changed`. On `resync_required` or
a new `epoch` it reloads everything, and it reconnects with back-off.
Redaction (§4.3) is applied before every cut on the host and is unchanged.
The page renders every host-supplied string with `textContent`, never
`innerHTML`, so a transcript cannot inject markup even if the CSP were
relaxed. The spec states that rule, and a test enforces it.

## The actions and effects

No `AppAction`, no `Effect`, no core state: serving a page and
authenticating a cookie are the port's job (`src/adapters/remote.rs`,
`serve_remote`), as `/info` is. Pairing, revoking and scope changes
already go through `Paired`, `RevokeDevice` and `SetDeviceScope`.

**Decision 1: the certificate.** Click-through, not a downloadable
certificate. A downloaded certificate must be installed and then trusted
in full on the device (on iOS a profile, then Certificate Trust Settings).
That flow is heavy and easy to get wrong. It also trusts the certificate
for every name the device reaches, a wider grant than a per-address
exception. The click-through grants one browser an exception for one
address, which is all this surface needs. What it buys is encryption on
the wire and a browser that remembers the exception. What it does not buy
is any proof of whose certificate it is: on a hostile network an impostor
can present its own, relay the real host's `/info`, show the genuine key
digits, and the owner's click is what lets it. **So the page's key is not
a check and the plan does not tell the owner to compare it.** The page
still shows the host name and short key from `/info`, labelled "as the
host reports it", only to tell two Switchboards apart and to drive the
"identity changed" state. The browser's own certificate viewer shows the
certificate's fingerprint, not the SPKI hash, and a phone's shows neither
usefully, so no comparison is offered there either.

The amendment, for the owner to sign off (this is the one open decision
the plan cannot make): **a browser pairs without the key having been
verified first. The only trust step is the owner's decision to open the
pairing window and scan its QR on a network they trust, with the code
single-use, five minutes and three wrong tries.** What bounds the damage:
the scope is chosen in the window (`read` by default), the new device
appears in the paired-devices dialog by name the moment it pairs, and
Revoke closes it. The window's text and the dialog say so in one
sentence: "A browser cannot verify this Mac's key. Pair a browser only on
a network you trust; check the device appears below, and revoke it if it
is not yours." §5 and the "Remote client" section of design.md gain that
sentence as a deliberate exception for the web page. If the owner declines
the amendment, the web page is not built, and the plan has no second
design to offer: a downloaded, trusted certificate verifies the host but
carries the blast radius above.

For a Mac or other desktop browser the window also shows the
certificate's SHA-256 fingerprint (a new line, "Certificate", computed
from `identity.certificate`; Chrome and Safari on macOS show the same
value in their certificate viewer) as an optional comparison the owner can
make by hand. It is offered as optional and is not required for the page
to work.

The certificate stays as `make_identity` makes it
(`src/adapters/remote.rs:95`, no SubjectAltName). The spike records
whether Safari and Chrome offer "proceed" for it. If either does not, the
fix is an IP SAN added when the certificate is made at serve time. The
pin is the key, so reissuing the certificate invalidates no pairing.
`a_change_of_address_rebinds_the_port` (`tests/remote.rs:1494`) is where
that would be tested.

§5's rule is kept and amended: **before pairing, no bind serves anything
but `/info`, `/pair` and the page's own three files.** Every other path is
refused, loopback or not.

**Decision 2: the token reaches the socket as a cookie.**
`POST /api/v1/pair` takes an extra body field `"session": "cookie"`. The
reply then omits `token` and sets
`Set-Cookie: __Host-sb-<simple id>=<token>; Path=/; Secure; HttpOnly; SameSite=Strict; Max-Age=34560000`
(400 days, the cap browsers apply; the token does not expire and
revocation is the host's). **The name's one form is pinned:** the prefix
`__Host-sb-` followed by `identity.server_id.simple().to_string()`, which
is the `Uuid` as 32 lowercase hex digits with no hyphens, so the name is
always 42 characters. `Uuid`'s `Display` (the hyphenated 36 characters,
which is what `/info`, the JSON invite and the URL's `id=` carry) is not
used in the cookie name. One helper, `remote::cookie_name(&Uuid) -> String`
in `src/adapters/remote.rs`, builds it, and `Set-Cookie`, `read_request`'s
filter and `authenticate` all call it or its prefix constant, so the three
cannot drift. **The name carries the host's `server_id`**
because cookies ignore ports: a second instance on the same IP
(`SWITCHBOARD_REMOTE_PORT`, or a second data directory) has its own
`server_id` and so its own cookie, and neither overwrites the other
(`__Host-` allows any name after the prefix). `authenticate` reads only
the cookie carrying its own `server_id`. A 401 therefore means this host
does not know the token, never that another host replaced it, so the
page's "revoked" state is not reachable by a collision. Two browsers' tabs
on different ports of one IP stay independent as well. A script never holds the token, `fetch` and
`new WebSocket` both carry the cookie, and the token is in no URL and no
`Sec-WebSocket-Protocol`. `localStorage` is readable by any script on the
origin and a cookie is not, which settles where the token lives. Native
clients send no `session` field and are unchanged.

`authenticate` (`src/app.rs:3127`) takes the token from `Authorization:
Bearer` and, absent that, from the cookie. Cookies are not isolated by
port and `SameSite=Strict` does not separate ports of one IP (same site,
different origin), so a page served by another service on the same IP
sends the cookie with its requests, reads and upgrades included. A read
whose response a hostile page cannot read still leaks timing and, for a
subscription, holds a socket. So the rule covers **every
cookie-authenticated request, `GET` and upgrade included**; a bearer
request is never subject to it. Before any translation or reply, a
cookie-authenticated request is refused 403 `origin` unless all of these
hold:

1. `Host` is an IP literal, optionally with a port (never a name). A
   DNS-rebinding page reaches the host under an attacker's name, and a
   cookie bound to the IP is not sent there; this check also stops a
   request that arrives with a name in `Host`.
2. `Sec-Fetch-Site`, when the browser sends it, is `same-origin`
   (another port on the same IP is `same-site`, and is refused). The
   header is on Safari 16.4+, Chrome and Firefox.
3. `Origin`, when present, equals `https://<Host header>`. A WebSocket
   upgrade and every `POST` carry `Origin` in every browser, so for them
   it is required. A same-origin `GET` from `fetch` carries `Origin` in
   some browsers and not in others, so for a `GET` the rule is: `Origin`
   if present must match, and with no `Origin` the request needs
   `Sec-Fetch-Site: same-origin`. A cookie `GET` with neither is refused,
   which also refuses a browser too old to send either; that browser
   cannot use the page, and the page says so ("this browser is too old").

Revocation needs no new path. The record is gone, the cookie's hash is
unknown, the next request is 401 `unauthorized`, and the socket closes as
it does today.

**This decision is conditional on the spike.** If the spike shows that
Safari on iPhone or Chrome refuses to store the `Secure` cookie from a
click-through origin, or does not send it on `fetch` or `wss://`, this
decision does not stand and the work stops before the cookie step (see
"Order"). The fallback is a `localStorage` token and a single-use ticket
for the socket, which puts the token within reach of any script on the
origin, changes the threat model, and so goes back to the owner as a new
plan rather than being built speculatively.

**Port changes** (`src/adapters/remote.rs`, `src/app.rs`):

- `RemoteRequest` (`:360`) gains `cookies: Vec<(String, String)>`,
  `origin`, `host` and `sec_fetch_site` (`Option<String>`), filled in
  `read_request` (`:444`) beside `bearer` (`:487`). `read_request` does
  not know the identity, so it keeps only the cookies whose names are
  exactly 42 characters and start `__Host-sb-` (at most four; the name's
  length is fixed by the pinned form in decision 2, and each value is
  capped at 64 bytes, the token being 43) and `authenticate` picks the one
  whose name equals `cookie_name(&identity.server_id)`. None of them is
  logged. `RemoteReply` (`:389`) gains `headers: Vec<(String, String)>`,
  which `respond` (`:530`) writes after `Content-Type`. That means adding
  `headers: Vec::new()` to the `RemoteReply { … }` literals: counted with
  `grep -o`, 28 in `src/app.rs` and 1 in the adapter (`:829`), 29 in all,
  plus any the compiler finds in `tests/` or the fakes. A mechanical edit
  that `cargo build` completes.
- **Static paths are answered on the connection thread**, in `connection`
  (`:770`) before the request is handed to the frame, from a fixed `match`
  in a new `src/adapters/web.rs` (`pub fn asset(method, path) -> Asset`).
  The path is matched exactly and never joined to anything. They cost the
  frame nothing and do not use the 64-deep inbox. The set, `GET` only
  (anything else on these paths is 405 with `Allow: GET`; the query is
  ignored and never read):

  | Path | `Content-Type` |
  |---|---|
  | `/` | `text/html; charset=utf-8` |
  | `/app.css` | `text/css; charset=utf-8` |
  | `/app.js` | `text/javascript; charset=utf-8` |

  Anything else not under `/api/v1/` stays 404 `not_found` from
  `serve_remote`. Every static reply carries:
  - `Content-Security-Policy: default-src 'none'; script-src 'self'; style-src 'self'; connect-src 'self'; img-src 'self' data:; base-uri 'none'; form-action 'none'; frame-ancestors 'none'`.
    There is no `unsafe-inline` and no host. The HTML has no inline script
    or style, and carries `<link rel="icon" href="data:,">` so no favicon
    is requested. If the spike shows Safari does not match `'self'` to
    `wss:`, the explicit `wss://<Host>` source is added at serve time.
  - `X-Content-Type-Options: nosniff`.
  - `Referrer-Policy: no-referrer`. **This is load-bearing:** the page's
    own URL holds the code, and without it the browser sends that URL as
    `Referer` with the request for `/app.js`.
  - `Cache-Control: no-store`, so nothing is cached across versions and no
    version is put in a URL.
  - `Cross-Origin-Opener-Policy: same-origin`.
  - Never a `Set-Cookie`.
- **Pools and rate** (`:567-587`, `:631`). A static reply is
  `authenticated: false`, so a browser's connections sit in the pool of 4
  until its first authenticated call (the `events` upgrade or a read) moves
  them to the paired pool (`:835`). A browser may open up to six
  connections for a page. Past four, an accept is closed at once and the
  browser retries. The page is three files so keep-alive covers it with
  one or two. The pools are not changed: the spike counts the connections
  of a real page load, and only a measured failure changes the pool
  accounting. The only addition is `PAGE_PER_MINUTE = 60`: a `page`
  `RateWindow` beside `pair` and `info` in `Limits` (`:630`), and
  `limited` (`:631`) changes from an exact match on two constants to
  `if web::is_page(path) { &self.page }` before the existing arms, where
  `web::is_page` is the same fixed `match` that serves the files. One
  window is shared by the three paths. A load is three requests, so
  twenty loads a minute. Only page requests count: the events upgrade and
  every API call are not in this window, so a reconnect storm after a
  host restart cannot lock the owner out of reloading the page, and a
  reload loop cannot starve `/info` or `/pair`, which keep their own
  windows. A 429 on a page path is the same JSON error body `/info` gets,
  with `Cache-Control: no-store` and `X-Content-Type-Options: nosniff`
  (so a browser never runs the body as script) and **no CSP**, which a
  JSON error that is not a document has no use for; `respond` takes the
  `headers` for it.
- **`GET /api/v1/me`** (`serve_remote`, after `authenticate`): `{device_id,
  name, scope}`, any paired device. The cheapest "am I still paired" probe
  and the one the revoke test polls. It goes in the §4.3 table, and its
  reply is a shared fixture: `client/Fixtures/me.json`, written by
  `write_fixtures` (`tests/remote.rs:1587`), decoded by a small
  `DeviceMe` `Decodable` in `SwitchboardProtocol` and a line in
  `FixturesTests.swift`. No native view calls it today; the fixture is so
  the wire shape is pinned on both sides like every other body.
- **Unpaired with no code and no cookie**, the page shows the host name
  and key (`/info`), "Not paired", and "Open Remote devices on the Mac,
  allow pairing, and scan its QR code", with `pairing_open` from `/info`.
  With a code and pairing closed (403 `pairing_closed`) it says so. On
  `wrong_key` it says "this is not the host whose code was scanned". On
  `wrong_code` it says the code was wrong or used.

**Decision 3: one URL.** The pairing window's payload becomes
`https://<ip>:<port>/?code=<code>&key=<spki_sha256>&id=<server_id>` (the
address is IPv4, from `local_address`, `src/adapters/remote.rs:962`; the
alphabet needs no escaping). A pure
`core::remote::invite_url(address, server_id, spki, code)` builds it, so a
core test pins the shape. **The QR carries the URL, and nothing is
removed.** "Copy invite" keeps copying the JSON exactly as today
(`{server_id, spki_sha256, code, addresses}`), so every installed native
client and every pasted invite still works, `addresses` keeps its array
shape, and `PairingInvite`'s decoder is not narrowed. A new "Copy link"
button copies the URL. The QR is the only thing whose payload changes, and
no shipped client reads it: the headset has no camera and the CLI takes
text (a search of `client/Sources` for a camera or scanner finds none), so the change
breaks nothing. The host's address is a single `ip:port` in both forms
today (`local_address`), so nothing is lost by the URL carrying one.
The Swift `parse` gains the URL form for a link pasted into the headset or
the CLI, in addition to JSON, and not instead of it. `PairingInvite.parse` (`client/Sources/SwitchboardProtocol/Info.swift:114`)
reads JSON when the text starts with `{`, else a URL, taking `addresses`
from its host and port, `key` as `spkiSha256`, `id` as `serverID`. It is
the single entry for the CLI (`main.swift:107`) and for the headset's paste
(`BrowserView.swift:106`, `:883`), so neither changes beyond `main.swift`'s
usage text (`:10`). `switchboard-remote pair` keeps both forms: one
argument (URL or JSON) or three.

**The code in a GET.** The browser requests `/?code=…`. `read_request`
already parses a query (`:497`), the static arm never reads it, and
`refuse` logs method and path only. `app.js` begins with
`history.replaceState(null, "", "/")` after copying the three values into
locals, before any request. The code is single-use and lives five minutes,
which bounds what a camera app or browser history kept of the URL.

## The UI

**The host** (`src/ui/remote.rs`): `pairing` (`:141`) keeps the JSON
payload for "Copy invite" (`:160-166`, unchanged) and builds the QR from
`invite_url`. The text beside the QR changes from "Scan, or type the key
and the code" to "Scan with a phone's camera, or type the key and the
code in the headset", followed by the sentence from decision 1. Beside
"Copy invite" sits "Copy link". A "Certificate" line shows the SHA-256 of
the certificate, labelled optional. The paired-devices dialog (`:230`)
gains one line under its heading with the decision 1 sentence. A browser
appears in the list by its name, with scope combo and Revoke as every
device does.

**Reachability.** `bind_remote` (`src/app.rs:2924`) binds `127.0.0.1`
unless `identity.listen.lan` is set (the "Allow devices on the local
network" checkbox, `src/ui/remote.rs:110-117`), and `local_address()` is
only the address the system routes outward on, not proof of the bind. A
phone cannot reach a loopback bind, so while `identity.listen.lan` is
false the pairing window **suppresses the QR and "Copy link"** and shows
"This Mac is listening on this computer only. Allow devices on the local
network above, then scan." The code, key and "Copy invite" stay, since
the CLI on the same Mac still uses them. When the bind is on the network
the QR uses `local_address()` and the bound port (`cx.remote_port`, as
today); with no network address (`None`) the QR is also suppressed with
"No network address found". `invite_url` stays pure; the decision is in
`pairing`.

**The page** (`web/index.html`, `web/app.css`, `web/app.js`; no framework,
no build step, no dependency; light and dark by `prefers-color-scheme`;
one column at phone width):

- *Header*: the host name, its short key labelled "as the host reports it"
  (identification, not a check), the device's scope
  (`read`/`control`) from `/me`, and a connection dot.
- *List*: sessions grouped by project, the waiting ones first and marked,
  each with its name, `kind`, `liveness` and `card` label, and for a
  waiting session `waiting_reason`.
- *Session view*: the last response (`last-response`, `text`, with
  `truncated` said) for a Claude Code session, or the screen
  (`screen?lines=200`, monospace) for anything else, refreshed on
  `changed`. If `trust_question` is true, a "Folder trust question" line
  and a Trust button for a `control` device.
- *Reply box*: shown for a `control` device only. It posts
  `sessions/{id}/send` with `{op, text}`, with `op` a UUID per click
  (`crypto.randomUUID`, falling back to `getRandomValues`, since a
  click-through origin may not be a secure context). A timeout is followed
  by `GET ops/{op}`, never a blind resend (§4.5).
- *States said plainly*: unpaired, pairing closed, wrong key, "identity
  changed, pair again", "revoked", host unreachable (retrying), and the
  429 and 503 refusals.

## The tests

The tests are named in the order of the pieces above. The existing
`the_allowlist_refuses_everything_else_with_no_side_effect`
(`tests/remote.rs:682`) must keep passing.

`tests/remote.rs`: `http()` (`:245`) drops response headers, so a second
helper returns `(status, headers, body)` and `http` wraps it. Then:

- **Static paths.** Each of the three answers 200 with its `Content-Type`
  and the full header set, with no `Set-Cookie`. The CSP contains no
  `unsafe-inline`, no `http:` or `https:` source and no `*`. A
  `POST` to `/` is 405.
- **Not in the set.** `/app.js.map`, `/web/`, `/index.html`, `//`,
  `/../Cargo.toml` and `/app.js/` are 404, and each is refused to an
  unpaired caller (no file read).
- **The page is self-consistent.** Every `src` and `href` in the embedded
  HTML is a path the host serves. Every `"/api/v1/…"` literal in `app.js`
  matches a row of the §4.3 allowlist (the test keeps a table), so a typo
  is caught. `app.js` contains no `innerHTML`, `outerHTML`,
  `document.write`, `eval` or `new Function`.
- **Rate.** The 61st page request in a minute is 429, in the shape of
  `pair_is_rate_limited_per_listener` (`:1406`).
- **Pairing end to end, as the CLI does it.** Build the URL with
  `invite_url`, parse it back, compare `key` with `/info`, `POST /pair`
  with `"session":"cookie"`. Assert the `Set-Cookie` attributes
  (`__Host-`, `Secure`, `HttpOnly`, `SameSite=Strict`, `Path=/`, no
  `Domain`), that the cookie's name is exactly `cookie_name(server_id)`
  (42 characters, 32 lowercase hex after the prefix, no hyphen), and no
  `token` in the body. Then `GET /me` and a read with the
  cookie and no bearer succeed. A bearer-only pairing and read behave as
  before.
- **Origin, every cookie request.** A cookie `POST …/send` with a foreign
  or missing `Origin` is 403 `origin` and sends nothing to the pane; with
  the right one it is typed once per op. A cookie `GET` is accepted with
  `Sec-Fetch-Site: same-origin` and no `Origin`, accepted with a matching
  `Origin`, and refused (403 `origin`) with `Sec-Fetch-Site: same-site` or
  `cross-site`, a foreign `Origin`, or neither header. A cookie request
  with a non-IP `Host` (`evil.test`) is refused. The upgrade with a
  foreign or missing `Origin` is refused; with the right one it gets
  `hello`. A bearer request with none of these headers is unaffected.
- **Two hosts, one IP.** Two ports built in one test (`port()` twice) pair
  the same client jar; each cookie authenticates only its own host, the
  other's cookie sent to it is 401, and neither pairing disturbs the other.
  The two names differ and both are 42 characters. A request carrying a
  cookie named `__Host-sb-` with the hyphenated id (36 characters) or one
  of any other length is not read, and is 401 like an absent cookie.
- **Revoke** (extend `:658`): after `RevokeDevice`, the cookie's `GET /me`
  is 401 `unauthorized`, and a subscribed cookie device's socket closes.
  A scope change shows in `/me` on the next call.
- **Fixtures** (`write_fixtures`, `:1587`): `me.json` is added, and
  `FixturesTests.swift` decodes it with `DeviceMe`.
- **Page order.** A test reads the embedded `app.js` and asserts
  `history.replaceState` occurs before the first `fetch(` or
  `new WebSocket(`, so the code leaves the address bar before any request.

`tests/ui.rs`: extend `remote_devices_dialog_opens_pairing_and_revokes_a_device`
(`:7424`). Its clipboard assertions (`:7484-7492`) on "Copy invite" stay
as they are: JSON with `code`, `spki_sha256` and an `addresses` array. The
test sets `listen.lan` true (its `Listen::default()` is loopback), clicks
"Copy link" and asserts the text starts with `https://` and its `code`,
`key` and `id` query values are the window's code, the identity's
`spki_sha256` and its `server_id`. The dialog text shows the sentence from
decision 1. A second test leaves `listen.lan` false and asserts that
"Copy link" is absent and the notice about the local network is shown,
with "Copy invite" still present.

`src/core/tests.rs`: `invite_url` is exact for a known input.

Swift (`client/Tests/SwitchboardClientCoreTests/PairingTests.swift`):
`parse` accepts the URL form and the old JSON, and refuses a URL with no
`code`. Run with `swift test` in `client/`, which CI does not.

No JS test runner exists and the issue forbids a build step. The Rust
tests above pin the page's text (no `innerHTML`, the `replaceState`
order, the API literals) and the host's behavior, not the page's running
in a browser. The claims that need a browser (the cookie stored and sent,
`textContent` rendering a hostile transcript line as text, the WebSocket
carrying the cookie) are covered by the spike and the acceptance run on a
phone, and the plan says so rather than counting a grep as a browser test.

**Spike first** (CLAUDE.md): `spikes/23-web-surface/README.md` with
measured results, written before the page is relied on. It answers on
Safari on an iPhone and Chrome on a Mac:
- does a click-through certificate (as `make_identity` makes it) get a
  "proceed" option, and is a `__Host-` `Secure; HttpOnly; SameSite=Strict`
  cookie stored from it, sent on `fetch` and on `wss://`;
- does `connect-src 'self'` allow the same-origin `wss://`;
- how many connections a page load opens against the pool of 4;
- does the camera open the `https://ip:port/?…` URL, and does `replaceState`
  keep the code out of the history.

The spike needs a phone. It is a **gate, not a formality**: the cookie
and `wss://` rows (first bullet) must be measured on Safari on an iPhone
and on Chrome before the cookie step of the order below begins. An
implementer without a phone does not proceed past step 2; they hand the
plan back to the owner with the unmeasured rows named. Only the pool,
camera and `replaceState` rows may be left for the acceptance run, since
none of them can invalidate a decision.

**Acceptance** (the issue's, by hand): open the pairing window, scan with a
phone, accept the warning, see the sessions; start a session waiting and
watch it move to the top; send a reply and see it arrive; revoke the phone
in the dialog and the page says "revoked" on its next request.

## Docs, in the same commit

- `docs/remote-client.md`:
  - §4.1: the page rate and the pool note;
  - §4.3: rows for the three static paths and `/me`;
  - §5: the pairing QR is a URL, the **deliberate exception to "pairing
    verifies the key before the code is sent" for a browser** and what
    bounds it (decision 1), the cookie and its per-`server_id` name (the
    "sent only as a bearer header" bullet gains "or the browser's
    cookie"), the Origin, `Sec-Fetch-Site` and IP-`Host` rule for every
    cookie request, the amended "nothing but `/info`, `/pair` and the
    page" rule, and the per-address known gap;
  - §11: the tests above;
  - a §0 revision note.
- `docs/design.md`: there is no "Milestone N status" for the remote port;
  its status is the one section "Remote client (2026-10-12)"
  (`:3107-3584`, checked: the next heading is "Versioning and updates" at
  `:3585`), whose last bullet is "Known gaps at the first merge" (`:3573`).
  That bullet is a record of the first merge and is left alone. The plan
  adds, before it, a bullet "The web surface" (what is built, the browser
  trust amendment, the cookie, the Origin rule, the spike's measured
  results) and, after it, a bullet "Known gaps of the web surface": no
  excerpt in the list, no Dispatch tickets, conversation, drafts or runs,
  pairing per address, a browser too old to send `Sec-Fetch-Site`.
- `README.md`: Layout rows for `web/`, `src/adapters/web.rs` and
  `spikes/23-web-surface`. The `web/` row says the three files are
  embedded with `include_str!` in `src/adapters/web.rs`, so they compile
  into the binary: `scripts/bundle.sh` copies binaries only and needs no
  change, the Linux CI build needs nothing outside the crate, and
  `Cargo.toml` has no `include` list that would leave `web/` out; the `adapters/remote.rs` row (`:383`) gains the
  cookie, the Origin rule and the static paths; the `ui/remote.rs` (`:423`)
  and `tests/remote.rs` (`:437`) rows; the dev-aid paragraph (`:267-279`)
  has one sentence that goes false (the dialog's "Copy invite" "puts the
  QR code's text on the clipboard", `:275-277`), and is rewritten: "Copy
  invite" copies the JSON invite (`server_id`, `spki_sha256`, `code`,
  `addresses`), while the QR and "Copy link" carry the URL. What either
  button copies pairs with `switchboard-remote pair "$(pbpaste)"`. The "Copy invite" hover text
  in `src/ui/remote.rs` ("The QR code's text, to paste into a client")
  (`:202`) and the comment above it (`:199-200`) say the same untrue thing and
  become "The JSON invite, to paste into a client". No new environment
  variable and no new script line.
- `CHANGELOG.md`, under Unreleased: "A web page on the remote port: scan
  the pairing QR code with a phone to see the sessions, which are waiting,
  and send a reply."

## Order inside the one commit (the app runs after each)

This is one commit, as the task asks, so the gate is the order and not a
second commit. Nothing is committed until every step is done; if a gate
fails the work is stopped with nothing committed, and the spike's
README (which records the failure) goes back to the owner with the plan.

0. **Owner sign-off** on the decision 1 amendment. Without it, nothing
   below begins.
1. The spike, with the cookie and `wss://` rows measured (the gate).
2. The port: request and reply fields, the static arm with a placeholder
   page, the page rate window.
3. **Only if the gate passed:** the cookie, the Origin rule and `/me`.
4. The page.
5. `invite_url`, the reachability notice, the dialog text, "Copy link",
   and the Swift parse and `DeviceMe`.
6. The docs.

Before committing: `cargo fmt --all`, `cargo test --locked --workspace`,
`cargo clippy --locked --workspace --all-targets -- -D warnings`, and
`swift test` in `client/`. The `dispatch` crate is untouched, so its Linux
clippy is not needed. Nothing in the code, tests, fixtures, docs, commit
message or pull request names a client or customer, and none of them
carries a tool-attribution line.

## Commit message

`A web page on the remote port, paired from a URL in the QR code`

**Contract additions** (copy into the commit body):

- `RemoteRequest` gains `cookies: Vec<(String, String)>`, `origin`, `host`
  and `sec_fetch_site` (`Option<String>`); `RemoteReply` gains
  `headers: Vec<(String, String)>`.
- New module `adapters::web` (`asset`, `is_page`) and the `web/` files it
  embeds with `include_str!`.
- Port paths: `GET /`, `GET /app.css` and `GET /app.js` outside `/api/v1/`,
  answered on the connection thread; `GET /api/v1/me` for any paired
  device (fixture `me.json`, Swift `DeviceMe`).
- **Conditional on the spike's cookie rows and the owner's sign-off of the
  browser trust amendment:** `POST /api/v1/pair` accepts
  `"session": "cookie"` and then sets `__Host-sb-<server_id as 32 hex,
  no hyphens>` (42 characters, built by `remote::cookie_name`) and omits
  `token`; every cookie-authenticated request (reads and upgrades too)
  needs an IP-literal `Host`, `Sec-Fetch-Site: same-origin` or a matching
  `Origin`, else 403 `origin`. If either condition fails, this item and
  the page's pairing are not in the commit.
- `PAGE_PER_MINUTE` (60) in the port's limits, one window for the page paths.
- `core::remote::invite_url`. The pairing QR carries a URL
  (`?code=&key=&id=`); "Copy invite" still copies the JSON; "Copy link" is
  new; `PairingInvite.parse` accepts the URL **and** the JSON. The QR is
  suppressed while the port is bound to loopback.
- No `AppAction`, no `Effect`, no model change, and no schema bump:
  `SCHEMA_VERSION`, `VIEWS_SCHEMA_VERSION` and `IDENTITY_SCHEMA_VERSION`
  stay as they are.
