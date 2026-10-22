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
   the code to the host before `replaceState` runs. A fragment is not part of
   the request target, so a client does not send it (RFC 3986 §3.5 defines it
   as resolved client-side; RFC 9110 §7.1 builds the request target from the
   path and query), and a `Referer` carries no fragment (RFC 9110 §10.1.3).
   Those three citations are the whole basis for "the host never sees it"; I
   have not fetched them, and Facts has no line for them. The previewer half
   stays **Unresolved** (Facts, the iOS-camera line): a previewer or the
   camera might read the URL itself, fragment included, before the owner taps,
   and nothing in this plan stops that. The page and both native parsers
   accept either form, so the issue's URL still works (an old-form URL does
   send the code in the request line). The shape is
   `https://<ip>:<port>/#code=<code>&key=<spki_sha256>&id=<server_id>`.
2. **The card's one-line excerpt is not on the wire, and the spec line that
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
   `src/content.rs:42`). A shell, command or service row has state only.
   Fallback, built only if the owner asks: an additive `SessionView` excerpt.
   The commit also corrects the §4.3 sentence.
3. **A page needs to learn its scope later, and there is no way to.** Facts
   (Unresolved, last line) allows remembering the `/pair` reply's scope. That
   goes stale in the direction the owner most wants (raising a phone to
   `control` in the dialog, which §5 says takes effect "on the device's next
   request"). **Plan:** one additive authenticated read, `GET /api/v1/me` →
   `{device_id, name, scope}`. It is also the page's boot probe.
4. **The pairing window hands a phone an address that does not answer when
   the port is loopback-only.** Facts (Settled, `cx.remote_port` line) says the
   plan should close this. The QR's address comes from `local_address()` even
   when `identity.listen.lan` is false. The plan puts `127.0.0.1` in the URL
   in that case and the dialog says why a phone cannot use it (UI).
5. **The QR becomes URL-only; the JSON form is dropped from the QR.** This is
   the "One URL or JSON too" question, For the owner in Facts and unanswered;
   the plan takes the "drop the JSON" branch for what the QR encodes and
   keeps parsers accepting both. The cost: a headset or `switchboard-remote`
   built before this change cannot read the new QR or the copied invite,
   because `PairingInvite.parse` (`Info.swift:114`) is JSON-only, and those
   apps do not self-update with the Mac app. They are not stranded: the
   headset pairs from the Bonjour record and a typed code, and the CLI has
   its three-argument form `host:port code spki` (`main.swift:107`).
   Fallback, built only if the owner asks: "Copy invite" keeps copying JSON.

## The record

No new record and no `SCHEMA_VERSION` change (`src/core/model.rs:12`, 16
today; `VIEWS_SCHEMA_VERSION` `:612`, 7 today). A browser is a `PairedDevice`
(`src/ports/remote.rs:96`) like any other, named by the page in
`device_name`, which `remote_pair` (`src/app.rs:3606`) already reads at
`:3650`. The page derives the name from `navigator.userAgent` (`Safari on
iPhone`, `Chrome on Android`, `Safari on Mac` for an iPad, which reports a
Mac: Unresolved in Facts, the For-the-owner question on naming stands). It
shows the name in an editable field before the code is sent, so the owner can
type "Sully's iPad". No `User-Agent` is kept by the host and `PairedDevice`
gains nothing.

**The certificate does change, and `identity.json` with it.**

- *Decision 1 (click-through or download), which is For the owner.* The plan
  builds **click-through as the trust step**, said in the dialog (UI), and
  makes the certificate acceptable to the browsers that check. No download
  route: it needs an iOS profile-install flow that Facts lists as Unresolved.
  Fallback, built only if the owner asks: `GET /cert.der`.
- *If click-through hard-fails even with the SAN.* Facts leaves two lines
  Unresolved (whether Safari offers "Visit this website"; whether `fetch` and
  `wss://` keep working after a click-through), and the plan builds for the
  answer being yes. If a browser refuses the certificate with no way through,
  or trusts it for the page but not for `wss://`, **the page is unusable on
  that browser, and the plan accepts that**: the native clients are
  unaffected, and the design status says which browsers were tried and
  failed. No plain `http:` listener: it would carry the pairing code and the
  bearer token in the clear on the local network, against §5. Facts line 9
  (a bare-IP URL needs an IP-type SAN to match) is **an inference**: Apple's
  page speaks only of DNS names. The IP SAN is harmless if it is wrong; the
  unit test below checks it against `rustls`'s verifier, which is not a
  browser, and only a device settles it.
- *What is built.* Facts (Settled, Apple row): the current certificate has no
  SAN, no `serverAuth` and a year-4096 expiry, against Apple's 825 days.
  `make_identity` (`src/adapters/remote.rs:95`) and a new
  `reissue_certificate(identity, key_pem, addresses, now)` share one parameter
  builder: SAN of each address given plus `127.0.0.1`, `::1` and `localhost`;
  `serverAuth` EKU; not_before `now`, not_after `now` + 800 days (`time`,
  already in `Cargo.lock` through `rcgen`, becomes a direct dependency; `now`
  is a parameter so the function is deterministic). The reissue reuses the
  same key from the Keychain, so `spki_sha256` is unchanged and every pairing
  stays valid: §5 pins the key and the `Pin` verifier
  (`src/adapters/remote.rs:183–197`) looks at nothing else.
- *`Identity` (`src/ports/remote.rs:25`) gains* `#[serde(default)] pub sans:
  Vec<String>` (the addresses the certificate names) and `#[serde(default)]
  pub certificate_issued: Option<SystemTime>`. `IDENTITY_SCHEMA_VERSION`
  (`src/ports/remote.rs:17`) is 1 today, so the bump is to 2: written as the
  value in that file at implementation time, plus one, because a ticket
  landing in parallel may take 2 first. Nothing reads `identity.json`'s
  `schema_version`; `RemoteFiles::load` (`src/adapters/remote.rs:55`) is plain
  serde, so a version-1 file loads with both fields empty and no migration
  step is needed.
- *When it runs.* Every path that mints a certificate gets the SAN, so the
  QR never shows a certificate without one:
  - `remote::make_identity` gains a parameter `sans: &[IpAddr]`. Its four call
    sites: `src/app.rs:2866` (the one wrapper, `App::make_identity`, which
    both the key-lost arm `:2810` and the first-run arm `:2836` use, and which
    passes `remote::local_address()`), `src/adapters/remote.rs:982` and
    `:1012` (unit tests, passing `&[]`), `tests/remote.rs:1423` (`&[]`).
  - The existing-identity arm of `load_identity` (`src/app.rs:2781`), after
    the key check succeeds and not when `read_only()`, calls
    `reissue_certificate` when `sans` is empty, when `local_address()` is set
    and not in `sans`, or when `certificate_issued` is over 700 days old, and
    saves as `make_identity` does. A reissue never touches `devices.json`. A
    LAN address that changes while the app runs is not caught until the next
    launch; the dialog says so (UI) via a new `Identity::covers(ip) -> bool`.

## The signals

No new core signal, `AppAction` or event. The page consumes the existing
`/api/v1/events` socket (`hello`, `changed`, `removed`, `resync_required`,
§4.4) and sends `watch`/`unwatch` for the open session's `screen` or
`conversation` key. What is new is on the wire into the host:

**Static paths, answered on the connection thread** (`connection`,
`src/adapters/remote.rs:770`, after the rate check and before
`shared.requests.try_send`). They are constants and decide nothing, which
CLAUDE.md puts in the adapter. The frame's `/api/v1/` refusal at
`src/app.rs:3023` is untouched, so any other path, `/other` included, keeps
its JSON 404.

| Path | Content-Type | Source |
|---|---|---|
| `GET /` | `text/html; charset=utf-8` | `web/index.html` |
| `GET /app.js` | `text/javascript; charset=utf-8` | `web/app.js` |
| `GET /app.css` | `text/css; charset=utf-8` | `web/app.css` |

The query and fragment do not matter. Any other method on these paths is
`405` JSON. A new `src/adapters/web.rs` holds the table (`include_str!` of
`web/`, the first text assets in the tree; `include_bytes!` is used only at
`src/main.rs:167` and `src/ui/theme.rs:203`) and the header set; `respond`
(`:530`) gains a sibling `respond_page(w, content_type, body)` that writes:

- `Content-Security-Policy: default-src 'none'; script-src 'self'; style-src
  'self'; connect-src 'self' wss:; img-src data:; base-uri 'none'; form-action
  'none'; frame-ancestors 'none'`. Facts (Settled, CSP line) says `'self'` does
  not cover `wss:` in all browsers, so the plan names the scheme.
- `Cache-Control: no-store` (no caching across versions),
  `X-Content-Type-Options: nosniff`, `Referrer-Policy: no-referrer`.

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
  `bearer`; `authenticate` is unchanged. The 101 echoes the constant
  `switchboard.v1`, never the token. Facts (Settled, RFC 6455 block) requires
  exactly this: the server must echo one offered value or the browser fails
  the connection; base64url tokens are valid token characters. Whether to
  echo is not constant: a native client that sent `Authorization` offered no
  subprotocol, and a 101 naming one it did not offer is a handshake error. So
  `RemoteRequest` gains `via_subprotocol: bool`, set in `read_request`, read
  by `connection` before the request moves into `RemoteIncoming`, and passed
  to `respond_upgrade`. The frame never reads it.
- *Why not the cookie.* A cookie is ambient: the browser attaches it to any
  request to that origin, so authentication stops being a header a
  cross-site page cannot send, and Facts (Settled, cookie block) says
  `SameSite` ignores the port, so another service on the same IP is
  same-site. That needs `Set-Cookie` on `RemoteReply`, a `Cookie` field, and
  an `Origin` check on every `POST` and the upgrade. The subprotocol keeps
  today's property that nothing authenticates unless the page's own script
  puts the token there. Whether a `Secure` cookie survives a click-through
  certificate is Unresolved in Facts; the chosen route does not depend on it.
  Fallback, built only if the owner asks: the `HttpOnly; Secure;
  SameSite=Strict` cookie with those `Origin` checks.
- *Where the token lives.* `localStorage` under the page's origin, readable by
  script on that origin. The mitigation is the CSP above and a page that never
  assigns `innerHTML`.

**The page's pairing steps** (`web/app.js`), in this order and nothing
before step 1:

1. Read `location.hash` (and `location.search`), then `history.replaceState(
   null, "", location.pathname)`. Whether that keeps the code out of Safari's
   history and recents is Unresolved in Facts and needs a phone.
2. `GET /api/v1/info`; refuse and say so unless `body.spki_sha256` equals
   `key` (case-insensitive) and `body.server_id` equals `id`. The page cannot
   see the certificate, only what `/info` claims (the issue's own caveat).
3. `POST /api/v1/pair {code, spki_sha256: key, device_name}`. A `403
   wrong_key`/`wrong_code`/`pairing_closed` is shown as that.
4. Store `{token, scope, server_id, key}` in `localStorage`, then `GET /me`
   for the live scope and start the session list.

Later visits: `/info` first; a key differing from the stored one → "identity
changed, pair again" and nothing is sent, token included (§5); otherwise
`/me`; a 401 says "revoked or removed", because `authenticate`
(`src/app.rs:3127`) answers one 401 for a revoked and an unknown token and a
rotated key deletes the devices (§5), so the issue's plain "revoked" cannot
be told from removal; a network error says "can't reach the host" with a
retry. The events socket reconnects with a backoff of 1 s doubling to 30 s,
reruns `/me` (this is how a revoke, which closes the socket from the host
side in `dispatch_inner` `src/app.rs:486`, shows), and refetches on a new
`epoch` or `resync_required`. The reply box exists only when `/me` says
`control`; it posts `{op, text}` to `/sessions/{id}/send` with `op` from
`crypto.getRandomValues` (the idempotency key, `src/app.rs:3159`), and a
`403 scope` hides the box again.

## The actions and effects

None in `AppAction`/`Effect`. One frame-side route: `serve_remote`
(`src/app.rs:3017`) matches `("GET", ["me"])` after `authenticate` and
answers `envelope(200, {device_id, name, scope}, true)`.

## The UI

Only `src/ui/remote.rs`, and the page itself.

- `pairing` (`:141`): `payload` becomes `invite_url(...)`, a pure function
  in `src/core/remote.rs` (`ip`, `port`, server id, key hash, code), because
  the dialog and the fixtures both need the same string; it formats
  `SocketAddr::new(ip, port)` (so IPv6 is bracketed) as
  `https://{addr}/#code=…&key=…&id=…`. The QR (`qr_texture` `:280`) encodes
  it. The text beside the code shows the address and short key as now. "Copy
  invite" copies the URL and its hover says "the QR code's address, to paste
  into a client". The scope line says "Scan with the camera, or paste the
  address into a client."
- When `!identity.listen.lan` the address is `127.0.0.1` and a line under
  the code says "Listening on this Mac only; a phone cannot reach it. Turn
  on Allow devices on the local network, above."
- When `identity.listen.lan` and `remote::local_address()` is set but
  `!identity.covers(ip)` a line says "This Mac's address changed since
  launch. Restart Switchboard so the certificate names it."
- The trust step, a sentence near the pairing window: "A browser will warn
  about this Mac's certificate the first time. Open the address only from the
  code on this screen and accept the warning for it; the page then checks the
  key against the one shown here before it sends the code. A page from a
  different host is not protected by that check." The hover on the LAN
  checkbox (`:113`) and §5's "Require pairing before anything but `/info` and
  `/pair`" name the three page files.

The page, `web/index.html`, `web/app.js`, `web/app.css`: no inline script or
style attribute. Text from the host is set with `textContent` only. It is a
single column that works at phone width (16 px side gutter, no horizontal
scroll) with `prefers-color-scheme` colors defined once on `:root`. Views:
pairing/status messages, the session list (waiting first, `card`,
`waiting_reason`, `kind`), the session view, the header (host name from
`/info`, device scope from `/me`).

## The spike

`spikes/23-web-surface/` (the next free number; 20, 21, 22 exist) holds the
one check no later test repeats: **a page against the pool of 4.** Six
parallel keep-alive connections fetching the three files from a listener
built like `connection`: how many are refused or time out, with and without
`Connection: close`. Its README holds the command and the numbers. A phone's
actual behavior stays Unresolved. The certificate-against-a-verifier check
and the 101 echo are the unit test and `a_subprotocol_token_is_checked_like_a_bearer`
below; the README points at them rather than repeating them.

## The tests

`tests/remote.rs` (a new helper beside `http()` `:245` that returns response
headers, since `http()` discards them):

- `the_page_is_three_files_with_their_headers`: `GET /`, `/app.js`,
  `/app.css` with no token answer 200, the content types in the table, and
  exactly the CSP string exported from `adapters::web`, `Cache-Control:
  no-store`, `nosniff`, `no-referrer`; `GET /?code=X` answers the same page;
  `POST /` is 405; `/other`, `/index.html` and `/app.js.map` stay the JSON
  404. A page request makes no `RemoteIncoming`.
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
  with a character outside `[A-Za-z0-9_-]` is ignored; a client offering no
  subprotocol gets a 101 with no `Sec-WebSocket-Protocol` line.
- `me_is_read_scope_and_unauthenticated_is_refused`.
- The fixtures test (`:1829`) gains `me.json` and `pairing_url.txt` in
  `write_fixtures` (`:1587`), the second written from `invite_url` with fixed
  inputs.

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
`src/adapters/remote.rs:117`) gain the new fields. `Identity::covers`. A
fresh identity from the first-run and key-lost arms carries the local
address in `sans` (through the fake `local_address` seam the app already
uses, or by asserting on `make_identity`'s `sans` argument).

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
the query form. These tests are in neither the hook nor CI; the implementer
runs `swift test` in `client/` if the toolchain is there and says in the
commit body if it was not.

The page's JavaScript has no test tooling in the tree, so only the owner's
phone run checks its behavior. What that run must settle is listed once, as
"Not run", in the design-status bullet below.

## Docs and files the commit touches

`docs/remote-client.md`: §4.1 bounds (the page window), §4.3 table (`GET /`,
`/app.js`, `/app.css`, `GET /api/v1/me`) and the corrected `last_response`
sentence, §5 (QR is a URL; the pairing sentence; the token sentence names the
subprotocol carrier for the events upgrade; the browser trust paragraph; the
"Require pairing before…" line), and a subsection after 4.7 for the page.
`docs/design.md`: a "Web page" bullet at the end of "Remote client
(2026-10-12)" (before `## Versioning and updates`, line 3585) with what is
built, "Not run" for every device-only check (each browser's click-through
and its `fetch`/`wss://` after it, the iOS camera opening the URL,
`replaceState` and Safari's history, a phone's parallel connections, the
socket after sleep, the acceptance run), and the gaps listed under Pool and
rate, The record and departure 2.
`README.md`: Layout rows for `web/`, `src/adapters/web.rs`,
`spikes/23-web-surface` (if the README lists spikes), the changed
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
  by the adapter.
- `GET /api/v1/me` → `{device_id, name, scope}` (read scope).
- WebSocket upgrade accepts `Sec-WebSocket-Protocol: switchboard.v1,
  bearer.<token>` when there is no `Authorization` header, and echoes
  `switchboard.v1`.
- Pairing invite is a URL, `https://<addr>/#code=&key=&id=` (the query form
  also accepted); `PairingInvite.parse` takes it or the JSON.
- `Identity.sans: Vec<String>` and `Identity.certificate_issued:
  Option<SystemTime>` (serde default), with `IDENTITY_SCHEMA_VERSION` bumped;
  the certificate is reissued over the same key with a SAN, `serverAuth` and
  800 days.
- `RemoteRequest.via_subprotocol: bool` (adapter-local); `remote::make_identity`
  gains `sans: &[IpAddr]`.
- `core::remote::invite_url`, `Identity::covers`, `adapters::web`,
  `adapters::remote::reissue_certificate`, `PAGE_PER_MINUTE`,
  `spikes/23-web-surface`.
