# Plan: a web control surface on the remote port (#230)

Base: `7fe8949` "Release 0.4.1". Every reference below was checked in this
worktree. One commit. The app runs at every step, because the page is a
set of extra routes on a port that already works: until the page exists,
those routes answer 404 as they do today.

## Where this departs from the issue

- **The code goes in the URL's fragment, not its query.** The issue's URL
  is `/?code=…&key=…&id=…`. A browser sends the query on the first
  request, so the code would reach whoever answers at that address before
  any key check. That breaks §5's rule that the key is verified before the
  code is sent. A fragment never leaves the browser. The QR is
  `https://<ip>:<port>/#code=<code>&key=<spki_sha256>&id=<server_id>`.
  The page still clears it with `history.replaceState` before it does
  anything else.
- **The list cannot show "the one-line excerpt the card shows". It is not
  on the wire.** `SessionView` (`control/src/lib.rs:637`) has `card`,
  `waiting` and `waiting_reason`, and no excerpt. The card computes its
  excerpt in the UI (`src/ui/cards.rs:176-185`), from the parsed
  transcript for agents and from the caption poll for the rest. The
  Stop hook's `last_message` lives only on the hook event
  (`src/ports/events.rs:24`) and never on the record.
  `docs/remote-client.md:207` says otherwise and is wrong. In this cut
  the list line is `waiting_reason` when the session waits and the `card`
  word otherwise. The session view shows the last response or the screen.
  A masked excerpt on `SessionView` would move the metadata fingerprint
  whenever content moves, so it needs its own design (follow-up issue).
  The plan corrects line 207.
- **One new read: `GET /api/v1/device`.** The issue asks for no new read
  model unless the page needs one. The page does need one. After a
  reload it holds no pair reply, so it cannot learn its scope (which
  decides whether the reply box shows) or its name any other way. The
  route returns `{device_id, name, scope}` and adds the capability
  `device`.
- **The host's certificate is not offered for download (decision 1).**
  `make_identity` (`src/adapters/remote.rs:95-127`) issues a certificate
  with no SAN, no `serverAuth` EKU and `not_after` in 4096. iOS and macOS
  refuse to trust such a leaf even after it is installed. Making download
  work would mean reissuing the certificate with SANs for an address that
  DHCP moves, plus a CA or a profile walkthrough. The trust step is the
  click-through.
- **The device walkthrough is the owner's.** CLAUDE.md asks for a spike
  before a risky mechanism. The browser questions (WSS and cookies under
  a certificate exception, how long the exception lasts, CSP `'self'`
  matching `wss:`) can only be answered on a phone by a person. The
  implementer writes `spikes/23-browser-surface/README.md` with the
  procedure, the Rust-side measurements and an empty results table. The
  page degrades to polling when the socket fails, so the design does not
  depend on the answers.

## Decisions

1. **Trust: click-through on the local network, said in the dialog.**
   - *What it buys:*
     - An encrypted channel to whoever answered at that address on the
       first visit (trust on first use).
     - A genuine page that refuses to post `/pair` when `/info`'s
       `spki_sha256` or `server_id` differs from the fragment's `key` and
       `id`. That catches a stale QR, a wrong port and a re-keyed host.
     - The host still refuses a `/pair` whose key is not its own
       (`src/app.rs:3636-3644`).
   - *What it does not buy:*
     - An active intermediary at the first visit shows its own warning
       and serves its own page, which reads the fragment.
     - A browser cannot pin, so a later certificate swap shows only one
       more warning.
   - *What the dialog says:* "A browser warns once that it does not know
     this Mac's certificate. Accept it only on the first visit. If it
     warns again for a browser you already paired, stop: the key changed
     or something is in between."
   - *Fingerprint:* the dialog shows the certificate's SHA-256 in short
     form (a browser's certificate viewer shows that hash, not the SPKI
     hash) beside the key, so the owner can compare by eye.
   - *§5's rule becomes:* before pairing, a non-loopback bind serves
     `/api/v1/info`, `/api/v1/pair` and the fixed static set in
     decision 4, and nothing else. The static files carry no host data.
2. **The token travels in a cookie.**
   - *The cookie:* `HttpOnly; Secure; SameSite=Strict; Path=/;
     Max-Age=34560000`. `Path=/` lets the page's own static loads carry
     it, which moves their connection to the paired pool. It is named
     `sb_<server_id simple>`, because cookies ignore the port and two
     instances on one Mac must not overwrite each other.
   - *Opt-in:* `/pair` takes an optional `"cookie": true`. With it, the
     reply sets the cookie and leaves `token` out of the body. Without
     it, native clients get the same reply as today.
   - *Order:* `authenticate` (`src/app.rs:3127`) reads the bearer header
     first, then the cookie.
   - *Origin check:* the page's POSTs and its WebSocket upgrade, and a
     `/pair` with `cookie: true`, must carry `Origin` equal to
     `https://<Host>`. Otherwise they get 403 `origin`. Same-origin GETs
     are exempt: Chrome sends no `Origin` on them, and SameSite=Strict
     already withholds the cookie cross-site. This is the second lock
     against cross-site request forgery, socket hijacking and DNS
     rebinding. Bearer requests are not checked, because native clients
     send no Origin.
   - *Revoked:* a 401 on a cookie-authenticated request also sends the
     cookie back with `Max-Age=0`.
   - *Why not `Sec-WebSocket-Protocol`:* it puts the token in script
     memory and, across reloads, in `localStorage`. A cookie stays out
     of the page's reach.
   - *Spec text:* "sent only as a bearer header" becomes "a bearer
     header, or for a browser an HttpOnly cookie". "Never in a URL"
     stands.
3. **One QR: the URL.**
   - *The QR:* the fragment URL replaces the JSON payload.
   - *Copy invite:* copies the same URL.
   - *Native clients:* `PairingInvite.parse` (`client/Sources/SwitchboardProtocol/Info.swift:113-116`)
     takes either the JSON, as before, or the URL: `addresses =
     [host:port]`, `key` to `spki_sha256`, `id` to `server_id`, from the
     fragment or, leniently, the query. `switchboard-remote pair`
     (`client/Sources/switchboard-remote/main.swift:111`), the headset's
     paste (`client/Sources/SwitchboardUI/BrowserView.swift:883`) and
     `SWITCHBOARD_INVITE` (`BrowserView.swift:106`) then take the URL
     with no change of their own.
   - *Why JSON stays readable:* only invites already copied produce it.
     Nothing scans it.
   - *Address:* the URL uses `local_address()`
     (`src/adapters/remote.rs:962`), as the QR does today. A `.local` name
     would survive a DHCP move, but the name Bonjour publishes
     (`hostname -s`, `src/adapters/bonjour.rs:41`) need not equal the
     LocalHostName. The spike records whether `.local` works, for a later
     change. The dialog says: "If this Mac's address changes, pair the
     browser again."
4. **Static paths.**
   - *Where they route:* before the `/api/v1/` strip in `serve_remote`
     (`src/app.rs:3023`), `GET` only:

     | Path | Answer |
     |---|---|
     | `/` | 200 `text/html; charset=utf-8` (`web/index.html`) |
     | `/app.js` | 200 `text/javascript; charset=utf-8` |
     | `/app.css` | 200 `text/css; charset=utf-8` |
     | `/favicon.ico` | 204, empty, so a browser's unasked request leaves no 404 log line |

     Every other path outside `/api/v1/` stays 404 through `refuse`,
     which logs method and path but never the query (or the fragment,
     which never arrives).
   - *Headers on every static reply:*
     - `Content-Security-Policy: default-src 'none'; script-src 'self'; style-src 'self'; connect-src 'self'; img-src 'self'; base-uri 'none'; form-action 'none'; frame-ancestors 'none'`.
     - `Referrer-Policy: no-referrer` and `X-Frame-Options: DENY`.
     - The HTML has no inline script or style. It loads `app.js` with
       `type="module"`, which defers by default.
     - If the spike finds that Safari's `'self'` does not match `wss:`,
       `connect-src` gains `wss://<Host>`, built from the request's Host
       header.
   - *Headers on every reply, API included* (`respond`,
     `src/adapters/remote.rs:530`): `Cache-Control: no-store` and
     `X-Content-Type-Options: nosniff`. no-store keeps a page and an API
     from different versions from ever mixing, and the files are small.
   - *Pool:* a cold, unpaired load is three requests plus the favicon,
     on keep-alive connections, inside the four-slot unauthenticated
     pool (`src/adapters/remote.rs:564`). A static request carrying a
     valid cookie answers `authenticated: true`, so a paired browser's
     loads move to the paired pool as its API calls do.
   - *Rate:* `Limits::limited` (`src/adapters/remote.rs:631`) counts the
     static paths against the `/info` window (60 a minute). A load costs
     about 4 of them, and the `/pair` window is untouched.
   - *With no fragment and no cookie,* the page shows: "Not paired. On
     the Mac, open the settings menu, then Remote devices, then Allow
     pairing, and scan the code." It sends nothing else.
   - *With a cookie and no fragment,* it rehydrates (see "The page"
     below).
5. **Where the page lives.**
   - *Files:* `web/index.html`, `web/app.js` and `web/app.css`, embedded
     by a new `src/adapters/web.rs` with `include_str!`. It holds the
     path→(type, bytes) table and the header constants.
   - *Code:* plain JS, no framework, no build step, no dependency.
   - *Tests:* listed under "Tests" below.
6. **The device record.** A browser is an ordinary `PairedDevice`
   (`src/ports/remote.rs:96`).
   - *Name:* the page derives it from `navigator.userAgent` ("Safari on
     iPhone", "Chrome on Android", "Safari on Mac", otherwise "Browser")
     and sends it as `device_name`. The host does no user-agent parsing
     and already caps the name (`src/core/remote.rs:166`).
   - *Scope:* chosen in the pairing window, as today.
   - *Dialog:* lists and revokes it unchanged.

## The record

No record change. `SCHEMA_VERSION` stays 16 (`src/core/model.rs:12`),
`VIEWS_SCHEMA_VERSION` stays 7 (`model.rs:612`) and
`IDENTITY_SCHEMA_VERSION` stays 1 (`src/ports/remote.rs:17`).
- `devices.json` gains no field.
- The page is not a record. It is compiled into the binary.
- On the device, the token is in the cookie. `localStorage` holds only
  `{server_id, spki_sha256, server_name}` from the pairing, which is not
  secret and is used to tell "revoked" from "identity changed".
- Host content stays in memory only, as §5 asks of every client.

## The signals

No new signal. The page uses the existing `/api/v1/events` socket:
- `hello` carries the epoch and the revision.
- `changed {session|sessions|waiting}`.
- `removed`, `resync_required`, and `watch` / `watching` for the open
  session's `conversation` or `screen`.

## The adapter and the app (no core action or effect)

- **`RemoteRequest`** (`src/adapters/remote.rs:360`) gains three fields,
  filled in `read_request` (444-515):
  - `cookies: Vec<(String, String)>`: only the `sb_*` pairs, taken out
    of the headers the way `bearer` is, so they are never logged with
    them;
  - `origin: Option<String>`;
  - `host: Option<String>`.
- **`RemoteReply`** (389) gains `content_type: &'static str` and
  `headers: Vec<(&'static str, String)>`.
  - Add `RemoteReply::json(status, body, authenticated)` and turn the
    30 literals into it: 28 in `src/app.rs` and 2 in
    `src/adapters/remote.rs`.
  - `respond` writes `content_type`, the extra headers and the two
    every-reply headers, and gains the reason "No Content" for 204.
- **`Limits::limited`** maps `/`, `/app.js`, `/app.css` and
  `/favicon.ico` to the info window.
- **`serve_remote`** (`src/app.rs:3017`) answers the static set from
  `web::asset(path)` before the `/api/v1/` strip.
  - It asks `authenticate` only when a cookie is present, and then only
    to set `authenticated`. A dead cookie on a static request is not an
    error: the page loads and finds out on its first API call.
  - The `/api/v1/device` route follows `authenticate`.
- **`authenticate`** (3127) tries the bearer first, then the cookie
  named for this installation.
  - On a cookie hit with a non-GET method or an upgrade, it checks
    `origin == format!("https://{host}")`, or answers 403 `origin`.
  - A cookie miss answers 401 with the clearing `Set-Cookie`.
- **`remote_pair`** (3606):
  - with `cookie: true`, it checks Origin, then sets the cookie and
    leaves out `token`;
  - otherwise the reply is unchanged.
- **`remote_info`** (3568) adds `"device"` to `capabilities`.
- **`src/core/remote.rs`** gains a pure
  `pairing_url(address: &str, identity: &Identity, window: &PairingWindow) -> String`,
  so building the URL leaves `ui/`. The code is Crockford, the key is
  hex and the id is a UUID, so nothing needs escaping.
- **`src/adapters/remote.rs`** gains
  `certificate_sha256(identity) -> Option<String>` (`sha256_hex` over
  `certificate_der`, 130 and 292).

## The page

`web/app.js` speaks §4.4 as the headset does.

1. **On load:**
   - If `location.hash` holds a code, read `code`, `key` and `id`, then
     `history.replaceState(null, "", "/")` first.
   - `GET /api/v1/info`.
2. **Pairing (fragment present):**
   - If `key` or `id` differs from `/info`, show "This is not the host
     whose code was scanned" and stop.
   - Otherwise `POST /api/v1/pair {code, spki_sha256: key, device_name,
     cookie: true}`.
   - On 200, store `{server_id, spki_sha256, server_name}`.
   - On `wrong_code` or `pairing_closed`, say so.
3. **Rehydrate (no fragment):**
   - If the stored key or id differs from `/info`, show "This host's
     identity changed. Pair again." and send nothing more.
   - `GET /api/v1/device`. A 401 with the same key shows "This browser
     was revoked on the Mac. Pair again from the Mac." A 401 with nothing
     stored shows the unpaired text.
4. **Live:**
   - Open `wss://<location.host>/api/v1/events`, wait for `hello` and
     buffer events until then.
   - Read the snapshot: `GET /projects`, `/projects/{id}/sessions` for
     each project, then `/waiting`.
   - Apply the buffered events by revision.
   - Event handling:
     - `changed` refetches its key;
     - a new epoch refetches everything;
     - `resync_required` reconnects.
   - `/device` is re-read on each `hello` and on any 403 `scope`.
   - When the socket closes, the page sends one `/device`. A 401 there
     shows "revoked". Revoke closes the socket (`src/app.rs:485-498`), so
     that request comes at once.
   - Otherwise it reconnects with backoff (1, 2, 4 … 30 s). While
     disconnected, it polls `/waiting` and the open session every 10 s,
     so a browser where WSS under the exception fails still works.
5. **Views:**
   - *Header:* the host's name and the device's scope.
   - *List:* sessions grouped by project, waiting first and marked, each
     with kind, liveness and the line from "Where this departs".
   - *Session view:*
     - Claude Code: `GET sessions/{id}/last-response`. Anything else:
       `GET sessions/{id}/screen`.
     - It registers `watch`, refetches on `changed` and every 30 s.
   - *Reply box:* shown only under `control` scope. It posts
     `sessions/{id}/send {op, text}`, with a UUID made from
     `crypto.getRandomValues`.
     - A timeout is followed by `GET ops/{op}`, never by a resend (§4.5).
     - On `trust_question` it says the session is at the folder trust
       question, with no trust button in this cut.
6. **Text:** every string goes in through `textContent` and never through
   `innerHTML`. With no inline script, the CSP holds.

## The UI

The Remote devices dialog (`src/ui/remote.rs`):
- `pairing()` (160-169) builds the payload with `core::remote::pairing_url`.
  The QR and "Copy invite" use the URL.
- The help line (191-197) gains "Scan with a phone's camera to pair its
  browser."
- A "Certificate <short sha256>" meta line sits under "Key …".
- A small paragraph under the device list, beside the redaction
  sentence (69-77), carries the click-through warning from decision 1
  and the address-change line.

## Tests

- **Core** (`src/core/tests.rs`): `pairing_url` holds the fragment, no
  query, and the code, key and id.
- **`tests/remote.rs`.** Add `http_full`, which takes extra request
  headers and returns the response headers. `http` (245) becomes a
  wrapper over it.
  - *The page references only what is served:* parse `web::INDEX` for
    `src=`/`href=`, and `web::APP_JS` for `'/…'` path literals and the
    WebSocket path. Each one is in the static set or under `/api/v1/`.
  - *The static set's headers:* unpaired, on a LAN-style bind, each
    path answers 200 (204 for the favicon) with its `Content-Type`, the
    CSP, `no-store` and `nosniff`. `/other` and `/app.js.map` answer 404.
    An API reply carries `no-store`.
  - *Browser pairing end to end:*
    - `/info`, then `/pair {cookie:true}` with a matching Origin. The
      reply has a `Set-Cookie` with `HttpOnly`, `Secure`,
      `SameSite=Strict` and the installation's name, and no `token`.
    - `GET /projects` with only the cookie answers 200.
    - `POST sessions/{id}/send` with the cookie and no Origin, or a
      foreign one, answers 403 `origin`. With the matching Origin it
      answers 200.
    - `/pair {cookie:true}` with a foreign Origin answers 403.
    - `GET /device` returns the scope.
    - A static request with the cookie moves the connection to the
      paired pool: fill the unauthenticated pool, then load.
  - *The socket:* the upgrade with the cookie and a matching Origin gets
    101. Without Origin it gets 403. The helper at 1981 builds the
    request, so the headers are added there.
  - *Revoke:* the cookie request answers 401 with `Max-Age=0`.
  - *Rate:* static loads count against the info window, and `/pair`'s
    window is untouched (beside `pair_is_rate_limited_per_listener`,
    1406).
  - *Unchanged:* `info_is_served_to_anyone_and_every_other_read_is_refused_unpaired`
    (426) and `the_allowlist_refuses_everything_else_with_no_side_effect`
    (682) stay green with `/device` added to the refused-unpaired list.
- **Fixtures** (`write_fixtures`, 1587): add `device.json`, the
  `/device` reply, for the Swift side.
- **`tests/ui.rs`** (7476-7490): the copied invite is a URL that starts
  with `https://` and whose fragment carries `code=<code>` and
  `key=<spki>`. The JSON asserts are replaced.
- **Swift** (`client/Tests/SwitchboardProtocolTests/FixturesTests.swift:157`):
  `PairingInvite.parse` cases for the URL (fragment and query) beside the
  JSON case. Run with `swift test` in `client/`.
- **Manual acceptance:** scan, accept the warning, see the list, a
  session starts waiting and moves up, reply, revoke. The owner runs it
  on a phone, following the spike README, and records the results in
  its table.

## Docs

- **`docs/remote-client.md`:**
  - §4.1: the static set, its pool and its rate.
  - §4.3: the static rows, `GET /device`, `/pair {cookie}`, the Origin
    rule, and line 207 corrected.
  - §5: browser trust (what it buys and does not), the cookie, the
    fragment URL as the QR, and the before-pairing rule.
  - §11: the new tests.
  - §13: the authentication and client-storage rows.
- **`docs/design.md`:**
  - a "Browser surface (2026-10-…)" entry in the "Remote client
    (2026-10-12)" status, before "Known gaps" (3573);
  - the device rows of the spike as a known gap until the owner
    measures them;
  - the trust-boundary paragraph: the token may live in a browser cookie.
- **`README.md`:**
  - Layout lines for `web/` and `src/adapters/web.rs`;
  - the dialog's line (423) now says "QR code (a URL a phone opens)".
- **`client/README.md`** (180-235): the invite is a URL, and JSON still
  parses.
- **`spikes/23-browser-surface/README.md`** and the spikes index.
- **`CHANGELOG.md` Unreleased:** "A browser on the local network can pair
  from the pairing window's QR code and list sessions, read the last
  response or screen, and send a reply."

## Commit

`Remote port serves a browser page, paired from a URL that carries the code (#230)`

Contract additions:
- `model.rs`, `action.rs`: none. `ports/remote.rs`: none. No schema bump.
  If a parallel ticket changes a serialized type first, any bump here is
  from the value in `model.rs` at implementation time, plus one.
- Wire protocol, all additive:
  - `GET /api/v1/device` → `{device_id, name, scope}`;
  - capability `device` in `/info`;
  - `/pair` body field `cookie: bool`, which makes the reply set
    `Set-Cookie: sb_<server_id simple>` and omit `token`;
  - cookie authentication with the Origin rule and error code `origin`;
  - static `GET /`, `/app.js`, `/app.css`, `/favicon.ico`;
  - `Cache-Control: no-store` and `X-Content-Type-Options: nosniff` on
    every reply;
  - the invite as `https://<addr>:<port>/#code=&key=&id=`, with the JSON
    still accepted.
- Adapter types:
  - `RemoteRequest.{cookies, origin, host}`;
  - `RemoteReply.{content_type, headers}` and `RemoteReply::json`;
  - `adapters::web` (`INDEX`, `APP_JS`, `APP_CSS`, `asset`);
  - `adapters::remote::certificate_sha256`;
  - `core::remote::pairing_url`.
- Swift: `PairingInvite.parse` accepts the URL form.
