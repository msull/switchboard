# Web control surface (issue #230)

The remote port also serves a browser page. A phone, an iPad or another
Mac on the local network opens it from the pairing window's QR code,
pairs, lists the sessions with the waiting ones first, reads a session's
last response or its screen, and replies. The browser is a paired device
like the headset. It speaks the same `/api/v1` protocol, and its token is
an `HttpOnly` cookie its script never sees.

Line numbers below are from this worktree at `7fe8949`.

## Where this departs from the issue

- **The card's one-line excerpt is not on the wire.** `SessionView`
  (`control/src/lib.rs:637`) carries the card word, `waiting`,
  `waiting_reason` and `kind`, but no excerpt. The Mac computes the
  excerpt from the transcript (`agent_excerpt`, `src/ui/cards.rs:558`).
  The headset fetches `last-response` for each row
  (`client/Sources/SwitchboardUI/BrowserView.swift`), and the page does
  the same. An agent row shows the first line of its `last-response`.
  Every other row shows its card word. This needs no new read model.
- **No reply tells a device its current scope.** The owner can change a
  scope in the dialog at any time. The headset only learns of the change
  from a 403 `scope`. The page has to show the scope in its header and
  hide the reply box to match, so the plan adds one read,
  **`GET /api/v1/device`**, answering `{device_id, name, scope}`. This is
  the "unless the page cannot do without one" case.
- **The certificate as it is made today may not be one a browser lets
  the owner click through.** `make_identity`
  (`src/adapters/remote.rs:95-127`) builds it with no SANs
  (`CertificateParams::new(Vec::<String>::new())`, line 102), no EKU, and
  rcgen's default `not_after` in the year 4096. Apple's TLS policy wants
  a SAN, the serverAuth EKU and at most 825 days. Chrome on macOS is
  reported to show non-bypassable errors for certificates that break it.
  So the plan reissues the certificate from the **same key**, which §5
  already allows ("a certificate reissued from the same key still
  verifies"). The pin does not move.
- **`SameSite=Strict` does not keep other local services out.** Cookies
  are scoped by host, not by port, and for an IP address the site is the
  IP alone. Any other HTTPS service on the same Mac is therefore
  same-site, and WebSocket handshakes are not covered by CORS. The plan
  adds a rule: a cookie counts only on a request the page itself made
  (see Signals).
- **The host does not parse the user agent.** The page builds the device
  name from `navigator.userAgent` (for example "Safari on iPhone") and
  sends it as `device_name`, as every client does today
  (`src/app.rs:3650-3655`).
- **No fixture is added.** No Swift view changes, and nothing in Swift
  reads `/device`. The Rust test asserts that shape.
- **The spike is part of the one commit.** CLAUDE.md asks for a spike
  before building on a risky mechanism. `spikes/23-web-surface` (the next
  free number, 23 today) is the first step of the work, and its README
  lands in the same commit. It confirms the choices below and records the
  manual acceptance run. The choices do not wait on it, except where a
  fallback is named.

## Decisions

1. **Trust: the click-through is the trust step.** The host does not
   offer its certificate for download. On iOS a download means installing
   a profile and then turning on a second switch, and it leaves a trust
   anchor on the phone. A download path would also be one more thing
   served before pairing, and against an attacker at first contact it
   buys no more than the click-through.

   What the URL's `key` buys: the genuine page refuses to post the code
   when `/info` presents a different key. That covers a stale QR and a
   URL aimed at the wrong instance.

   What it does not buy: an impostor that answers the first visit serves
   its own page, reads the code and the key from the URL, and relays
   `/pair` to the real host with the real key. The host's key check
   (`src/app.rs:3636`) passes. For a browser, first contact is therefore
   trust on first use, which is weaker than the native pin. After that
   first visit the browser holds an exception for that one certificate,
   so a later warning means something else is answering.

   The dialog says this in one sentence: "Accept the browser's
   certificate warning once, when you pair. A warning at any other time
   means something else may be answering: stop, and pair again from this
   window." A renewal (below) also brings a warning, and pairing again is
   the right answer to that one too.

   §5's rule becomes: "Require pairing before anything but `/info`,
   `/pair` and the page's four fixed files."
2. **Token: an `HttpOnly` cookie set by `/pair`.**
   - Name: `__Host-switchboard-<first 8 hex of server_id>`. The id is in
     the name because two instances on one Mac share one cookie jar per
     IP.
   - Attributes: `Secure; HttpOnly; SameSite=Strict; Path=/;
     Max-Age=34560000` (400 days, Chrome's cap). `GET /api/v1/device`
     sends the cookie again, so the 400 days run from the last visit.
   - Why a cookie: no script can read it, and the page renders text an
     agent printed. Safari clears script-written storage after seven days
     without a visit, but keeps a cookie the server set. `new
     WebSocket()` carries it.
   - The bearer header keeps working for the native clients and is
     checked first.
3. **One QR: the URL**
   `https://<address>:<port>/?code=<code>&key=<spki_sha256>&id=<server_id>`.
   - Both the QR and "Copy invite" carry it.
   - `PairingInvite.parse` takes the URL, and still takes the JSON for an
     invite copied from an older host. The host no longer produces the
     JSON.
   - The address is the LAN address when `identity.listen.lan` is set.
     Otherwise it is `127.0.0.1`, so the command-line client on the same
     Mac can still pair, and the window says what to turn on.
4. **Static paths.** Exactly four:

   | Path | `Content-Type` |
   |---|---|
   | `/` | `text/html; charset=utf-8` |
   | `/app.js` | `text/javascript; charset=utf-8` |
   | `/app.css` | `text/css; charset=utf-8` |
   | `/icon.svg` | `image/svg+xml` |

   - All four are `GET` only, and any query is ignored.
   - The connection thread answers them from embedded bytes. They never
     reach the frame, so they cost no frame time and no inbox slot.
   - `/favicon.ico`, `/index.html` and every other path stay 404.
   - Headers on all four:
     - `Cache-Control: no-store`
     - `X-Content-Type-Options: nosniff`
     - `Referrer-Policy: no-referrer`
     - `X-Frame-Options: DENY`
     - `Connection: close`
     - `Content-Security-Policy: default-src 'none'; script-src 'self';
       style-src 'self'; img-src 'self'; connect-src 'self' wss://<Host>;
       base-uri 'none'; form-action 'none'; frame-ancestors 'none'`.
       `<Host>` is the request's `Host` header when it is only
       `[A-Za-z0-9.:\[\]-]`, and is left out otherwise. It is written out
       because older Safari did not match `wss:` to `'self'`.
   - API replies also gain `nosniff`, `no-store` and `no-referrer`.
   - **Pool:** a page load counts against the unauthenticated pool of 4
     (§4.1), even for a paired browser. A connection moves pools only on
     an authenticated reply, and a static reply closes its connection, so
     each file holds a slot for one file's time.
   - **Rate:** the four paths have a window of their own,
     `PAGE_PER_MINUTE = 60` per listener, beside `/info`'s 60.
   - What a page opened without a code or a token shows is in the UI
     section.
5. **The page lives in `web/`** and is embedded with `include_str!` by a
   small `src/adapters/web.rs`. It is tested from `tests/remote.rs` with
   the test's own TLS client (see Tests).
6. **Device record:** unchanged. A browser is a `PairedDevice`, named by
   the page, given the scope chosen in the pairing window, and listed and
   revoked in the dialog like any other device.

## The record

**No record changes.** The schema versions today are:

| Constant | Value | Where |
|---|---|---|
| `SCHEMA_VERSION` | 16 | `src/core/model.rs:12` |
| `VIEWS_SCHEMA_VERSION` | 7 | `src/core/model.rs:612` |
| `IDENTITY_SCHEMA_VERSION` | 1 | `src/ports/remote.rs:17` |

None is bumped.

**`identity.json`'s `certificate` field gets new bytes in the same
shape.**
- Today's form is replaced once, on the first listen after this change.
- The new certificate is then renewed whenever it is within 30 days of
  its `not_after`.
- `spki_sha256` does not change, so `devices.json` is untouched and every
  native pin still verifies.
- The write goes through the existing `save_identity`
  (`src/ports/remote.rs:113`).

**On the device:**
- The cookie holds the token.
- `localStorage` holds a non-secret marker,
  `{server_id, spki_sha256, device_id, scope, server_name}`, used only to
  tell three states apart: never paired, revoked, and identity changed.
- Every read and write of the marker is in try/catch. A missing marker
  reads as "not paired", which is safe.

**Rehydration is a page load:**
1. `/info`, compared with the marker.
2. `/device`.
3. Open `/api/v1/events` and buffer its events until `hello`.
4. Snapshot `waiting`, `projects`, and each project's `sessions`.
5. Apply the buffered events by revision (§4.4).

The page keeps nothing else, and a reload is the reconnect.

## The signals

**`RemoteRequest`** (`src/adapters/remote.rs:360-372`) gains four fields:
- `web_tokens: Vec<(String, String)>`: only the cookies whose name starts
  with `__Host-switchboard-`, so other services' cookies are dropped at
  the parse.
- `origin: Option<String>`
- `host: Option<String>`
- `web: bool`: an `X-Switchboard-Web: 1` header was present.

`read_request` (444) fills them next to the `bearer` extraction (487).
The tokens get the same treatment as `bearer`: never logged. `refuse`
(`src/app.rs:3754`) logs only the method and the path, and the query is
never logged. That keeps the page's `?code=` out of the log.

**The page rule.** A cookie token counts only on a request the page
itself made:
- A WebSocket upgrade must carry `Origin` equal to `https://<Host>`.
- Any other request must carry `X-Switchboard-Web: 1`, and its `Origin`,
  if present, must match. A page on another origin cannot add that header
  without a CORS preflight, which the host never answers.

A cookie that fails the rule gets a 403 `cross_origin` and no
`DeviceSeen`.

**Host-side meaning of the new requests:**

| Request | Effect |
|---|---|
| `GET /api/v1/device` | Answers the device's current scope. |
| `POST /api/v1/pair {"cookie": true, ...}` | Same `Paired` transition. The token goes in `Set-Cookie`, not the body. |
| A 401 on a request that carried our cookie | Answers `Set-Cookie: <name>=; ...; Max-Age=0`, so a revoked browser forgets the token. |

**The certificate.** `make_identity` and a new
`reissue_if_due(identity, key_pem, now) -> Result<Option<Identity>, String>`
share one params builder:
- the same CN;
- SAN `localhost` (an IP SAN would force a reissue every time the address
  changes, and a name mismatch is a warning the owner can click through);
- EKU serverAuth;
- `not_before` one day before now;
- `not_after` 820 days after now, inside Apple's 825.

`certificate_not_after(der)` reads the validity with the existing DER
walker (`der_element`, `src/adapters/remote.rs:268`, as
`spki_sha256_of_cert` at 247 does). A certificate is due when its
`not_after` is unreadable, more than 825 days away, or under 30 days
away. rcgen's `KeyPair::from_pem` signs with the Keychain's key. Dates
reach rcgen through `rcgen::date_time_ymd` from `chrono`, which is
already a dependency, so `time` is not added.

## Actions and effects

**No new `AppAction` or `Effect`.** Pairing is the existing
`PairingFailed` and `Paired`, revocation is `RevokeDevice`, and the
`DeviceSeen` behaviour stays as it is (`src/core/remote.rs`).

The work is in the adapter and the frame.

**`src/adapters/remote.rs`**
- `RemoteReply` (389-394) gains `set_cookie: Option<String>`. Every
  literal gains `set_cookie: None`: about 30, almost all in `src/app.rs`.
- `respond` (530-553) becomes `respond_with(w, status, content_type,
  headers, body, close)`. `respond` stays as the JSON wrapper. It adds the
  API's three headers, plus `Set-Cookie` when the reply carries one.
- `Limits` (623-639) gains `page: Mutex<RateWindow>`, keyed on the four
  paths and built beside `pair` and `info` (684-685).
- In `connection` (770-876), right after the 429 check (787):
  1. `if let Some(file) = web::static_file(&request.method,
     &request.path)`
  2. write it with its headers;
  3. `return Ok(())`.
- `make_identity`'s doc comment (90-94) stops saying "never expires".

**`src/adapters/web.rs`** (new)
- `pub fn static_file(method, path) -> Option<StaticFile>`, where
  `StaticFile` is `{content_type, body: &'static [u8]}`.
- `pub fn headers(host: Option<&str>) -> Vec<(&'static str, String)>`,
  which builds the CSP.
- The four `include_str!("../../web/...")` constants.

**`src/app.rs`**
- **The identity load** (2799-2832): in the arm where the key matches and
  the instance is not read-only, call `remote::reissue_if_due` with the
  wall clock. If it returns a new identity, `save_identity` it, log
  "remote certificate renewed", and continue with the new identity. A
  failure is logged and the old certificate is kept.
- **`serve_remote`** (3017):
  - The prefix match at 3023 stays. Static paths never get here.
  - After `authenticate` (3032-3035), add
    `("GET", ["device"]) => return Served::Now(self.remote_device(&device, request))`.
    The handler answers `{device_id, name, scope}` in the envelope and
    sets the cookie again when the request came by cookie.
- **`authenticate`** (3127-3149):
  - Try `bearer` first.
  - Otherwise use the `web_tokens` entry with this installation's cookie
    name, under the page rule; a failure is a 403 `cross_origin`.
  - A 401 on a request that carried that cookie also sets the clearing
    cookie.
- **`remote_pair`** (3606-3673):
  - When the body has `"cookie": true`, require the page rule's `Origin`
    match before anything is counted. A mismatch is a 403
    `cross_origin`, and no `PairingFailed` is counted, because the code
    was not tried.
  - The reply (3660-3665) then leaves out `token` and carries `set_cookie`
    instead.
- **`remote_info`** (3580): the capabilities gain `"web"`.

**`src/core/remote.rs`**
- A pure `pub fn invite_url(address: &str, identity: &Identity, code: &str)
  -> String`. Every part of it is hex, a UUID or Crockford, so it needs
  no percent-encoding.

**Swift** (`client/Sources/SwitchboardProtocol/Info.swift:92-117`)
- `PairingInvite.parse` tries `URLComponents` first: it needs an `https`
  scheme and a host, a port and `code`, `key` and `id`, which fill
  `addresses: ["host:port"]`, `spkiSha256`, `code` and `serverID`.
- Otherwise it falls back to the JSON decode at line 114.
- The command-line `pair` (`main.swift:107-125`) and both headset paths
  (`BrowserView.swift:106`, `:883`) call `parse`, so they get the URL form
  with no other change.
- The usage line (`main.swift:13`) becomes `pair '<invite url or json>'`.

## The UI

### The dialog (`src/ui/remote.rs`)

- The payload (155-165) becomes `core::remote::invite_url(...)`. The
  address is `local_address()` when `identity.listen.lan` is set, and
  `127.0.0.1` otherwise.
- The QR texture cache (166-173) is unchanged, still keyed by code.
- "Copy invite" (199-206) copies the URL. Its hover text becomes "The QR
  code's link, to open on a device or paste into a client".
- The hint (191-197) gains the trust sentence from decision 1.
- When `lan` is off, the hint gains: "Turn on 'Allow devices on the local
  network' (above) to pair a phone or another computer." The checkbox is
  at 110-112.
- The devices list (230-276) is unchanged: a browser shows its name, for
  example "Safari on iPhone".

### The page (`web/`)

**Files**
- `index.html` holds the structure only: no inline script, no `style=`.
- `app.js` is plain ES2020 in one file.
- `app.css` uses `prefers-color-scheme` for light and dark.
- `icon.svg`.
- No framework, no build step, no dependency.
- Text is set with `textContent` only and never with `innerHTML`, because
  agent output is untrusted.
- Every fetch sends `X-Switchboard-Web: 1`.
- The `op` for a send comes from `crypto.getRandomValues`. That does not
  need `isSecureContext` (`randomUUID` does), so the click-through cannot
  break it.

**Load**
1. The first statement of `app.js` reads the query and runs
   `history.replaceState(null, "", "/")`.
2. Then `GET /api/v1/info`.
3. If a code came in the query, compare `/info`'s `spki_sha256` and
   `server_id` with `key` and `id`:
   - On a mismatch, say "This is not the host whose code you scanned" and
     send nothing.
   - On a match, post `/pair {code, spki_sha256, device_name, cookie:
     true}`, write the marker, and continue.
4. Otherwise compare `/info` with the marker. If the key differs, say
   "<host>'s identity changed. Pair again from its Remote devices window"
   and send nothing more.
5. `GET /api/v1/device`:
   - 200: header and scope; rebuild the marker if it was lost.
   - 401 with a marker: "This device was revoked on <host>. Pair again".
   - 401 without a marker: "Open Remote devices on the Mac, choose Allow
     pairing, and scan its code with this device's camera".
6. Subscribe, snapshot, then apply the buffered events by revision.
7. `pairing_closed`, `wrong_code`, `wrong_key`, `rate_limited` and
   `cross_origin` are each said in words.

**List**
- Projects are headings, and each holds its sessions.
- Waiting sessions sort first, both inside a project and among projects,
  and are marked with their `waiting_reason`.
- Each row shows the kind, the card word and the excerpt (see the first
  departure).
- `last-response` is fetched one at a time in list order, at most 2 in
  flight. It is fetched again when a `changed` event names that session's
  conversation key.
- A `changed` event on `waiting` or a project's `sessions` refetches that
  snapshot.
- A session that starts waiting therefore moves up without a reload.

**Session view**
- An agent shows its `last-response`, refreshed on `changed`.
- Anything else shows `screen?lines=60` under a `watch` on the socket,
  refreshed on `changed`, with an `unwatch` when the view closes.
- The reply box (a textarea and Send) appears only when `/device` says
  `control`:
  - It posts `/sessions/{id}/send {text, op}`.
  - A 403 `scope` refetches `/device` and hides the box.
  - A repeat of the same op is answered from the log, as today.

**Reconnect**
- On a socket close, `/device` first: a 401 shows "revoked".
- Otherwise reconnect after 1, 2, 4 … 30 s.
- `visibilitychange` to visible reconnects at once.
- A new `epoch` refetches everything, and `resync_required` reconnects.

## The tests

All run under `cargo test --locked --workspace`, except the Swift ones.

**`src/core/tests.rs`**
- `the_invite_url_carries_the_code_key_and_id`.

**`src/adapters/remote.rs` unit tests**
- `a_request_reads_its_web_headers`: only `__Host-switchboard-*` cookies
  are kept, and `origin`, `host` and `web` are read.
- `a_reissued_certificate_keeps_the_key`: the SPKI hash is unchanged,
  `not_after` is within 825 days, the pinned client still connects, and a
  fresh one is not due.
- `todays_certificate_form_is_due`: a certificate built with today's
  params is due, and so is one 20 days from expiry.
- `the_page_window_answers_429_past_its_limit`.

**`tests/remote.rs`**

First, extend `http()` (245) to take extra request headers and return the
response headers. `call` and `get` (299, 327) keep their signatures over
it.

- `the_page_and_its_files_are_served_with_their_headers`:
  - each of the four paths answers 200 with its type and every header from
    decision 4, and `Connection: close`;
  - `/favicon.ico`, `/index.html` and `POST /` answer 404;
  - `/?code=X` serves the page and leaves the pairing window open with no
    failure counted.
- `the_page_names_only_paths_the_host_serves`:
  - scan `index.html` for `src=`/`href=`; each must be served by
    `web::static_file`;
  - scan `app.js` for `/api/v1/` literals; each, with ids filled in, must
    not answer 404 with a paired cookie;
  - `index.html` has no inline `<script>` body and no `style=`.
- `before_pairing_a_lan_bind_serves_only_info_pair_and_the_page`: extends
  the model at 426.
- `a_browser_pairs_by_cookie_and_reads_with_it`:
  - with the window open, `/pair {"cookie":true}` with a matching
    `Origin` returns `Set-Cookie` with every attribute and a body with no
    `token`;
  - `/projects` with the cookie and `X-Switchboard-Web` returns 200;
  - `/device` returns the scope and sets the cookie again;
  - the events upgrade with the right `Origin` gets `hello`;
  - a bearer device beside it still reads.
- `a_cookie_off_the_page_is_refused`:
  - no `X-Switchboard-Web`, a wrong `Origin`, or an upgrade with a wrong
    or missing `Origin` each answer 403 `cross_origin`;
  - `last_seen` does not move;
  - `/pair {"cookie":true}` with a wrong `Origin` is a 403 that counts no
    failure.
- `a_revoked_browser_is_told_and_forgets_its_cookie`: follows the model
  at 658. After `RevokeDevice` the socket closes, and the next `/device`
  is a 401 whose `Set-Cookie` carries `Max-Age=0`.
- `a_scope_change_shows_on_device`: read, then control after
  `SetDeviceScope`.
- `info_lists_the_web_capability`.

**`tests/ui.rs`**
- `remote_devices_dialog_opens_pairing_and_revokes_a_device` (7424): the
  invite assertion (7486-7492) parses a URL instead of JSON. It asserts an
  `https://127.0.0.1:` prefix, since the fixture's `Listen::default()` is
  loopback, then the `code=`, `key=` and `id=` values, then the
  turn-on-the-local-network hint.

**Swift** (`client/Tests/SwitchboardProtocolTests/FixturesTests.swift`)
- Add `testInviteParsesTheInviteURL` beside `testInviteParsesTheQRText`
  (156), which stays as the JSON fallback.
- A URL with no `code` throws.
- Run with `(cd client && swift test)`.

**Manual: `spikes/23-web-surface/README.md`**

The README records each item below with its numbers. If a measurement
fails, the named fallback is taken in the same commit.

- **Certificate click-through.** `serve.sh` makes a certificate in
  today's form and one in the reissued form with `openssl`, and serves
  each with `openssl s_server -www` on a LAN port. Record whether Safari
  on iOS, Safari on macOS and Chrome on macOS offer a click-through for
  each, and `isSecureContext` on the page that results.
  - Fallback: if the reissued form cannot be clicked through in a
    browser, the plan stops for that browser and says so.
- **Cold page load against the pool of 4.** Measured with the built app,
  `SWITCHBOARD_DATA_DIR`, and `remote-listen lan <port>`.
  - Fallback: drop `icon.svg` and its `<link>`, which leaves three files.
- **Whether the exception holds** across a reload, a relaunch, and an
  address change.
- **The acceptance run:**
  1. Scan the code and accept the warning.
  2. See the sessions.
  3. A session starts waiting and moves to the top.
  4. Send a reply from the phone and see it arrive.
  5. Revoke the phone; the page says so on its next request.

## Docs in the same commit

**`docs/remote-client.md`**
- §4.1: the page pool and its window.
- §4.3: the `/device` row and the four static rows.
- §5:
  - the pairing rule now includes the page's files;
  - the browser-trust paragraph from decision 1;
  - the cookie and the page rule;
  - "never in a URL" stays;
  - the URL invite replaces the JSON.

**`docs/design.md`**
- In the Remote client status (from line 3107): the web surface is built,
  with the spike's results.
- The known gaps:
  - tickets, drafts, runs and pinned files are not on the page;
  - first contact for a browser is trust on first use.

**`README.md`**
- The Layout table gains `web/` and `src/adapters/web.rs`.
- The `switchboard-remote pair "$(pbpaste)"` note (line 277) says the
  clipboard holds the URL.

**`spikes/README.md`**
- Row 23.

**`CHANGELOG.md`**
- Under Unreleased: "Open Switchboard in a browser on your local network:
  scan the pairing code with a phone to see your sessions, which ones are
  waiting, and reply."

## Commit

`Serve a browser control page on the remote port, paired from a URL in the QR code`

Contract additions:
- `src/core/model.rs`: none.
- `src/core/action.rs`: none.
- `src/ports/`: none.
- `src/core/remote.rs`: `invite_url`.
- Wire, additive:
  - `GET /api/v1/device` → `{device_id, name, scope}` (read scope);
  - `/pair` body field `cookie: bool`; with it, the reply has no `token`
    and sets `__Host-switchboard-<8 hex of server_id>`;
  - `/info` capability `"web"`;
  - error code `cross_origin` (403);
  - request header `X-Switchboard-Web: 1`;
  - static `GET /`, `/app.js`, `/app.css`, `/icon.svg`.
- Adapter, additive:
  - `RemoteRequest.{web_tokens, origin, host, web}`;
  - `RemoteReply.set_cookie`;
  - `remote::{reissue_if_due, certificate_not_after}`;
  - `adapters::web::{static_file, headers}`.
- Swift: `PairingInvite.parse` accepts the URL form.
