# Plan for #230: a web control surface on the remote port

Worktree `/Users/sbgragent/.dispatch/worktrees/f96f6569` at 0.4.1; every
line number is from this tree. One commit; the app runs at every step.
Facts are cited as `facts: Settled`, `facts: Unresolved` or `facts: For
the owner`.

## Where this departs from the issue

- **The session list carries no excerpt.** The issue's list shows "the
  one-line excerpt the card shows". No wire view carries one yet:
  `SessionView` (`control/src/lib.rs:637-688`) has `card`, `waiting`
  and `waiting_reason` and no excerpt field, and adding one is a wire
  crate change plus masking on the frame (`docs/remote-client.md:201`),
  outside this cut. The list shows kind, the card's word and the
  waiting reason; the session view shows the last response or the
  screen. The spec sentence at `docs/remote-client.md:207` is reworded
  in this commit to say the field is not yet on the wire; the field is
  a known gap in design.md.
- **The code rides in the URL's fragment, not its query.** The issue
  writes `/?code=…`. A fragment is never sent to the server (facts:
  Settled, RFC 3986 §3.5), so the code cannot reach a request log by
  any future change to `refuse`, which today logs path without query
  (`src/app.rs:3754-3766`). The page reads `location.hash` and clears
  it the same way the issue says; the camera-to-browser unknown is the
  same for both forms (facts: For the owner). The URL is
  `https://<ip>:<port>/#code=<code>&key=<spki_sha256>&id=<server_id>`.
- **The browser's trust step is the certificate warning, and the page
  works without the WebSocket.** The issue offers click-through or a
  downloadable certificate. Downloading cannot work with the identity
  the spec fixed: iOS trusts no server certificate without a SAN, over
  825 days or without EKU (facts: Settled, Apple 103769), and lists a
  certificate for trust only when it is CA-shaped (facts: Settled, Apple
  DTS), so "download" means a CA plus a leaf, a different identity model
  from `docs/remote-client.md:307-308` (one self-signed certificate,
  SPKI pinned). That is a follow-up for the owner, not this plan. The
  click-through is the trust step, said in the dialog. Whether the
  bypass extends to `wss://` on iOS Safari is unresolved (facts:
  Unresolved, against it), so the page polls over `fetch` by default and
  the socket is the enhancement behind it.
- Everything else in the issue is as the tree bears it out.

## The record

No record changes. A browser is a `PairedDevice` (`src/ports/remote.rs:95-108`)
written by `Effect::SaveDevices` on `AppAction::Paired`
(`src/core/remote.rs:66-72`), its name from the page's user agent,
listed and revoked in the dialog as any device. No serialized type
changes, so no schema bump.

The page's state on the device: the token is in an `HttpOnly` cookie
named `switchboard_token_<server_id>` that the host sets and script
never reads; the page keeps only `server_id`
and `spki_sha256` from the pairing in `localStorage`, to say "identity
changed" later (both are public, `/info` serves them to anyone).

## The signals

No new signal.

## The actions and effects

No new `AppAction` or `Effect`. The changes are in the adapter, the
frame's request routing, and one pure function in the core.

### Decision 1: browser trust (the click-through)

The warning the owner clicks through on the local network is the trust
step. What the `key` in the URL buys: a genuine page refuses to post the
code to a host whose `/info` key differs, which is the native client's
check; what it does not buy: a page served by an impostor is not a
genuine page, so the owner's eyes on the warning, on their own network,
stand in for the SPKI pin. The paired-devices dialog says so in one
sentence (UI below). The host side is unchanged: no non-loopback bind
serves anything but `/info`, `/pair` and now the three page files before
pairing (§5, `docs/remote-client.md:306` amended). Chrome remembers the
bypass for a week per host (facts: Settled); Safari's persistence is
unknown (facts: Unresolved), so the dialog's sentence says the warning
may return and the page loses nothing when it does (the cookie and the
stored ids survive a reload).

### Decision 2: the token reaches the WebSocket as a cookie

The cookie is the safer store (script cannot read an `HttpOnly` cookie;
anything in `localStorage` or a `Sec-WebSocket-Protocol` value is in
script), and the settled facts say it reaches both `fetch` and the
WebSocket on an IP origin with `Secure; SameSite=Strict` and no `Domain`
(facts: Settled, RFC 6265bis and HTML "obtain a site"). Whether a browser
stores a `Secure` cookie on a bypassed-certificate origin is unresolved
(facts: Unresolved): **that is the first thing the implementer tests on
a phone, before anything else is built**, with the host's `/pair`
answering a cookie and a `curl`-free check that `/api/v1/waiting` then
answers 200 from the page. If it fails, the `Sec-WebSocket-Protocol`
transport is a follow-up (end of plan), not a second transport built
now. The page shows "pair again" when its first authenticated call after
`/pair` is refused, so a dropped cookie is a visible state, never a
silent one.

Mechanics:

- The cookie is named for the host: `switchboard_token_<server_id>`,
  the `server_id` UUID the pair reply already carries
  (`src/app.rs:3606-3674`, `"server_id": identity.server_id`; hex and
  hyphens, a valid cookie-name token). Cookies are scoped by host and
  path, not port, so two instances on one Mac (the README's dev-aid
  setup) share a jar: a fixed name would have the second pairing
  overwrite the first's token. The server id is kept across a key
  rotation (`make_identity` is called with `Some(identity.server_id)`,
  `src/app.rs:2811`), so the name is stable for a host's life. One pure
  function, `core::remote::token_cookie_name(&Identity) -> String`,
  builds the name for the `Set-Cookie` and for the lookup.
- `POST /api/v1/pair` takes an additive body field `"transport":
  "cookie"`. With it, the reply body omits `token` and carries
  `Set-Cookie: switchboard_token_<server_id>=<token>; Path=/api/v1/;
  Secure; HttpOnly; SameSite=Strict`. Without it (the headset, the CLI,
  `tests/remote.rs::pair` at 406) the reply is as today, token in the
  body, no cookie. `remote_pair` (`src/app.rs:3606-3674`) is the only
  change on the frame; `Path=/api/v1/` keeps the cookie off the page
  files, which need none.
- `read_request` (`src/adapters/remote.rs:444-517`) does not know the
  host's id, and the browser may send several hosts' cookies in one
  header, so it reads the `cookie` header and keeps every pair whose
  name starts `switchboard_token_` in a new
  `RemoteRequest::cookies: Vec<(String, String)>` field (name, value).
  `authenticate` (`src/app.rs:3127-3157`) takes `bearer` first and,
  failing that, the cookie named `token_cookie_name(identity)`; a
  cookie for another host is ignored, not an error. The native clients
  are unchanged, and a browser never sets the bearer. The WebSocket
  upgrade authenticates on the same request, so the socket needs
  nothing new; `respond_upgrade` (`remote.rs:519-527`) is unchanged.
- A 401 for an unknown token from this host's own cookie (revoked, or
  every token after a key rotation) carries `Set-Cookie:
  switchboard_token_<server_id>=; Path=/api/v1/; Max-Age=0; Secure;
  HttpOnly; SameSite=Strict`, so the browser drops it; a 401 with no
  cookie of this host's name clears nothing, and never touches another
  host's cookie. The refusal text stays "the token is not known; pair again":
  "revoked" and "never paired" remain one answer on the wire (facts:
  Settled), and the page tells them apart itself (UI below).
- Two instances on one IP differ by port only; the per-host name keeps
  their tokens apart, and each instance's lookup ignores the other's
  cookie.
- `SameSite=Strict` is the cross-site guard: a page on another origin
  cannot make the browser attach the cookie to a `fetch` or a WebSocket
  handshake to the host. No `Origin` check is added.
- The spec's "never in a URL" rule stands (`docs/remote-client.md:312`);
  the sentence gains "or as the cookie `/pair` sets for a browser".

### Decision 3: one invite, the URL

The QR and "Copy invite" carry the URL. A new pure function in
`src/core/remote.rs`, `pairing_invite(identity: &Identity, address:
&str, code: &str) -> String`, builds it (IPv6 bracketed, code uppercase,
`key` the full hex `spki_sha256` so the page compares it with `/info`
by string equality), shared by the dialog, `tests/remote.rs` and
`src/core/tests.rs`. The URL is 153 bytes, QR version 9 at 53 modules,
smaller than today's JSON at version 10 (facts: Settled); `QR_SCALE`
(`src/ui/remote.rs:42`) stays 4, and rises only if the owner finds it
unreadable (facts: For the owner). The JSON form is no longer produced
by the host; the native text path keeps parsing it so an old copied
invite still works:

- `PairingInvite.parse` (`client/Sources/SwitchboardProtocol/Info.swift:114-116`)
  branches: text starting with `https://` has its authority taken as
  text, the substring between `https://` and the first `/`, straight
  into `addresses: [that]`, so the brackets the host wrote around an
  IPv6 literal survive; the fragment after `#` is split on `&` into
  `code`, `key`, `id`. Anything else is the JSON as today. `parse` uses
  nothing from `SwitchboardTransport`. `switchboard-remote pair
  '<url>'` (`main.swift:107-125`), the headset's paste field
  (`BrowserView.swift:883`) and `SWITCHBOARD_INVITE` per line
  (`BrowserView.swift:104-108`) all pass through `parse` and need no
  other change; the three-argument CLI path (`main.swift:112-118`) is
  unchanged.

Without a network address (`remote::local_address()` is `None`,
`remote.rs:962-973`) there is no URL: the dialog shows the code and the
key with no QR and no Copy invite, and the sentence says to pair by the
CLI's three-argument form. The address is the default route's source
(facts: Settled); a VPN taking the default route makes it unreachable
from the phone (facts: For the owner), and listing every interface is a
follow-up.

### Decision 4: static paths on the allowlisted port

The served set, all `GET`, all before authentication, matched exactly
in `serve_remote` (`src/app.rs:3017-3024`) ahead of the `/api/v1/`
prefix so the allowlist stays in one place:

| Path | Content-Type | Body |
|---|---|---|
| `/` | `text/html; charset=utf-8` | `web/index.html` |
| `/app.css` | `text/css; charset=utf-8` | `web/app.css` |
| `/app.js` | `text/javascript; charset=utf-8` | `web/app.js` |

Every other path, `/index.html` and `/app.js/` included, and every
other method on these three, stays 404 `not_found` as today. Each
static reply carries:

- `Content-Security-Policy: default-src 'none'; script-src 'self';
  style-src 'self'; connect-src 'self'; base-uri 'none';
  form-action 'none'; frame-ancestors 'none'`. `connect-src 'self'`
  covers the same-host `wss://` in CSP3 and WebKit since 2022
  (facts: Settled). No inline script, no inline style, no external
  resource: the HTML has one `<link>` and one `<script src>`.
- `Cache-Control: no-store`, so a new version is what the next open
  gets and nothing is cached across versions.
- `X-Content-Type-Options: nosniff`.

Pool and window: a page load is unauthenticated and sits in the
four-slot stranger pool (`MAX_UNAUTHENTICATED`, `remote.rs:567`) until
its first authenticated reply. The count the page is designed to: three
load connections for the three files, kept alive; the first
authenticated request is a `fetch` of `/api/v1/waiting`, which moves its
connection to the paired pool and frees a slot; the WebSocket opens only
after that reply, as the fourth stranger at worst. Whether a browser
retries a dropped extra connection is unresolved (facts: Unresolved);
the poll fallback (page step 5) covers a socket that was dropped. The
three paths get
their own rate window, `PAGE_PER_MINUTE = 120` per listener (a reload
is three requests), beside `PAIR_PER_MINUTE` and `INFO_PER_MINUTE`
(`remote.rs:584-587`, `Limits::limited` at 631), answered 429 past it
without reaching the frame. §4.1's bounds list gains the line.

`RemoteReply` (`remote.rs:389-393`) gains an additive field
`headers: Vec<(String, String)>`, written after `Content-Length` by
`respond` (`remote.rs:530-554`), which takes the reply rather than
`(status, body)`; `Content-Type` is a header in that list, defaulting
to `application/json` when absent, so the 28 literals in `src/app.rs`
and the two in the adapter change only by `headers: Vec::new()`.
`Set-Cookie` travels the same way. `Served::Now`
(`app.rs:277-280`) is how the static replies go out; bodies are
`include_str!` constants in a new `src/web.rs` (`pub const INDEX_HTML`,
`APP_CSS`, `APP_JS`), the pattern of `include_bytes!` for the icon and
fonts (`src/main.rs:167`, `src/ui/theme.rs:203-211`). Nothing in
`Cargo.toml` changes.

### What the page does (`web/app.js`)

In order, on load:

1. Read `location.hash`; if it names `code`, `key` and `id`, call
   `history.replaceState(null, "", "/")` before any request. The
   fragment never reaches the host (facts: Settled); a copy in the
   browser's history is a spent single-use code once pairing succeeds
   or the window closes (facts: Unresolved on `replaceState` and
   history; `docs/remote-client.md:311` on single use), and the plan
   says so in the spec's new paragraph rather than promising more.
2. `GET /api/v1/info`. If the stored `server_id` matches and
   `spki_sha256` differs: show "This host's identity changed. Pair
   again." and send nothing else. If the URL's `key` differs from
   `/info`'s: show "This is not the host whose code was scanned." and
   do not post the code.
3. With a code and a matching key: `POST /api/v1/pair` with
   `{code, spki_sha256, device_name, transport: "cookie"}`;
   `device_name` from `navigator.userAgent` as "Safari on iPhone",
   "Chrome on Android", "Safari on Mac"; an iPad presents as a Mac
   (facts: Settled), so `Macintosh` with `navigator.maxTouchPoints > 1`
   is "Safari on iPad". Store `server_id` and `spki_sha256`. A 403
   `wrong_code` or `pairing_closed` is shown by its message.
4. `GET /api/v1/waiting`, the first authenticated request. A 401 here
   with no pairing ever stored shows "Not paired. Open Remote devices on the Mac and scan the code."; a 401
   with a stored pairing shows "This device was revoked. Pair again."
   (the cookie is already cleared by the host). Then step 5, then
   `/projects` and `/projects/{id}/sessions` per project.
5. Open `wss://<host>/api/v1/events` after the `/waiting` reply of
   step 4 and before the `projects` and `sessions` snapshots (so one
   connection is already in the paired pool, Decision 4), buffer
   events, apply by revision (§4.4 "Subscribe first, then snapshot":
   the collections are the snapshot, `/waiting` is refetched if an
   event's revision passes its own),
   send `watch {kind, id}` for the open session's `conversation` or
   `screen` key. On open failure or close: poll `waiting`, `projects`
   and the open session's content every 5 s, refetching a collection
   whose `revision` moved, and retry the socket every 30 s. Header says
   "live" or "polling".
6. Render: sessions grouped by project, waiting first within and across
   groups, each with kind, the card word, the waiting reason; tapping
   one opens the view: `GET sessions/{id}/last-response` for a Claude
   Code session (`text`, as `client/Fixtures/last_response.json`),
   `GET sessions/{id}/screen` for anything else (`text`), refreshed on
   `changed {conversation|screen}` or on the poll. A reply box posts
   `POST sessions/{id}/send {op, text}` with a fresh UUID `op`, shown
   only when `/pair`'s or the stored `scope` is `control`; the device's
   scope and the host's `server_name` are in the header. Scope is not
   on `/info`, so a scope lowered in the dialog reaches the page as the
   403 its next `send` answers, which it shows as "This device is
   read-only now."

No framework, no build step: three files, plain DOM.

## The UI

`src/ui/remote.rs` `pairing` (141-228): the payload at 160-165 becomes
`core::remote::pairing_invite(..)`; the sentence becomes "Scan with a
phone's camera, or paste the invite into a client. The browser will
warn about the certificate: on your own network, accepting it is the
trust step, and it may ask again. Grants … scope. Open for five
minutes or three wrong codes." Without an address: no QR, no Copy
invite, "No network address: pair by `switchboard-remote pair
<host:port> <code> <key>`". `devices` (230-276) is unchanged: a browser
row reads "Safari on iPhone", read or control, paired and seen ago,
Revoke. No new script line: `pairing <code>` and `remote-devices`
(`src/script.rs:729-736`) already reach the window.

## The tests

Core (`src/core/tests.rs`): `pairing_invite` builds the URL with an
IPv4 and a bracketed IPv6 address, uppercases the code, carries the
full key and the id in the fragment, and never the token.
`token_cookie_name` returns `switchboard_token_` plus the identity's
`server_id`, and differs for two identities.

`tests/remote.rs` (the `http` helper at 245-297 gains a `cookie`
argument and returns the response headers; `call` at 299 and `get`
follow; `subscribe_at` at 1960 gains a cookie variant):

- `the_page_is_served_unpaired_with_its_headers`: `/`, `/app.css`,
  `/app.js` answer 200 with the content types, CSP, `no-store`,
  `nosniff`; `/index.html`, `/app.js/`, `/other` and `POST /` are 404
  `not_found`; the `/other` line in
  `info_is_served_to_anyone_and_every_other_read_is_refused_unpaired`
  (`tests/remote.rs:464`, the test at 426) stays.
- `the_page_references_only_what_the_host_serves`: every `src` and
  `href` in `web::INDEX_HTML` is one of the three paths; no `<script>`
  has a body; no `style=`, no `http://` or `https://` in any of the
  three files; `web::APP_JS` never contains `Authorization` (the token
  never enters script) and never writes `document.cookie`.
- `a_browser_pairs_from_the_invite_and_reads_through_its_cookie`: open
  the window, build the invite with `pairing_invite`, parse its
  fragment, `GET /info`, compare `key`, `POST /pair` with `transport:
  cookie` and `device_name: "Safari on iPhone"`; the body has no
  `token`, the `Set-Cookie` header has the name, `HttpOnly`, `Secure`,
  `SameSite=Strict`, `Path=/api/v1/`, and the name is exactly
  `token_cookie_name(identity)` (`switchboard_token_<server_id>`);
  `GET /waiting` with the cookie is
  200 and `DeviceSeen` moved `last_seen`; the events upgrade with the
  cookie gets `hello`; the device is listed by name; a bearer in the
  same request outranks the cookie (a known bearer and an unknown
  cookie: 200). A request carrying another host's cookie name
  (`switchboard_token_<other uuid>=<this host's valid token>`) is 401
  with no `Set-Cookie`, and the same request with both names is 200
  through the right one. Then `RevokeDevice`: the next `GET` is 401
  with a clearing `Set-Cookie` of the same name and the subscriber's
  socket closes (the pattern of the existing revocation test). The same test asserts the existing
  `pair` helper's native reply has the token in the body and no
  `Set-Cookie`.
- `a_page_load_fits_the_stranger_pool`: three connections each `GET`
  one page file and are held open (keep-alive, not closed); a fourth
  does `GET /waiting` with the cookie (200, moved to the paired pool)
  and a fifth then upgrades to the events socket with the cookie and
  receives `hello`, while the three are still held.
- `the_page_window_is_rate_limited`: the 121st static `GET` in a minute
  is 429, the shape of `pair_is_rate_limited_per_listener` (1406).
- Fixtures: `write_fixtures` (1587) adds `client/Fixtures/invite.txt`
  with the URL form; `the_client_fixtures_decode_on_both_sides` (1829)
  asserts the fixture equals `pairing_invite(..)` over the fixture
  identity, and the Swift side gains `testInviteParsesTheURL` reading
  that fixture beside the kept inline JSON test
  (`FixturesTests.swift:158-166`); the Swift test is the parser's test.
  No view fixture changes.

`tests/ui.rs` `remote_devices_dialog_opens_pairing_and_revokes_a_device`
(7424-7489): the clipboard assertion parses the URL: starts with
`https://`, the fragment carries `code=<code>` and the identity's full
`spki_sha256` as `key`.

Not driven by a test, done by hand before the commit and recorded in
the PR: the cookie on a bypassed certificate (the Decision 2 check),
and the acceptance steps from the issue on one phone.

## Docs, in the same commit

- `docs/remote-client.md`: §4.1 bounds gain the page window; §4.3 table
  gains the three rows with scope "none" and the "everything else is
  refused" sentence names them; the `last_message` sentence at 207
  becomes "the excerpt is not yet on the wire; it will be a redacted
  field of the session view, masked as the redaction list above says",
  so 201 and 207 agree; §5 line 306 becomes "before
  anything but `/info`, `/pair` and the page", line 309 the URL, line
  312 the cookie clause, and a new paragraph "Browser trust" with
  Decision 1 and the fragment note; §13 Authentication row gains "a
  browser pairs from the invite URL and holds its token in an HttpOnly
  cookie".
- `docs/design.md` remote status (3107-3584): a bullet "**The web page
  (2026-10-22).**" after the typed-code bullet, and the known-gaps
  bullet gains the excerpt not on the wire, the socket-on-iOS and the
  one-address items.
- `README.md`: Layout rows 383, 401, 423, 437, 446 amended; new rows
  for `src/web.rs` and `web/`; the dev-aid paragraph at 267-279 says
  the invite is a URL a phone opens and that `switchboard-remote pair
  "$(pbpaste)"` still takes it.
- `CHANGELOG.md` Unreleased: "The remote port serves a browser page:
  scan the pairing QR with a phone to see the sessions, the waiting
  ones first, read a session's last response or screen and reply."

## Follow-ups for the owner (not in this commit)

The excerpt on the wire, the socket on iOS and the one address are in
the design.md known-gaps bullet above. Beyond those:

- Reissuing the leaf with an IP SAN and 825-day validity from the same
  key, or a CA-plus-leaf identity, so a device can trust the host
  instead of bypassing: a change to the identity model in §5.
- `Sec-WebSocket-Protocol` as the token transport, only if the cookie
  fails the Decision 2 check on the owner's phone.
- Dispatch tickets and the full conversation on the page, as the issue
  defers.

## Commit

`Remote port serves a browser page paired from the invite URL`

## Contract additions (copy into the commit message)

- `RemoteRequest::cookies: Vec<(String, String)>` (name and value of
  each `switchboard_token_*` cookie), `src/adapters/remote.rs`.
- `core::remote::token_cookie_name(&Identity) -> String`
  (`switchboard_token_<server_id>`), `src/core/remote.rs`.
- `RemoteReply::headers: Vec<(String, String)>`, `src/adapters/remote.rs`.
- `core::remote::pairing_invite(&Identity, &str, &str) -> String`,
  `src/core/remote.rs`.
- `web::{INDEX_HTML, APP_CSS, APP_JS}`, new module `src/web.rs`
  embedding `web/index.html`, `web/app.css`, `web/app.js`.
- `POST /api/v1/pair` body field `transport: "cookie"` (the cookie is
  named `switchboard_token_<server_id>`); `GET /`,
  `GET /app.css`, `GET /app.js` on the remote port.
- `PairingInvite.parse` accepts the URL form, `client/Sources/SwitchboardProtocol/Info.swift`.
