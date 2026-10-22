# A web control surface (#230)

The remote port also serves a browser page: three files embedded in
the binary, opened from the pairing window's QR as a URL, paired by
the code in that URL, authenticated afterwards by an `HttpOnly` cookie
that `fetch` and the events WebSocket both send. The page speaks the
protocol the headset speaks. No record changes. `identity.json` gains
one field. A spike comes before any code.

## Where this departs from the issue

- **The code goes in the fragment, not the query.** The issue's
  `/?code=…&key=…&id=…` puts the code in the request line of the very
  first `GET /`, so it reaches whatever answers at that address before
  the page has compared any key. The plan's URL is
  `https://<address>:<port>/#code=<code>&key=<spki_sha256>&id=<server_id>`.
  A fragment is never sent to a server and never appears in a
  `Referer`. The code then leaves the browser only in the `/pair` the
  page posts after the key comparison. The page still clears it with
  `history.replaceState` before doing anything else.
- **The certificate as it stands cannot be clicked through on Apple
  platforms.** `make_identity` (`src/adapters/remote.rs:95-127`) makes a
  certificate with no subject alternative name
  (`CertificateParams::new(Vec::new())`), no extended key usage, and
  rcgen's default `not_after` in the year 4096. iOS 13 and macOS 10.15
  refuse such a server certificate. Safari and Chrome on macOS say so
  with no "proceed" option (Apple's TLS requirements: SAN required, EKU
  `serverAuth`, at most 825 days). The plan reissues the certificate
  **over the same key** so that it meets those rules (see "The record"
  and "The port"). The SPKI hash does not change, so every native pin
  and every pairing stays valid (§5 already allows a reissue over the
  same key). Spike 23 confirms both halves before anything is built on
  them.
- **There is no one-line excerpt on the wire.** The card's excerpt is
  `agent_excerpt` (`src/ui/cards.rs:558`), computed in the UI from the
  parsed transcript. `SessionView` (`control/src/lib.rs:637-688`) has no
  such field. §4.3's `last_message` "from the `Stop` hook on the record"
  is not one either. Adding it would mean parsing a transcript on the
  frame, which §4.1 forbids. So a list row shows `waiting_reason` when
  the session waits and otherwise the card word with its age. The last
  response is shown when the session is opened.
- **The page needs to know its own scope after a reload, and no
  endpoint says it.** The `/pair` reply carries the scope once. A
  cookie the script cannot read gives no way to recover it, and the
  scope can change in the dialog afterwards. The plan adds
  `GET /api/v1/device` (`{id, name, scope}` of the asking device),
  under read scope. It is the page's "am I paired?" probe and the
  carrier of the cookie's renewal.
- **"Sessions grouped by project" costs one request per project.**
  There is no all-sessions read, so the page reads `/projects` and then
  `/projects/{id}/sessions` for each project, one request at a time
  (see "The UI" for why one at a time). It does not read `/waiting`:
  `waiting` is on every `SessionView`.

## The decisions

1. **Browser trust: the click-through is the trust step, and the
   dialog says so.**
   - *What it buys:* TLS against a passive listener on the local
     network. A genuine page refuses to send the code to a host whose
     `/info` key or id differs from the QR's; that catches a second
     installation at the address, or a stale QR after a key rotation.
   - *What it does not buy:* a browser cannot pin. An active attacker
     who answers at the address presents a certificate warning of its
     own. That warning looks exactly like the genuine one. If the owner
     accepts it, the attacker serves its own page, reads the fragment,
     and can pair with the real host using the real key hash. The
     host's `spki_sha256` check cannot tell. The cookie is scoped to
     the host and not the port, so another service on the same address
     whose warning the owner accepts would also receive it.
   - *What the owner can check:* the dialog shows the certificate's
     SHA-256 fingerprint (what Safari's and Chrome's certificate
     details show) beside the key. The trust sentence reads: "A browser
     cannot check this Mac's key the way the headset does: accepting
     its certificate warning is what trusts this Mac. Accept a warning
     only when its certificate details show the fingerprint above."
   - *Why not download the certificate instead:* a downloaded
     certificate trusted on iOS becomes a trust anchor for every name
     on the device. Installing it takes a profile install plus a trip
     through Certificate Trust Settings. It would also need the same
     SAN, EKU and validity rules anyway. That is more risk and more
     steps for the same LAN-only reach.
   - *§5's rule:* it becomes "nothing but `/info`, `/pair` and the
     page's three files (`/`, `/app.js`, `/app.css`), which carry no
     host data, before pairing".
2. **The token travels in a cookie.** The cookie is
   `__Host-switchboard=<token>; Path=/; Secure; HttpOnly; SameSite=Strict; Max-Age=34560000`
   (400 days, the most Chrome keeps). It is set by a `/pair` that asks
   for it, and renewed by every cookie-authenticated `/device`, which
   the page calls on each load.
   - The token is never in the reply body, never in script-readable
     storage, and never in a URL. `localStorage` holds only the public
     marker `{server_id, spki_sha256, device_id}`. The page uses it to
     tell "revoked" (same key, 401) from "identity changed" (different
     key).
   - `Sec-WebSocket-Protocol` is rejected because the script would
     have to hold the token.
   - The bearer header still wins when present, so the native clients
     are untouched.
   - CSRF: a cookie-authenticated `POST` or WebSocket upgrade must
     carry an `Origin` equal to `https://<Host header>`. Otherwise it
     is refused `403 origin`. `SameSite=Strict` already keeps
     cross-site requests from carrying the cookie; the `Origin` check
     does not lean on browser behaviour alone.
3. **One QR, the URL.** The dialog's QR and its "Copy invite" both
   carry the URL. `PairingInvite.parse` (Swift) accepts the URL and
   still accepts the JSON. Nothing produces the JSON any more, but
   pasted invites and the fixture test keep it readable. Both the
   headset's paste and `switchboard-remote pair` go through `parse`
   (`client/Sources/SwitchboardUI/BrowserView.swift:106,883`,
   `client/Sources/switchboard-remote/main.swift:111`).
4. **Static paths.**
   - *The paths:* `GET /` (`text/html; charset=utf-8`), `GET /app.js`
     (`text/javascript; charset=utf-8`) and `GET /app.css`
     (`text/css; charset=utf-8`). Nothing else outside `/api/v1/` is
     served; `/favicon.ico` answers 404 as `/other` does today.
   - *Headers on the three:*
     `Content-Security-Policy: default-src 'none'; script-src 'self'; style-src 'self'; connect-src 'self'; base-uri 'none'; form-action 'none'; frame-ancestors 'none'`,
     `Cache-Control: no-store`, `X-Content-Type-Options: nosniff` and
     `Referrer-Policy: no-referrer`.
   - *Every API reply too:* `Cache-Control: no-store` and `nosniff`,
     so a browser never keeps session text on disk. This is harmless
     to the native clients.
   - *No caching across versions:* `no-store` means a new build's page
     is the next load's page, with no version in any path.
   - *Pool and rate:* the files are answered by the connection thread,
     never the frame. They count against the **unauthenticated pool**
     (4) like `/info`, and against a **page window of 60 a minute per
     listener**, separate from `/info`'s so a reloading browser never
     starves a headset's `/info`. A browser's keep-alive connection
     moves to the paired pool on its first cookie-authenticated API
     request, as a native one does on its first bearer request.
   - *Opened with no code and no cookie:* the page shows the host's
     name from `/info` and "Not paired. On the Mac, open Remote devices
     in the settings menu, choose Allow pairing, and scan the code with
     this device's camera."
5. **The page lives in `web/`** (`index.html`, `app.js`, `app.css`),
   embedded with `include_str!` in a new `src/adapters/web.rs`, and is
   tested from `tests/remote.rs` (see "The tests"). The fixtures do not
   change: no view changes, and `/device` has no Swift reader yet.
6. **The device record is a `PairedDevice` like any other.**
   - The page names itself from `navigator.userAgent`: `Safari on
     iPhone`, `Chrome on Mac` and so on. An iPad's desktop-class user
     agent is told apart by `navigator.maxTouchPoints > 1`.
   - The host keeps its 64-character cut (`new_device`,
     `src/core/remote.rs:166`).
   - The scope is the pairing window's, as today. The device is listed
     and revoked in the same dialog.

## Spike 23, first

`spikes/23-browser-page/` is a standalone Cargo project like spike 22.
It is one binary over `rustls` and `rcgen` that serves a test page
under the CSP above. The page has a `POST /set` that sets the cookie,
a `GET /echo` that reports whether the cookie and `Origin` arrived, and
a WebSocket that reports the same. It makes the certificate in two
forms: `--legacy` (today's: no SAN, year 4096) and the reissued form.

Run it on Safari on iPhone and iPad, Safari on the Mac and Chrome on
the Mac. Its README records, per browser:

- whether the legacy certificate can be clicked through (expected:
  no);
- whether the reissued one can;
- whether the cookie is stored, sent on `fetch`, sent on
  `new WebSocket`, and still present after a reload and after a
  browser restart;
- whether `connect-src 'self'` admits the `wss:`;
- whether `Origin` arrives on the POST and the upgrade.

The decision rules, fixed now:

- If the reissued certificate cannot be clicked through either, stop
  and report.
- If a browser does not send the cookie on the WebSocket, the page's
  polling fallback (see "The UI") is its normal mode there, and the
  design status says so.
- If a browser drops the cookie at restart, the page says "pair again
  after the browser restarts" for that browser.

## The record

- **No record change.** `SCHEMA_VERSION` (16, `src/core/model.rs:12`)
  and `VIEWS_SCHEMA_VERSION` (7, `src/core/model.rs:612`) stand.
  `devices.json` is unchanged: a browser is a `PairedDevice`.
- **`identity.json` gains one field.** It is
  `#[serde(default)] certificate_names: Vec<String>` on `Identity`
  (`src/ports/remote.rs:26-42`): the names and addresses the current
  certificate is issued for.
  - `IDENTITY_SCHEMA_VERSION` (1 today, `src/ports/remote.rs:17`) goes
    to the value in `ports/remote.rs` at implementation time plus one.
  - An older file reads with no names, which marks its certificate for
    reissue at the next bind. Nothing refuses a newer `identity.json`
    today and nothing starts to.
  - `created` keeps its spec meaning, the certificate's creation time
    (§3.1), so a reissue moves it.

## The signals

No new event. The page consumes `hello`, `changed`, `removed`,
`watching` and `resync_required` exactly as the headset does.

## Actions, effects and the port

**One action.** `AppAction::CertificateReissued(Identity)`, handled in
`remote_action` (`src/core/remote.rs:21-103`), replaces the held
identity only when its `server_id` and `spki_sha256` equal the held
ones. A reissue never changes who the host is. There is no effect: the
app saved the file before dispatching, as `make_identity` does
(`src/app.rs:2860-2888`). Core test in `src/core/tests.rs`: a reissue
with the same id and key replaces the certificate; one with another
key is ignored.

**One pure function in the core.** `pairing_url(address: SocketAddr,
server_id, spki_sha256, code) -> String` in `src/core/remote.rs`
builds `https://{address}/#code=…&key=…&id=…`. `SocketAddr`'s
`Display` brackets IPv6. Core test: the exact string for an IPv4 and
an IPv6 address.

**The certificate** (`src/adapters/remote.rs`):

- One builder, `certificate(key: &rcgen::KeyPair, server_id, names:
  &[String], now: SystemTime)`, with:
  - the common name as today;
  - a SAN per name (`SanType::IpAddress` for an address,
    `SanType::DnsName` otherwise);
  - `extended_key_usages = [ServerAuth]`;
  - `not_before` a day before `now` and `not_after` 397 days after.
    `time::OffsetDateTime`, already in `Cargo.lock` through rcgen,
    becomes a direct dependency at that version, which adds no crate.
- `make_identity` uses the builder with `["localhost", "127.0.0.1"]`.
- `reissue(identity, key_pem, names, now) -> Result<Identity>` keeps
  the key, so `spki_sha256` is unchanged.
- `certificate_current(identity, names, now) -> bool` is true when
  every name is in `certificate_names` and `created` is under 367 days
  old.
- `certificate_fingerprint(identity) -> String` is the SHA-256 of the
  DER as uppercase byte pairs: the first eight in the dialog, the whole
  in its hover.

**When it is reissued** (`src/app.rs`):

- In `bind_remote` (`2936-2986`), before `tls_config` (`2957`): the
  wanted names are `localhost`, `127.0.0.1`, and
  `remote::local_address()` when `listen.lan`. When the certificate is
  not current for them, `reissue`, `services.remote.save_identity`,
  then `dispatch_inner(CertificateReissued)`. The TLS configuration is
  built from the reissued identity. A failed save logs and keeps the
  old certificate; the port still binds.
- After an `OpenPairing` dispatch, in `dispatch_inner` beside the
  `RevokeDevice` arm (`486`): if the certificate is not current for
  the address the QR will name, `bind_remote()` again. The phone is
  about to open that address, and a rebind drops connections, which
  devices reconnect from anyway.

**The adapter** (`src/adapters/remote.rs`):

- `RemoteRequest` (`359-372`) gains three fields, filled in by
  `read_request` (`444-514`):
  - `cookie: Option<String>`, the `__Host-switchboard` value, taken
    out of the `Cookie` header among other cookies the way `bearer` is
    taken out, so it is never logged;
  - `origin: Option<String>`;
  - `host: Option<String>`.
- `RemoteReply` (`388-393`) gains `set_cookie: Option<String>`. The 28
  literals in `src/app.rs` and the 2 here get `set_cookie: None`.
- `respond` (`530-552`) writes `Cache-Control: no-store` and
  `X-Content-Type-Options: nosniff` on every reply, and the
  `Set-Cookie` when there is one. A `respond_asset` writes an asset
  with its content type and the four headers above.
- In `connection` (`770-867`), after the rate check at `788`: a `GET`
  of a path in `web::asset(path)` is answered there and the loop
  continues. It never touches the inbox, the frame or the pools.
- `Limits` (`623-639`) gains `page: Mutex<RateWindow>` at
  `PAGE_PER_MINUTE = 60`, matched for the three paths.

**`src/adapters/web.rs`** (new):

- `pub struct Asset { pub content_type: &'static str, pub body:
  &'static str }`.
- `pub fn asset(path: &str) -> Option<Asset>` over the three
  `include_str!("../../web/…")` files.
- `pub const PATHS: [&str; 3]`.
- `pub const CSP: &str`.

**The frame** (`src/app.rs`):

- `authenticate` (`3127-3152`): the token is `bearer`, else `cookie`.
  For a cookie, a `POST` or an upgrade whose `origin` is not
  `https://{host}` is refused `403 origin` before the device is looked
  up. Unknown tokens answer as today, `401 unauthorized`.
- `serve_remote` (`3017-3123`) gains `("GET", ["device"])`. It answers
  `{id, name, scope}` in the envelope, and when the token came by
  cookie it sets `set_cookie` to the same token with a fresh
  `Max-Age`. The `/api/v1/` prefix match at `3023` is unchanged: the
  page's files never reach it.
- `remote_pair` (`3606-3673`): the body may carry `cookie: true`. Then
  `origin` must match before the key and the code are checked; a
  mismatch is `403 origin` and does not count against the window. The
  reply body drops `token`, and `set_cookie` carries it. A native
  `/pair` is unchanged, `pair.json` included.
- `remote_info` (`3568-3599`): `capabilities` gains `"device"`.

## The UI

**The Remote devices dialog** (`src/ui/remote.rs`):

- The QR and "Copy invite" carry `core::remote::pairing_url(…)` in
  place of the JSON at `160-166`.
- With no local address there is no QR. The code and key are shown
  for the headset, with "No local network address; a phone cannot
  open this Mac's page."
- With `listen.lan` off, a line under the code: "This Mac answers
  only itself; allow devices on the local network for a phone to open
  the page."
- The caption at `193-197` becomes "Scan with a phone's or tablet's
  camera to open this Mac's page in its browser, or type the key and
  the code on the headset. Grants {scope} scope. Open for five minutes
  or three wrong codes."
- A `Certificate AB CD EF 01 23 45 67 89` line under the Key line,
  from `certificate_fingerprint`, kept in `RemoteDraft` at `open`.
- The trust sentence from decision 1 under the redaction sentence at
  `70-76`.

**The page** (`web/`, plain ES2020 in one script, no module, no
framework):

- *Load:*
  1. The first statement reads `location.hash`. If it holds `code=`,
     the page keeps the invite in a local variable and calls
     `history.replaceState(null, "", "/")`.
  2. `GET /info` gives the server name, the key and `pairing_open`.
- *With an invite:*
  - The invite's `key` or `id` differs from `/info`'s: "This is not
    the Mac whose code you scanned; nothing was sent."
  - `pairing_open` is false: "The pairing window on the Mac is closed;
    open it again and scan the new code."
  - Otherwise the page posts `/pair` with
    `{code, device_name, spki_sha256, cookie: true}`. On 200 it stores
    the marker. `wrong_code`, `pairing_closed` and `rate_limited` are
    said in words.
- *Then `GET /device`:*
  - 200: the header shows the host name and `read` or `control`.
  - 401 with the marker's key equal to `/info`'s: "This browser's
    pairing was revoked on the Mac. Pair again from the pairing
    window."
  - 401 with the key different: "This Mac's identity changed. Pair
    again."
  - 401 with no marker: the not-paired text of decision 4.
- *Sync:*
  - The page opens `/api/v1/events` and waits for `hello`.
  - It takes its snapshot: `/projects`, then each project's sessions.
  - Each resource key holds a revision, and a reply applies only at
    or past it. `changed {session}` refetches the session,
    `changed {sessions, p}` the list, `changed {projects}` the
    projects. `removed` drops the resource. A new `epoch` refetches
    everything.
  - `resync_required`, a close or `visibilitychange` to visible
    reconnects (1 s doubling to 30 s) and resnapshots.
  - If the socket fails twice while `/device` answers 200, the page
    polls the list and the open view every 5 s, and the header says
    "refreshing every 5 s".
- *One request in flight at a time:* a promise queue, so a browser
  holds one keep-alive connection plus the socket and never more than
  the unauthenticated pool's four before its first authenticated
  request.
- *The list:*
  - Projects that hold a waiting session come first, then the rest by
    name.
  - Within a project: waiting first, then the card rank of §8, then
    `last_activity_ms` descending.
  - Each row has the name, kind, card word and a "waiting on you"
    mark, with `waiting_reason` or the card word and age as its line.
  - A session that starts waiting moves to the top on its `changed`,
    without a reload.
- *A session:*
  - Opening one sends `watch` (`conversation` for `claude`, `screen`
    otherwise) and fetches on `watching`: `/last-response` (its
    `final_text` in a `pre-wrap` block, "earlier text cut" when
    `truncated`) or `/screen?lines=80`.
  - It refetches on `changed` for that key, and on a 30 s timer (§4.4).
  - Back sends `unwatch`.
  - A not-running session shows the refusal's reason.
  - `trust_question` shows "Claude Code is asking to trust this
    folder; answer it on the Mac."
- *The reply box:*
  - Shown only under `control`, for a running `claude`, `codex` or
    `shell`.
  - Send posts `{op, text}`, with the op made from
    `crypto.getRandomValues` (`randomUUID` needs a secure context,
    which a clicked-through origin may not be).
  - 200 clears the box. 409 `interrupted` says "The Mac started this
    send and its outcome is unknown; look at the session before
    sending again." A network error or timeout asks `/ops/{op}` and
    follows §4.5's rule; it never resends blindly.
- *Text:* every string goes in through `textContent`; `innerHTML` is
  never used. `app.css` uses system fonts and has light and dark
  versions through `prefers-color-scheme`.

**The Swift text path:**

- `PairingInvite.parse` (`client/Sources/SwitchboardProtocol/Info.swift:113-116`)
  first tries the text as an `https` URL. It takes `code`, `key` and
  `id` from the fragment (and from the query, for a hand-typed URL),
  and makes `addresses` the URL's `host:port`, with IPv6 bracketed.
  Otherwise it decodes the JSON as today.
- The usage line in `main.swift:12` becomes
  `pair '<invite url or json>'`.
- `client/README.md` gets the same change.

## The tests

**`tests/remote.rs`:**

- *Helpers:*
  - `http` (`245-296`) splits into `http_with(addr, config, method,
    path, headers: &[(&str, &str)], body) -> (u16, Vec<(String,
    String)>, Vec<u8>)`, with `http` over it.
  - `subscribe_at` (`1960-2042`) takes its headers from the caller, so
    a test can subscribe with `Cookie` and `Origin` in place of
    `Authorization`.
- `the_page_is_served_with_its_headers_and_names_only_served_paths`:
  - Each of `web::PATHS` answers 200 unpaired, with its content type,
    the exact CSP, `no-store`, `nosniff` and `no-referrer`.
  - Every `src=` and `href=` in `index.html` is one of `PATHS`.
  - No asset contains `http://` or `https://`, an inline `<script>`
    body, a `<style>` element, a `style=` attribute, an `on…=`
    attribute, or `innerHTML`.
  - Every `"/api/v1/…"` string in `app.js`, the page keeping them in
    one `API` table, is requested with a seeded project and session id
    put in for `{id}`, by the method the table names, under a paired
    cookie. None answers 404, so every path the page names is a route
    the host serves.
  - `/favicon.ico` answers 404.
  - An API reply carries `no-store`.
- `a_browser_pairs_by_cookie_reads_subscribes_and_sends`:
  - A `/pair` with `cookie: true` and a matching `Origin` answers 200
    with no `token` in the body. Its `Set-Cookie` has `__Host-`,
    `Path=/`, `Secure`, `HttpOnly`, `SameSite=Strict` and `Max-Age`.
  - `/device` with that cookie answers the scope and renews the
    cookie. With a bearer it sets no cookie.
  - `/projects` with the cookie reads.
  - A cookie subscription with `Origin` gets `hello`. Without
    `Origin`, or with another one, it is refused 403 `origin`.
  - Under `control`, a cookie `send` with `Origin` is typed once.
    Without `Origin` it is refused and nothing is typed.
  - A `/pair` with `cookie: true` and a foreign `Origin` is `403
    origin` and the window's failure count does not move.
  - After `RevokeDevice`, the cookie's next `/device` is 401 and the
    socket closes.
- `the_page_window_limits_page_loads_apart_from_info`: the 61st page
  load in a minute is 429 while `/info` still answers.
- `the_certificate_is_reissued_over_the_same_key_for_its_addresses`:
  - A legacy identity (no `certificate_names`) is reissued at bind,
    with the same `spki_sha256`, so the pinned test client still
    connects.
  - `certificate_names` holds `localhost` and `127.0.0.1`, and
    `created` moved.
  - A second bind does not reissue.

**`src/adapters/remote.rs` unit tests:**

- `read_request` takes the cookie out from among others, with
  `Origin` and `Host`.
- `respond` writes `Set-Cookie` and the two headers.
- `certificate` puts in the SANs, the EKU and a validity of 397 days,
  read back by `spki_sha256_of_cert` and by a rustls client that
  verifies the name `localhost` against the certificate as its own
  root.
- `certificate_current` is false at 367 days and for a missing name.

**Core tests (`src/core/tests.rs`):** `CertificateReissued` and
`pairing_url`, as above.

**UI test** (`tests/ui.rs:7424`): the invite assertion at `7476-7491`
parses a URL whose fragment carries the code, the key and the id. The
`Certificate …` label is found.

**Swift** (`swift test` in `client/`): `testInviteParsesTheURL` beside
`testInviteParsesTheQRText`
(`client/Tests/SwitchboardProtocolTests/FixturesTests.swift:157`), for
an IPv4 and a bracketed IPv6 URL.

**No UI test drives a browser.** The page's acceptance is by hand, on
a phone:

1. Open the pairing window and scan the QR.
2. Accept the warning after comparing the fingerprint.
3. See the sessions.
4. A session that starts waiting moves to the top.
5. A reply sent from the phone arrives in the pane.
6. Revoke the phone, and the page says so on its next request.

The design status records this run.

## Docs and the commit

- **`docs/remote-client.md`:**
  - §3.1: the certificate is reissued over the same key with its
    names, and `certificate_names`.
  - §4.1: the page window, and that the page's files are answered on
    the connection thread.
  - §4.3: rows for the three files, for `GET /api/v1/device`, and for
    cookie authentication with the `Origin` rule.
  - §5: the pairing rule's exceptions, the URL QR, browser trust as
    decision 1, the cookie.
  - A §4.8 "The page" with the page's flow.
  - A row in §13.
- **`docs/design.md`:**
  - A bullet "**A page in the browser (date)**" in the Remote client
    status, before "Known gaps" (`3573`), with spike 23's results.
  - A known gap: dispatch tickets, the conversation, drafts and runs
    are not on the page.
  - The trust boundary's remote sentence (`250-254`) adds that a
    browser trusts the host by the owner accepting its certificate
    warning, and holds its token as an `HttpOnly` cookie.
- **`README.md`:**
  - Layout lines for `web/` and `src/adapters/web.rs`.
  - The remote adapter's line gains the cookie, the page window and
    the reissue.
  - The `tests/remote.rs` line gains the page and cookie tests.
  - The `SWITCHBOARD_REMOTE_PORT` dev-aid paragraph (`267-279`): open
    `https://127.0.0.1:<port>/` on this Mac to see the page, and Copy
    invite now copies the URL, which `switchboard-remote pair` takes.
  - The data directory's `identity.json` line names
    `certificate_names`.
- **`spikes/23-browser-page/README.md`** with the measured results,
  and the spike index line in `spikes/README.md`.
- **`CHANGELOG.md`, under Unreleased:** "A browser on the local network
  can open this Mac's page from the pairing window's QR code: the
  sessions with the waiting ones first, a session's last response or
  screen, and a reply under control scope."

Commit message:

```
A web control surface: the remote port serves a browser page, paired from a URL that carries the code
```

Contract additions, for the commit message:

- `AppAction::CertificateReissued(Identity)` (`src/core/action.rs`).
- `Identity::certificate_names: Vec<String>` (`src/ports/remote.rs`,
  `#[serde(default)]`), with `IDENTITY_SCHEMA_VERSION` set to the
  value at implementation time plus one.
- No `Effect` and no port trait method. `model.rs` is unchanged and
  `SCHEMA_VERSION` (16) and `VIEWS_SCHEMA_VERSION` (7) stand.
- On the wire (additive):
  - `GET /api/v1/device`;
  - `cookie: true` on `/pair`, with its token-less reply and
    `Set-Cookie`;
  - authentication by the `__Host-switchboard` cookie under the
    `Origin` rule;
  - `"device"` in `info.capabilities`;
  - `Cache-Control: no-store` and `nosniff` on every reply;
  - the static paths `/`, `/app.js`, `/app.css`.
- Swift: `PairingInvite.parse` accepts the pairing URL.
