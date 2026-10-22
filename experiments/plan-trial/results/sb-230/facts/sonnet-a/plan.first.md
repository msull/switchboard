# Plan: a web control surface on the remote port (#230)

The host serves a small page from the remote port's own listener. A phone
opens it from the pairing window's QR, pairs, lists the sessions with the
waiting ones first, reads a session's last response or screen, and sends a
reply. The page speaks the protocol the headset speaks. One commit; the app
runs at every step.

Line numbers are from this worktree at 7fe8949 (Release 0.4.1). "Facts"
means `facts.md` for this ticket; a decision that rests on its Settled
section cites the line, and one that rests on Unresolved or For the owner
says so where it is made and names the fallback.

## Where this departs from the issue

1. **The pairing values go in the URL fragment, not the query.** The issue
   writes `/?code=…&key=…&id=…`. `read_request` (`src/adapters/remote.rs:444`)
   splits the query off and hands it to the frame, and the browser puts the
   whole request line in its history and, by the issue's own account, sends
   the code to the host before `replaceState` runs. A fragment is never sent,
   never in a `Referer`, and not fetched by a link previewer. Whether the iOS
   camera or a previewer fetches the URL is Unresolved in Facts; the fragment
   is right whichever way that goes. The page and both native parsers accept
   either form, so the issue's URL still works. The shape is
   `https://<ip>:<port>/#code=<code>&key=<spki_sha256>&id=<server_id>`.
2. **"Any path outside `/api/v1/` is refused" stays true for everything but
   three files.** `serve_remote` refuses at `src/app.rs:3023`. The plan does
   not touch that line: the three page paths are answered on the connection
   thread before a request reaches the frame (Signals, below), so the frame's
   allowlist is unchanged and `/other` still answers the JSON 404 that
   `tests/remote.rs:464` asserts.
3. **The card's one-line excerpt is not on the wire, and the spec line that
   says it is does not match the tree.** `docs/remote-client.md` §4.3 (the
   `session.last_response` paragraph) says the excerpt "is `last_message` from
   the `Stop` hook on the record, a field of the session's metadata, and moves
   with `changed {session}`". `last_message` is carried by the hook event
   (`src/ports/events.rs:24`, `src/adapters/hooks.rs:71`) and nothing in
   `src/core` keeps it; `SessionView` (`control/src/lib.rs:637`) has no such
   field. The card's text is `agent_excerpt` (`src/ui/cards.rs:558`) over a
   parsed conversation, and a shell's is a `UiState` caption. This is also
   For the owner in Facts ("Excerpt on the wire"), unanswered. **Plan:** no
   new field. The page shows the state word and `waiting_reason` for every
   row, and for a Claude Code session the first line of
   `GET /sessions/{id}/last-response` (`text`, `src/content.rs:827`), fetched
   lazily for rows on screen, two at a time, because a read goes through the
   one content worker and enters the watch set (cap 128, `WATCH_CAP`
   `src/content.rs:42`). A shell, command or service row has state only. If
   the owner answers yes to an additive `SessionView` excerpt, the
   implementer moves `agent_excerpt` to the core, adds
   `#[serde(default)] excerpt: Option<String>` to `SessionView`, and the page
   reads it instead of fetching; nothing else in this plan changes. The
   commit also corrects the §4.3 sentence.
4. **A page needs to learn its scope later, and there is no way to.** Facts
   (Unresolved, last line) allows remembering the `/pair` reply's scope. That
   goes stale in the direction the owner most wants (raising a phone to
   `control` in the dialog, which §5 says takes effect "on the device's next
   request"). **Plan:** one additive authenticated read, `GET /api/v1/me` →
   `{device_id, name, scope}`. It is also the page's boot probe: a 401 from it
   is how the page learns it was revoked.
5. **"The page says so on its next request" cannot tell revoked from
   removed.** `authenticate` (`src/app.rs:3127`) answers the same 401 for a
   revoked token and an unknown one, and a rotated key also deletes the
   devices (§5). The page distinguishes by what it holds: a stored token
   getting 401 with an unchanged key says "this device was revoked or removed
   on the host; pair again"; a changed key says "identity changed, pair
   again" and sends nothing, token included (§5).
6. **§5 says "Require pairing before anything but `/info` and `/pair`" and
   the pairing hover says "only paired devices get past /info"**
   (`src/ui/remote.rs:~112`). Both change to name the three page files.
7. **The pairing window hands a phone an address that does not answer when
   the port is loopback-only.** Facts (Settled, `cx.remote_port` line) says the
   plan should close this. The QR's address comes from `local_address()` even
   when `identity.listen.lan` is false. The plan puts `127.0.0.1` in the URL
   in that case and the dialog says why a phone cannot use it (UI).

## The record

No new record and no `SCHEMA_VERSION` change (`src/core/model.rs:12`, 16
today; `VIEWS_SCHEMA_VERSION` `:612`, 7 today). A browser is a `PairedDevice`
(`src/ports/remote.rs:96`) like any other, named by the page in
`device_name`, which `remote_pair` already reads (`src/app.rs:3606`). The
page derives the name from `navigator.userAgent` (`Safari on iPhone`,
`Chrome on Android`, `Safari on Mac` for an iPad, which reports a Mac:
Unresolved in Facts, the For-the-owner question on naming stands). It shows
the name in an editable field before the code is sent, so the owner can type
"Sully's iPad". No `User-Agent` is kept by the host and `PairedDevice` gains
nothing.

**The certificate does change, and `identity.json` with it.** This is the
one place the browser surface touches stored state.

- *Why.* Facts (Settled, Apple row) says Apple wants the name in a SAN,
  `serverAuth`, and 825 days or fewer, and that the current certificate fails
  all three (`make_identity` `src/adapters/remote.rs:95–108` builds
  `CertificateParams::new(Vec::new())` with a CN only; not_after year 4096).
  Whether Safari then still offers "Visit this website" is Unresolved and
  needs a device.
- *Decision 1 (click-through or download), which is For the owner.* The plan
  builds **click-through as the trust step**, said in the dialog, with the
  certificate made acceptable to the browsers that check. It does not build a
  download route. Why: a download needs an iOS profile-install flow that
  Facts lists as Unresolved and must be tried on a device first, and it adds
  a route outside the three-file set. What click-through buys: the browser
  accepts this certificate for this address on this device, and the `key` in
  the URL lets the genuine page refuse to send the code to a host whose
  `/info` differs. What it does not buy: a page served by an impostor is not a
  genuine page, so the page's key comparison protects only against a host
  that differs from the one the QR named *and says so*; the dialog states
  that the trust step is the owner scanning the QR from their own screen.
  **Fallback if the owner wants a download:** add `GET /cert.der`
  (`application/x-x509-ca-cert`, the DER from `certificate_der`) to the
  static set below, and put the install steps in the dialog; the SAN reissue
  in this section still applies, since iOS wants the same fields.
- *What is built.* `make_identity` and a new `reissue_certificate(identity,
  key_pem, addresses, now)` share one parameter builder: SAN of each address
  given plus `127.0.0.1`, `::1` and `localhost`; `serverAuth` EKU;
  not_before `now`, not_after `now` + 800 days (`time`, already in
  `Cargo.lock` through `rcgen`, becomes a direct dependency; `now` is a
  parameter so the function is deterministic). The reissue reuses the same
  key from the Keychain, so `spki_sha256` is unchanged and every pairing
  stays valid: §5 pins the key and the `Pin` verifier
  (`src/adapters/remote.rs:183–197`) looks at nothing else.
- *`Identity` (`src/ports/remote.rs:25`) gains* `#[serde(default)] pub sans:
  Vec<String>` (the addresses the certificate names) and `#[serde(default)]
  pub certificate_issued: Option<SystemTime>`. `IDENTITY_SCHEMA_VERSION`
  (`:17`, 1 today) becomes the value in that file at implementation time,
  plus one: a ticket landing in parallel may take the next number first. An
  older file loads with both empty. `RemoteFiles::load` is plain serde, so
  no migration step is needed.
- *When it runs.* In `load_identity` (`src/app.rs:2781`), after the key check
  succeeds and not when `read_only()`: reissue when `sans` is empty, when
  `local_address()` is set and not in `sans`, or when `certificate_issued` is
  over 700 days old; then save the identity as `make_identity` does. A
  reissue never touches `devices.json`. A LAN address that changes while the
  app runs is not caught until the next launch; the dialog says so (UI) via a
  new `Identity::covers(ip) -> bool`.

## The signals

No new core signal, `AppAction` or event. The page consumes the existing
`/api/v1/events` socket (`hello`, `changed`, `removed`, `resync_required`,
§4.4) and sends `watch`/`unwatch` for the open session's `screen` or
`conversation` key. What is new is on the wire into the host:

**Static paths, answered on the connection thread** (`connection`,
`src/adapters/remote.rs:770`, after the rate check and before
`shared.requests.try_send`). They are constants and decide nothing, which
CLAUDE.md puts in the adapter; the cookie route would have needed frame state
(`Set-Cookie` after `/pair`), the route chosen below does not.

| Path | Content-Type | Source |
|---|---|---|
| `GET /` | `text/html; charset=utf-8` | `web/index.html` |
| `GET /app.js` | `text/javascript; charset=utf-8` | `web/app.js` |
| `GET /app.css` | `text/css; charset=utf-8` | `web/app.css` |

The query and fragment do not matter (`read_request` already splits the
query; the fragment never arrives). Any other method on these paths is
`405` JSON; every other path keeps today's route to the frame and its JSON
404. A new `src/adapters/web.rs` holds the table (`include_str!` of `web/`,
the first text assets in the tree; `include_bytes!` is used only at
`src/main.rs:167` and `src/ui/theme.rs:203`) and the header set; `respond`
(`:530`) gains a sibling `respond_page(w, content_type, body)` that writes:

- `Content-Security-Policy: default-src 'none'; script-src 'self'; style-src
  'self'; connect-src 'self' wss:; img-src data:; base-uri 'none'; form-action
  'none'; frame-ancestors 'none'`. Facts (Settled, CSP line) says `'self'` does
  not cover `wss:` in all browsers and the plan names it. It names the scheme,
  not the origin, because the address is not known to the binary and the
  `Host` header is not kept; the cost is that script that was already running
  could open a `wss:` connection elsewhere, which `script-src 'self'` plus
  text-only rendering (Tests) is there to prevent.
- `Cache-Control: no-store` (no caching across versions),
  `X-Content-Type-Options: nosniff`, `Referrer-Policy: no-referrer`. Facts
  (Unresolved, last-but-one line) says these are standard practice with no
  primary source fetched; adopted as such.
- `Connection: keep-alive` and `Content-Length`, as `respond`.

**Pool and rate.** Facts (Settled, `Limits::limited` line): a page load
counts against no rate window and only the 4-slot unauthenticated pool. The
plan adds a window, `PAGE_PER_MINUTE = 120` per listener (about 40 loads
with `no-store`), in `Limits` (`:621`) and `limited` (`:631`) for the three
paths, answering the existing 429. The unauthenticated pool is unchanged at
4 (`MAX_UNAUTHENTICATED`, `:567`): a connection that has only fetched files
is "unauthenticated" until a paired request, then moves to the paired pool
like any other. Whether a phone's parallel page load fits in 4 within the
30 s `IDLE_TIMEOUT` is Unresolved in Facts and needs a phone. **If it starves**
the implementer does not raise the pool; they set `Connection: close` on
the three file replies, which frees the slot as soon as the file is sent.
The page is three requests plus `/info`, `/pair` and `/me` and then one
WebSocket.

**Without a code and without a token**, the page loads and shows: the host
cannot be known, "Open the pairing window on the Mac (Settings, Remote
devices, Allow pairing for 5 minutes) and scan its code with this device's
camera." No fields to type a code into.

**Decision 2: the token reaches the WebSocket as a subprotocol value.** For
the owner in Facts, so the choice and its fallback:

- *Chosen.* `new WebSocket(url, ["switchboard.v1", "bearer." + token])`. In
  `read_request`, for an upgrade request with no `Authorization` header, a
  `Sec-WebSocket-Protocol` list containing `switchboard.v1` and an entry
  `bearer.<token>` (token characters `[A-Za-z0-9_-]`, at most 64) sets
  `bearer`; `authenticate` is unchanged. The 101 (`respond_upgrade` `:519`)
  echoes `switchboard.v1` only, never the token. Facts (Settled, RFC 6455
  block) requires exactly this: the server must echo one offered value or the
  browser fails the connection; base64url tokens are valid token characters.
  `RemoteRequest` gains `subprotocol: Option<String>` (the echo); the only
  construction site is `read_request` at `:506` (Facts, Settled, `RemoteReply`
  line), so no literal elsewhere breaks.
- *Why not the cookie.* A cookie is ambient: the browser attaches it to any
  request to that origin, so authentication stops being a header a
  cross-site page cannot send, and Facts (Settled, cookie block) says
  `SameSite` ignores the port, so another service on the same IP is
  same-site. That needs `Set-Cookie` on `RemoteReply`, a `Cookie` field, and
  an `Origin` check on every `POST` and the upgrade. The subprotocol keeps
  today's property that nothing authenticates unless the page's own script
  puts the token there. The Origin check on the upgrade stays optional per
  Facts and is not added. Whether a `Secure` cookie survives a click-through
  certificate is Unresolved in Facts; the chosen route does not depend on it.
- *Where the token lives.* `localStorage` under the page's origin, readable by
  script on that origin. The mitigation is the CSP above and a page that never
  assigns `innerHTML`.
- *Fallback if the owner prefers the cookie.* `RemoteReply` gains
  `set_cookie: Option<String>` written by `respond`; `read_request` keeps
  `cookie`; `authenticate` reads it when there is no bearer; `remote_pair`
  sets `HttpOnly; Secure; SameSite=Strict; Path=/api/v1/` and the page drops
  `localStorage` for the token; `RemoteRequest` keeps `origin` and the frame
  refuses a `POST` or upgrade whose `Origin` is not the request's own host.
  The `/me` probe and everything else stay.

**The page's pairing steps** (`web/app.js`), in this order and nothing
before step 1:

1. Read `location.hash` (and `location.search`), then `history.replaceState(
   null, "", location.pathname)`. Whether that keeps the code out of Safari's
   history and recents is Unresolved in Facts and needs a phone; the fragment
   choice (departure 1) is what makes the answer matter less.
2. `GET /api/v1/info`; refuse and say so unless `body.spki_sha256` equals
   `key` (case-insensitive) and `body.server_id` equals `id`. The page cannot
   see the certificate, only what `/info` claims (the issue's own caveat).
3. `POST /api/v1/pair {code, spki_sha256: key, device_name}`. A `403
   wrong_key`/`wrong_code`/`pairing_closed` is shown as that.
4. Store `{token, scope, server_id, key}` in `localStorage`, then `GET /me`
   for the live scope and start the session list.

Later visits: `/info` first; a key differing from the stored one →
"identity changed, pair again" and nothing is sent; otherwise `/me`; 401 →
"revoked or removed"; a network error → "can't reach the host" with a
retry. The events socket reconnects with a backoff of 1 s doubling to 30 s,
reruns `/me` (this is how a revoke, which closes the socket from the host
side in `dispatch_inner` `src/app.rs:486`, shows), and refetches on a new
`epoch` or `resync_required`, as §4.4 and the Swift client do. The page
orders rows waiting first (`/waiting`) within a project (`/projects`,
`/projects/{id}/sessions`), shows `card`, `waiting_reason` and `kind`, and
for the open session reads `/last-response` (Claude Code) or
`/screen?lines=40` (anything else), watched by `watch` on the same key. The
reply box exists only when `/me` says `control`; it posts `{op, text}` to
`/sessions/{id}/send` with `op` from `crypto.getRandomValues` (op is the
idempotency key, `src/app.rs:3159`), and a `403 scope` hides the box again.

## The actions and effects

None in `AppAction`/`Effect`. Frame-side additions in `src/app.rs`:

- `serve_remote` (`:3017`): `("GET", ["me"])` after `authenticate`, answering
  `envelope(200, {device_id, name, scope}, true)`. It is on the allowlist
  by being matched there, not by a change to the prefix check at `:3023`.
- `load_identity` (`:2781`): the reissue step above.
- `bind_remote` (`:2936`) is unchanged; it reads the identity the core holds.

Adapter-side: `src/adapters/web.rs` (new), `connection`/`read_request`/
`respond_upgrade`/`Limits` in `src/adapters/remote.rs`, `reissue_certificate`.
Core-side, one pure function: `invite_url(ip: IpAddr, port: u16, server_id:
Uuid, spki_sha256: &str, code: &str) -> String` in `src/core/remote.rs`,
formatting `SocketAddr::new(ip, port)` (so an IPv6 address is bracketed) as
`https://{addr}/#code=…&key=…&id=…`. It lives in the core because the
dialog and the fixtures both need the same string, and CLAUDE.md says logic
found under `ui/` moves to the core.

## The UI

Only `src/ui/remote.rs`, and the page itself.

- `pairing` (`:141`): `payload` becomes `invite_url(...)`; the QR
  (`qr_texture` `:280`) encodes it. The text beside the code shows the
  address and short key as now. "Copy invite" copies the URL and its hover
  says "the QR code's address, to paste into a client". The scope line says
  "Scan with the camera, or paste the address into a client."
- When `!identity.listen.lan` the address is `127.0.0.1` and a line under
  the code says "Listening on this Mac only; a phone cannot reach it. Turn
  on Allow devices on the local network, above."
- When `identity.listen.lan` and `remote::local_address()` is set but
  `!identity.covers(ip)` a line says "This Mac's address changed since
  launch. Restart Switchboard so the certificate names it."
- A sentence in the dialog, near the pairing window, for the trust step:
  "A browser will warn about this Mac's certificate the first time. Open the
  address only from the code on this screen, accept the warning for this
  address, and the page checks the key against the one shown here before it
  sends the code." The hover on the LAN checkbox (`:~112`) names `/info`,
  `/pair` and the page files.
- The devices table (`:229`) is unchanged: a browser is a row with a name,
  a scope combo and Revoke.

The page, `web/index.html`, `web/app.js`, `web/app.css`: no framework, no
build step, no external resource, no inline script or style attribute. Text
from the host is set with `textContent` only. It is a single column that
works at phone width (16 px side gutter, no horizontal scroll) with
`prefers-color-scheme` colors defined once on `:root`. Views: pairing/status
messages, the session list, the session view, the header (host name from
`/info`, device scope from `/me`).

## The tests

No JavaScript runs in this tree's tests, and the plan adds no JS test tooling
(the issue: no dependency for the page). The page's JavaScript is therefore
covered by static checks plus the end-to-end flow driven from the host's
side, and the owner's phone run is the only check of the page's behavior.
The README and design status say so.

`tests/remote.rs` (a new helper beside `http()` `:245` that returns response
headers, since `http()` discards them):

- `the_page_is_three_files_with_their_headers`: `GET /`, `/app.js`,
  `/app.css` with no token answer 200, the content types in the table, and
  exactly the CSP string exported from `adapters::web`, `Cache-Control:
  no-store`, `nosniff`, `no-referrer`; `GET /?code=X` answers the same page;
  `POST /` is 405; `/other`, `/index.html` and `/app.js.map` stay the JSON
  404 (the assertion at `:464` is kept). A page request makes no
  `RemoteIncoming` (the frame is never pumped for it).
- `the_page_references_only_what_the_host_serves`: every `src`/`href` in
  `index.html` is one of the three paths; no `http:`/`https:` URL in any of
  the three files; no `<script>` without `src`, no `style=` attribute, no
  `innerHTML`, `outerHTML`, `document.write` or `eval(` in `app.js`; every
  `/api/v1/<segment>` literal in `app.js` has a first segment in `{info,
  pair, me, projects, sessions, waiting, events}`.
- `the_page_is_rate_limited_per_listener`: `PAGE_PER_MINUTE` + 1 requests →
  429, shaped like `pair_is_rate_limited_per_listener` (`:1406`).
- `a_browser_pairs_and_follows_the_host_end_to_end`: the issue's acceptance
  from the host's side with the host's own client, as the CLI does. Build the
  URL with `invite_url`, split its fragment, `GET /info` and compare, `POST
  /pair` with `device_name: "Safari on iPhone"`, `GET /me` → the scope from
  the window; open the socket with the subprotocol list and no
  `Authorization`, assert the 101 echoes `switchboard.v1` and not the token,
  receive `hello`; change the session to waiting through the control socket
  and see its `changed`; raise the device to `control` with
  `SetDeviceScope` and see `/me` say so without re-pairing; `POST /send`
  and see it typed once; revoke, assert `/me` is 401 and the socket closes
  (the pattern of `revoke_closes_the_devices_socket` `:2892`).
- `a_subprotocol_token_is_checked_like_a_bearer`: a wrong token → 401 and no
  101; an `Authorization` header wins over the subprotocol; a subprotocol on
  a non-upgrade request authenticates nothing; a token over 64 characters or
  with a character outside `[A-Za-z0-9_-]` is ignored.
- `me_is_read_scope_and_unauthenticated_is_refused`.
- The `/other` test (`:426`) and the allowlist test (`:682`) keep passing
  unchanged; the fixtures test (`:1829`) gains `me.json` and
  `pairing_url.txt` in `write_fixtures` (`:1587`), the second written from
  `invite_url` with fixed inputs.

`src/adapters/remote.rs` unit tests: a fresh certificate and a reissued one
are accepted by `rustls`'s `WebPkiServerVerifier` (the certificate as its own
root) for `ServerName::IpAddress` of a SAN address and for `localhost`, are
refused for an address not in the SAN and 801 days after issue, and keep the
same `spki_sha256`. If webpki will not take a leaf as its own root, assert on
the `CertificateParams` the shared builder returns instead. A reissued
identity round-trips through `RemoteFiles`; an older file with neither new
field loads.

`src/core/tests.rs`: `invite_url` for an IPv4 and an IPv6 address; the three
`Identity { .. }` literals (`src/core/tests.rs:11769`, `tests/ui.rs:7436`,
`src/adapters/remote.rs:117`) gain the new fields. `Identity::covers`.

`tests/ui.rs` (the test around `:7474`): "Copy invite" now yields a URL;
parse its fragment and assert `code`, `key` equal to the identity's and
`id`; a loopback identity shows the "Listening on this Mac only" line and
puts `127.0.0.1` in the URL; an identity whose `sans` lack the local address
shows the restart line.

Swift (`client/`): `PairingInvite.parse` (`Info.swift:114`) accepts an
`https://host:port/` URL with `code`, `key`, `id` in the fragment or the
query — authority → `addresses: [authority]`, `key` → `spkiSha256`, `id` →
`serverID` — and the JSON as before, so no call site changes
(`main.swift:107`, `BrowserView.swift:106/883`). Tests in
`FixturesTests.swift` read `pairing_url.txt` and a JSON invite, and cover
the query form. Facts and the notes agree these tests are in neither the
hook nor CI; the implementer runs `swift test` in `client/` if the toolchain
is there and says in the commit body if it was not. The Swift change is not
verified by anything in CI.

Not verifiable in this tree, left for the owner's devices: whether each
browser lets the owner click through this certificate and then trusts it for
`fetch` and `wss://`; the iOS camera opening the URL; `replaceState` and
Safari's history; the phone's parallel connections against the pool of 4;
the socket after the phone sleeps; the acceptance run (scan, accept the
warning, see the sessions, a waiting session rises, a reply arrives, revoke
says so). The design status section lists each as "Not run".

## Docs and files the commit touches

`docs/remote-client.md`: §4.1 bounds (the page window), §4.3 table (`GET /`,
`/app.js`, `/app.css`, `GET /api/v1/me`) and the corrected `last_response`
sentence, §5 (QR is a URL; the pairing sentence; the token sentence names the
subprotocol carrier for the events upgrade; the browser trust paragraph; the
"Require pairing before…" line), and a subsection after 4.7 for the page.
`docs/design.md`: a "Web page" bullet at the end of "Remote client
(2026-10-12)" (before `## Versioning and updates`, line 3585) with what is
built, the unrun checks, and the three gaps (a changed LAN address needs a
relaunch; the excerpt for shells; the parallel-connection starvation).
`README.md`: Layout rows for `web/`, `src/adapters/web.rs`, the changed
descriptions of `src/ui/remote.rs`, `src/adapters/remote.rs` and
`tests/remote.rs`; the dev-aid paragraph at `:275` (the invite is now a
URL: `switchboard-remote pair "$(pbpaste)"` still works, and a desktop
browser can open it). `CHANGELOG.md`, under Unreleased: "A browser on the
local network can pair from the remote devices dialog's QR code, list
sessions with the waiting ones first, read a reply and send one."

## Commit

`Serve a web control surface from the remote port, paired from a URL in the QR code`

### Contract additions (for the commit message)

- `GET /`, `GET /app.js`, `GET /app.css`: static, unauthenticated, answered
  by the adapter; headers as above.
- `GET /api/v1/me` → `{device_id, name, scope}` (read scope).
- WebSocket upgrade accepts `Sec-WebSocket-Protocol: switchboard.v1,
  bearer.<token>` when there is no `Authorization` header, and echoes
  `switchboard.v1`.
- Pairing invite is a URL, `https://<addr>/#code=&key=&id=` (the query form
  also accepted); `PairingInvite.parse` takes it or the JSON.
- `Identity`: `sans: Vec<String>`, `certificate_issued: Option<SystemTime>`
  (serde default); `IDENTITY_SCHEMA_VERSION` = the value in
  `src/ports/remote.rs` at implementation time, plus one. The certificate is
  reissued over the same key with a SAN, `serverAuth` and 800 days; the
  `spki_sha256` and every pairing are unchanged.
- `RemoteRequest.subprotocol: Option<String>` (adapter-local).
- `core::remote::invite_url`, `Identity::covers`, `adapters::web`,
  `adapters::remote::reissue_certificate`, `PAGE_PER_MINUTE`.
- No `model.rs` or `action.rs` type changes; `SCHEMA_VERSION` and
  `VIEWS_SCHEMA_VERSION` untouched.
