# Web control surface (issue #230)

The remote port also serves a browser page. A phone, an iPad or another
Mac on the local network opens it from the pairing window's QR code,
pairs, lists the sessions with the waiting ones first, reads a session's
last response or its screen, and replies. The browser is a paired device
like the headset and speaks the same `/api/v1` protocol. Its token is
split in two: an `HttpOnly` cookie its script never sees, and a key held
in the page's own origin storage. A request needs both.

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
  verifies"). The pin does not move. A certificate that expires needs
  renewing while the app runs, so renewal is a daily check, not only a
  startup step.
- **A cookie alone cannot be the credential.** Cookies are scoped by
  host, not by port. The browser sends the cookie to every HTTPS service
  on the Mac's address, at any port, and Chrome also sends `Secure`
  cookies over plain HTTP to loopback. Whatever receives it can replay it
  from a non-browser client with any headers it likes, and a `control`
  token can type into agent sessions. So the token is split:
  - the cookie half is `HttpOnly`;
  - the key half lives in `localStorage`, which is scoped to scheme, host
    **and port**, and goes as a header (or, on the WebSocket, a
    subprotocol value).

  The device's `token_sha256` is the hash of both halves together, so a
  leaked cookie is useless alone. It also makes an Origin check
  unnecessary: a page on another origin cannot read the key half, so it
  cannot forge a request either.
- **An address in the invite URL makes a DHCP change unpair the
  browser.** The cookie, the key half and the certificate exception are
  all keyed to the URL's host. So the invite uses the Mac's
  `<LocalHostName>.local` name, which macOS's own mDNS responder answers
  for, and which iOS and macOS resolve. The LAN IP is used when there is
  no such name: off macOS, or when the read fails.
  - The name comes from `LocalHostName` (`scutil --get LocalHostName`),
    not from `hostname -s`.
  - `hostname -s` gives the short `HostName` when one is set, by the
    owner or by DHCP or reverse DNS, and that can differ from the name
    mDNS answers.
  - The Bonjour record (`src/adapters/bonjour.rs:41`) proves nothing
    here. It registers `<hostname -s>.local.` with an empty address list,
    so it publishes no A record of its own.
- **The invite stays readable by clients built before this change.** The
  QR and "Copy invite" carry the URL. A second button, "Copy JSON
  invite", keeps today's `{server_id, spki_sha256, code, addresses}` text
  for an older `switchboard-remote` or headset build, so the shared
  contract changes additively.
- **The host answers browser probes on the connection thread.** A browser
  asks for `/favicon.ico` and `/apple-touch-icon*.png` on its own. So
  every path outside `/api/v1/` is answered on the connection thread
  (one of four files, or a 404) and never reaches the frame.
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
   so a later warning means something else is answering. The one
   expected exception is right after a renewal.

   The dialog shows the certificate's issue date ("Certificate issued
   <date>") and says in one sentence: "Accept the browser's certificate
   warning when you pair, and once after the certificate is renewed. A
   warning at any other time means something else may be answering:
   stop, and pair again from this window."

   §5's rule becomes: "Require pairing before anything but `/info`,
   `/pair` and the page's four fixed files."
2. **Token: two halves, a cookie and an origin-scoped key, required
   together.**
   - `/pair` with `"cookie": true` mints two fresh `bearer_token()`
     values, `c` and `k`. It stores `token_sha256 = sha256(c + "." + k)`
     through the existing `new_device`, so the record shape does not
     change. It answers `k` in the body as `web_key`, and `c` as the
     cookie.
   - Cookie name: `__Host-switchboard-<first 8 hex of server_id>`. The id
     is in the name because two instances on one Mac share one cookie jar
     per host.
   - Cookie attributes: `Secure; HttpOnly; SameSite=Strict; Path=/;
     Max-Age=34560000` (400 days, Chrome's cap). `GET /api/v1/device`
     sends the cookie again, so the 400 days run from the last visit.
   - The page keeps `k` in `localStorage` with the marker.
   - `k` travels as `X-Switchboard-Key: <k>` on every fetch. On the
     WebSocket it is a subprotocol: `new WebSocket(url,
     ["switchboard.v1", "sbk." + k])`. The host accepts the upgrade with
     `Sec-WebSocket-Protocol: switchboard.v1`. The spec's "never in a
     URL" holds.
   - What each half protects against:
     - a stolen cookie (another service on the address, a loopback HTTP
       listener) lacks `k`;
     - a page on another origin cannot read `k`;
     - another service that overwrites the cookie (cookie tossing) can
       only make this browser's requests fail, and the page then says to
       pair again;
     - script on our own origin could read `k`, but it could already make
       any request, so `textContent`-only rendering is the defence there.
   - Why still a cookie: it keeps half the token out of every script, and
     `new WebSocket()` carries it.
   - The bearer header keeps working for the native clients and is
     checked first.
   - The cost, stated in §5 and the known gaps: Safari clears
     script-written storage after seven days of browsing without a visit
     to the page, and a home-screen web app is exempt. A browser that
     loses `k` must pair again, and the page says why.
   - Scope stays what the owner chose in the pairing window (`read` by
     default, as today), because a leaked cookie no longer carries it.
3. **One QR: the URL**
   `https://<host>:<port>/?code=<code>&key=<spki_sha256>&id=<server_id>`.
   - The QR and "Copy invite" carry it. "Copy JSON invite" keeps today's
     JSON for older clients (see departures).
   - `PairingInvite.parse` takes the URL, and still takes the JSON.
   - `<host>` depends on the listen setting:
     - `lan` on: `remote::local_host_name()` plus `.local` when the name
       is one DNS label (`[A-Za-z0-9-]{1,63}`); otherwise the LAN IP from
       `local_address()`;
     - `lan` on and neither available: no QR and no URL (see the UI);
     - `lan` off: `127.0.0.1`, so the command-line client on the same
       Mac can still pair, and the window says what to turn on.
4. **Static paths.** Every path outside `/api/v1/` is answered on the
   connection thread from embedded bytes and never reaches the frame, so
   none costs frame time or an inbox slot. Four of them are files:

   | Path | `Content-Type` |
   |---|---|
   | `/` | `text/html; charset=utf-8` |
   | `/app.js` | `text/javascript; charset=utf-8` |
   | `/app.css` | `text/css; charset=utf-8` |
   | `/icon.svg` | `image/svg+xml` |

   - All four are `GET` only, and any query is ignored.
   - Everything else outside `/api/v1/` is a 404 with the JSON error
     body, including `/favicon.ico`, `/apple-touch-icon*.png`,
     `/index.html` and `POST /`.
   - Headers on all of these replies:
     - `Cache-Control: no-store`
     - `X-Content-Type-Options: nosniff`
     - `Referrer-Policy: no-referrer`
     - `X-Frame-Options: DENY`
     - `Connection: close`
   - The four files also carry `Content-Security-Policy: default-src
     'none'; script-src 'self'; style-src 'self'; img-src 'self';
     connect-src 'self' wss://<Host>; base-uri 'none'; form-action
     'none'; frame-ancestors 'none'`.
     - `<Host>` is the request's `Host` header when it is only
       `[A-Za-z0-9.:\[\]-]`, and is left out otherwise.
     - It is written out because older Safari did not match `wss:` to
       `'self'`.
   - API replies also gain `nosniff`, `no-store` and `no-referrer`.
   - **Pool:** a page load and its probes count against the
     unauthenticated pool of 4 (§4.1), even for a paired browser. A
     connection moves pools only on an authenticated reply, and these
     replies close their connection, so each holds a slot for one file's
     time.
   - **Rate:** paths outside `/api/v1/` have a window of their own,
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

**No record change.** The schema versions today are:

| Constant | Value | Where |
|---|---|---|
| `SCHEMA_VERSION` | 16 | `src/core/model.rs:12` |
| `VIEWS_SCHEMA_VERSION` | 7 | `src/core/model.rs:612` |
| `IDENTITY_SCHEMA_VERSION` | 1 | `src/ports/remote.rs:17` |

None is bumped. A browser's `token_sha256` hashes both halves of its
token in the existing field.

**`identity.json`'s `certificate` field gets new bytes in the same
shape** whenever the certificate is due (see Signals):
- at `listen_remote`;
- by a daily check from `Tick` while the app runs.

Today's certificate is due on the first check after this change.
`spki_sha256` does not change, so `devices.json` is untouched and every
native pin still verifies. The write is the existing `update_identity` →
`Effect::SaveIdentity` path (`src/core/remote.rs:105-116`).

**On the device:**
- The cookie holds the token's first half.
- `localStorage` holds the key half and a marker,
  `{server_id, spki_sha256, device_id, scope, server_name, web_key}`. The
  marker tells four states apart: never paired, revoked, key lost, and
  identity changed.
- Every read and write is in try/catch. A missing entry reads as "not
  paired", which is safe.

**Rehydration is a page load:**
1. `/info`, compared with the marker.
2. `/device`.
3. Open `/api/v1/events` and buffer its events until `hello`.
4. Snapshot `waiting`, `projects`, and each project's `sessions`.
5. Apply the buffered events by revision (§4.4).

The page keeps nothing else, and a reload is the reconnect.

**Known gap: a stale device record.** A browser that loses its storage,
or whose Mac is renamed so the `.local` name changes, must pair again.
Its old `PairedDevice` stays in the dialog with its "seen <time>" until
the owner revokes it. The token it held is half in a cookie for an
address no longer used, and useless without the other half.

## The signals

**`RemoteRequest`** (`src/adapters/remote.rs:360-372`) gains four fields:
- `web_cookie: Vec<(String, String)>`: name and value of each
  `__Host-switchboard-*` cookie. Other cookies are dropped at the parse,
  and the frame picks the one named for this installation.
- `web_key: Option<String>`: `X-Switchboard-Key`, or the `sbk.` entry of
  `Sec-WebSocket-Protocol`.
- `ws_protocol: bool`: `switchboard.v1` was offered, so the 101 must echo
  it.
- `host: Option<String>`: for the CSP.

`read_request` (444) fills them next to the `bearer` extraction (487).
The cookie and key get the same treatment as `bearer`: never logged.
`refuse` (`src/app.rs:3754`) logs only the method and the path. Paths
outside `/api/v1/` are answered on the connection thread, which logs
nothing, so the page's `?code=` never reaches a log.

**Authentication:**

| Request | Result |
|---|---|
| bearer | as today |
| this installation's cookie and a key | `device_by_token(sha256(c + "." + k))`, else 401 `unauthorized` |
| this installation's cookie and no key | 401 `key_missing` |

A 401 on a request that carried our cookie also answers `Set-Cookie:
<name>=; Secure; HttpOnly; SameSite=Strict; Path=/; Max-Age=0`.

**New requests:**

| Request | Effect |
|---|---|
| `GET /api/v1/device` | Answers the device's current scope and, for a cookie request, sets the cookie again. |
| `POST /api/v1/pair {"cookie": true, ...}` | The same checks and the same `Paired` transition. The body carries `web_key` instead of `token`, and the reply sets the cookie. |

**The certificate.** `make_identity` and a new
`reissue_if_due(identity, key_pem, now) -> Result<Option<Identity>, String>`
share one params builder:
- the same CN;
- SAN `localhost` and, when `local_host_name()` gives one, `<name>.local`
  (no IP SAN: it would need a reissue for every address change);
- EKU serverAuth;
- `not_before` one day before now;
- `not_after` 820 days after now, inside Apple's 825.

`certificate_validity(der) -> Option<(SystemTime, SystemTime)>` reads
`not_before` and `not_after` with the existing DER walker (`der_element`,
`src/adapters/remote.rs:268`, as `spki_sha256_of_cert` at 247 does). A
certificate is due when its validity is unreadable, its `not_after` is
more than 825 days away, or it is under 30 days away. The due check
reads only the public certificate. The Keychain key is read only when it
is due. rcgen's `KeyPair::from_pem` signs with that key. Dates reach
rcgen through `rcgen::date_time_ymd` from `chrono`, which is already a
dependency, so `time` is not added.

`local_host_name()` reads `LocalHostName`:
- On macOS it runs `scutil --get LocalHostName` once and keeps the result
  in a `OnceLock`. The call is under `cfg(target_os = "macos")`, so no
  `unsafe` and no new dependency.
- Elsewhere, or when the command fails or the result is not one DNS
  label, it returns `None` and the invite falls back to the LAN IP.

`default_server_name` (`src/adapters/remote.rs:304`) keeps `hostname -s`
for the display name. The Bonjour host at `src/adapters/bonjour.rs:41` is
unchanged here (see the known gaps).

## Actions and effects

**Additions to the contract:**
- `AppAction::CertificateRenewed { certificate: String }`. The core runs
  `update_identity(|i| i.certificate = certificate)`, which emits the
  existing `Effect::SaveIdentity`. Its arm goes beside `SetServerName`
  in `src/core/remote.rs` (from line 23), and the variant joins the remote
  group that `src/app.rs:1248` routes there. A core test covers it.
- No new `Effect`. Pairing is the existing `PairingFailed` and `Paired`,
  revocation is `RevokeDevice`, and `DeviceSeen` is unchanged.

**`src/adapters/remote.rs`**
- `RemoteReply` (389-394) gains `set_cookie: Option<String>`. Every
  literal gains `set_cookie: None`: about 30, almost all in `src/app.rs`.
- `respond` (530-553) becomes `respond_with(w, status, content_type,
  headers, body, close)`. `respond` stays as the JSON wrapper. It adds the
  API's three headers, plus `Set-Cookie` when the reply carries one.
- `respond_upgrade` (519-526) takes `protocol: Option<&str>` and writes
  `Sec-WebSocket-Protocol` when given. `connection` passes
  `"switchboard.v1"` when the request's `ws_protocol` is set. A browser
  that offered subprotocols fails a 101 that names none.
- `Limits` (623-639) gains `page: Mutex<RateWindow>`, which counts every
  path outside `/api/v1/` and is built beside `pair` and `info`
  (684-685).
- In `connection` (770-876), right after the 429 check (787): a path
  outside `/api/v1/` is answered by `web::answer(&request)` (a file or
  the 404), and the connection returns.
- `make_identity`'s doc comment (90-94) stops saying "never expires".

**`src/adapters/web.rs`** (new)
- `pub fn answer(request: &RemoteRequest) -> WebReply`, where `WebReply`
  is `{status, content_type, headers, body}`.
- `pub fn static_file(method, path) -> Option<StaticFile>`, where
  `StaticFile` is `{content_type, body: &'static [u8]}`.
- `pub fn headers(host: Option<&str>) -> Vec<(&'static str, String)>`,
  which builds the CSP.
- The four `include_str!("../../web/...")` constants.

**`src/app.rs`**
- **`renew_remote_certificate(&mut self, now: SystemTime)`**, a public
  method, so a test can pass a time:
  1. Skip when read-only or there is no identity.
  2. Check the core identity's certificate with
     `remote::reissue_due(identity, now) -> bool`, which reads only the
     public certificate.
  3. When it is due, read the key from secrets, call `reissue_if_due`,
     and dispatch `CertificateRenewed`.
  4. A failure is logged and the old certificate is kept.
- **Where it runs:**
  - `listen_remote` (2928) calls it with `SystemTime::now()` **before**
    setting `remote_wake`, so its `SaveIdentity` only saves, and then
    binds with the renewed certificate.
  - `Tick` calls it once a day, tracked by a `certificate_checked:
    Option<Instant>` field. With the 30-day margin, a daily check never
    lets a running app reach `not_after`.
  - A rebind for a listen change binds with whatever the core holds, and
    the daily check covers it.
- **`run_remote_effect`** (2891), `SaveIdentity` arm: it already rebinds
  when the listen setting differs from `remote_bound`. It also rebinds
  when the certificate differs from the one bound. `bind_remote`
  (2936-2957) builds a fresh `tls_config`, so the new certificate is
  served. `remote_bound` becomes the bound `(Listen, certificate)`. The
  rebind closes open connections once per renewal, and clients reconnect
  as they do after any drop.
- **`serve_remote`** (3017):
  - The prefix match at 3023 stays as a backstop. Paths outside
    `/api/v1/` never get here.
  - After `authenticate` (3032-3035), add
    `("GET", ["device"]) => return Served::Now(self.remote_device(&device, request))`.
    It answers `{device_id, name, scope}` in the envelope, and sets the
    cookie again when the request came by cookie.
- **`authenticate`** (3127-3149): try `bearer` first, then the cookie
  plus key, as in the table above.
- **`remote_pair`** (3606-3673): with `"cookie": true`, mint the two
  halves. The reply (3660-3665) carries `web_key` and not `token`, and
  `set_cookie` is set.
- **`remote_info`** (3580): the capabilities gain `"web"`.

**`src/core/remote.rs`**
- A pure `pub fn invite_url(host: &str, port: u16, identity: &Identity,
  code: &str) -> String`. Every part of it is hex, a UUID, Crockford or a
  checked DNS label or IP, so it needs no percent-encoding.

**Swift** (`client/Sources/SwitchboardProtocol/Info.swift:92-117`)
- `PairingInvite.parse` tries `URLComponents` first. It needs an `https`
  scheme, a host, a port, and `code`, `key` and `id`. These fill
  `addresses: ["host:port"]`, `spkiSha256`, `code` and `serverID`.
- Otherwise it falls back to the JSON decode at line 115.
- The command-line `pair` (`main.swift:107-125`) and both headset paths
  (`BrowserView.swift:106`, `:883`) call `parse`, so they get the URL form
  with no other change.
- The usage line (`main.swift:13`) becomes `pair '<invite url or json>'`.
- A `.local` host resolves through the system resolver like any name.

## The UI

### The dialog (`src/ui/remote.rs`)

- The payload (155-165) becomes `core::remote::invite_url(...)` with the
  host from decision 3. The JSON is built beside it, unchanged.
- The QR texture cache (166-173) is keyed on the **payload string**
  instead of the code. Turning on the local network, a port change or a
  new address therefore redraws the QR, and the QR and "Copy invite"
  never disagree.
- When `lan` is on and neither a `.local` name nor a LAN address exists:
  - no QR is drawn and "Copy invite" is hidden, because a URL with no
    host opens nothing;
  - the window says "No local network address was found, so there is no
    code to scan. Check the Mac's network connection.";
  - the key and code stay shown, and "Copy JSON invite" stays.
- "Copy invite" (199-206) copies the URL. Its hover text becomes "The QR
  code's link, to open on a device or paste into a client".
- A new secondary button, "Copy JSON invite", has the hover text "The
  same invite for a Switchboard client built before the web page".
- The hint (191-197) gains the issue-date line and the trust sentence
  from decision 1.
- When `lan` is off, the hint gains: "Turn on 'Allow devices on the local
  network' (above) to pair a phone or another computer." The checkbox is
  at 110-112.
- The devices list (230-276) is unchanged: a browser shows its name, for
  example "Safari on iPhone", and its "seen <time>" shows a stale one.

### The page (`web/`)

**Files**
- `index.html` holds the structure only: no inline script, no `style=`.
- `app.js` is plain ES2020 in one file.
- `app.css` uses `prefers-color-scheme` for light and dark.
- `icon.svg`.
- No framework, no build step, no dependency.
- Text is set with `textContent` only and never with `innerHTML`, because
  agent output is untrusted.
- Every fetch sends `X-Switchboard-Key`.
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
     true}`, store the marker with `web_key`, and continue.
4. Otherwise compare `/info` with the marker. If the key differs, say
   "<host>'s identity changed. Pair again from its Remote devices window"
   and send nothing more.
5. `GET /api/v1/device`:
   - 200: header and scope.
   - 401 `key_missing`: "This browser lost part of its pairing (Safari
     clears a site's storage after a week without a visit). Pair again,
     and revoke the old entry in Remote devices on <host>."
   - 401 with a marker: "<host> no longer accepts this browser: it was
     revoked, or its sign-in was replaced. Pair again."
   - 401 without a marker: "Open Remote devices on the Mac, choose Allow
     pairing, and scan its code with this device's camera".
6. Subscribe, snapshot, then apply the buffered events by revision.
7. `pairing_closed`, `wrong_code`, `wrong_key` and `rate_limited` are each
   said in words.

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
- On a socket close, `/device` first: a 401 shows the matching state.
- Otherwise reconnect after 1, 2, 4 … 30 s.
- `visibilitychange` to visible reconnects at once.
- A new `epoch` refetches everything, and `resync_required` reconnects.

## The tests

All run under `cargo test --locked --workspace`, except the Swift ones.

**`src/core/tests.rs`**
- `the_invite_url_carries_the_code_key_and_id`.
- `a_renewed_certificate_saves_the_identity`: `CertificateRenewed`
  changes only `certificate` and emits `SaveIdentity`, and a read-only
  core emits nothing.

**`src/adapters/remote.rs` unit tests**
- `a_request_reads_its_web_headers`: only `__Host-switchboard-*` cookies
  are kept; the key is read from the header and from the `sbk.`
  subprotocol; `ws_protocol` and `host` are read.
- `an_upgrade_offering_the_subprotocol_echoes_it`.
- `a_reissued_certificate_keeps_the_key`: the SPKI hash is unchanged,
  `not_after` is within 825 days, the SANs include `localhost`, the
  pinned client still connects, and a fresh one is not due.
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
  - `/favicon.ico`, `/apple-touch-icon.png`, `/index.html` and `POST /`
    answer 404 with `Connection: close`, and the frame's inbox sees none
    of them;
  - `/?code=X` serves the page and leaves the pairing window open with no
    failure counted.
- `the_page_names_only_paths_the_host_serves`:
  - scan `index.html` for `src=`/`href=`; each must be served by
    `web::static_file`;
  - scan `app.js` for `/api/v1/` literals; each, with ids filled in, must
    not answer 404 for a paired browser;
  - `index.html` has no inline `<script>` body and no `style=`.
- `before_pairing_a_lan_bind_serves_only_info_pair_and_the_page`: extends
  the model at 426.
- `a_browser_pairs_with_a_cookie_and_a_key_and_reads_with_both`:
  - with the window open, `/pair {"cookie":true}` returns `Set-Cookie`
    with every attribute, a body with `web_key` and no `token`;
  - `/projects` with the cookie and `X-Switchboard-Key` returns 200;
  - `/device` returns the scope and sets the cookie again;
  - the events upgrade with the cookie and the `sbk.` subprotocol gets a
    101 naming `switchboard.v1`, then `hello`;
  - a bearer device beside it still reads.
- `half_a_browser_token_is_refused`:
  - the cookie alone is a 401 `key_missing` that clears the cookie;
  - the key alone, or as a bearer, is a 401;
  - the cookie with another key is a 401;
  - an upgrade with the cookie and no subprotocol is a 401;
  - `last_seen` does not move in any of these.
- `a_revoked_browser_is_told_and_forgets_its_cookie`: follows the model
  at 658. After `RevokeDevice` the socket closes, and the next `/device`
  is a 401 whose `Set-Cookie` carries `Max-Age=0`.
- `a_scope_change_shows_on_device`: read, then control after
  `SetDeviceScope`.
- `a_certificate_that_comes_due_while_running_is_renewed`:
  1. Bind.
  2. Call `renew_remote_certificate` with a time 10 days before the
     certificate's `not_after`.
  3. The identity's certificate changes and `spki_sha256` does not.
  4. `identity.json` on disk holds the new certificate.
  5. A pinned client connects, and the certificate it was served has the
     new `not_after`.
  6. A second call at the same time changes nothing.
- `info_lists_the_web_capability`.

**`tests/ui.rs`**
- `remote_devices_dialog_opens_pairing_and_revokes_a_device` (7424):
  - the invite assertion (7486-7492) parses a URL instead of JSON. It
    asserts an `https://127.0.0.1:` prefix, since the fixture's
    `Listen::default()` is loopback, then the `code=`, `key=` and `id=`
    values, then the turn-on-the-local-network hint;
  - "Copy JSON invite" copies JSON with the same code and key;
  - with the window still open, tick "Allow devices on the local
    network", `run_steps(2)`, and copy again: the URL's host is no longer
    `127.0.0.1`.
  - The texture cache keyed on the payload is not observable from the
    harness. The copy that changed is what this test pins.

**Swift** (`client/Tests/SwitchboardProtocolTests/FixturesTests.swift`)
- Add `testInviteParsesTheInviteURL` beside `testInviteParsesTheQRText`
  (156), which stays as the JSON form.
- A `.local` host round-trips into `addresses`.
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
- **The `.local` name.**
  - Record both `scutil --get HostName` (or "not set") and `scutil --get
    LocalHostName`, and `hostname -s`.
  - Does `https://<LocalHostName>.local:<port>/` load from each of the
    three browsers?
  - On a Mac where the names are equal, set a different `HostName` with
    `sudo scutil --set HostName <other>` for the run, check that the
    invite still carries `LocalHostName` and loads, then restore the old
    value (or `--set HostName ""` if none was set).
  - Fallback: the invite uses the LAN IP, and the address-change gap
    goes in the known gaps.
- **Cold page load against the pool of 4.** Measured with the built app,
  `SWITCHBOARD_DATA_DIR`, and `remote-listen lan <port>`. Count every
  request the browser makes, the icon probes included, from a fresh
  profile and from a home-screen launch on iOS.
  - Fallback: drop `icon.svg` and its `<link>`, which leaves three files.
- **Whether the exception holds** across a reload, a relaunch, and an
  address change.
- **The subprotocol** reaches the host from iOS Safari, and the 101 that
  echoes it is accepted.
- **The acceptance run:**
  1. Scan the code and accept the warning.
  2. See the sessions.
  3. A session starts waiting and moves to the top.
  4. Send a reply from the phone and see it arrive.
  5. Revoke the phone; the page says so on its next request.

## Docs in the same commit

**`docs/remote-client.md`**
- §4.1:
  - every path outside `/api/v1/` is answered on the connection thread;
  - the page pool and its window.
- §4.3: the `/device` row and the four static rows.
- §5:
  - the pairing rule now includes the page's four files;
  - the browser-trust paragraph from decision 1;
  - the split token, with its cookie exposure stated;
  - the URL invite, with the JSON kept behind "Copy JSON invite".
- §5, three sentences rewritten for the browser case:
  - line 309, "after pairing, every connection verifies the pin before
    the token is presented", becomes: "after pairing, a native client
    verifies the pin on every connection before the token is presented;
    a browser relies on the certificate exception it accepted when it
    paired (trust on first use)."
  - line 312, "It is sent only as a bearer header over the pinned
    connection, never in a URL", becomes: "A native client sends it only
    as a bearer header over the pinned connection. A browser holds it in
    two halves, an `HttpOnly` cookie and a key in the page's own storage,
    sent as a header or WebSocket subprotocol. Neither half is ever in a
    URL, and the cookie half, which the browser also sends to other
    services on the same address, is useless without the key."
  - line 318, "keep tokens out of logs, URLs, Bonjour TXT records and
    crash reports", gains: "The invite URL carries the single-use
    pairing code. The page removes it from the address bar before any
    request, the host answers that path on the connection thread
    without logging it, and every reply sends `Referrer-Policy:
    no-referrer`."

**`docs/design.md`**
- The Remote client summary (line 3111), "pairing with a pinned host
  key", becomes "pairing with a pinned host key (a browser: a
  click-through certificate exception and a split token)".
- In the Remote client status (from line 3107): the web surface is built,
  with the spike's results.
- The known gaps:
  - tickets, drafts, runs and pinned files are not on the page;
  - first contact for a browser is trust on first use;
  - the cookie half goes to every service on the Mac's address;
  - a browser that loses its storage, or a Mac renamed, leaves a stale
    device to revoke by hand;
  - a certificate renewal brings one expected warning per browser and
    rebinds the port;
  - the Bonjour record's host (`src/adapters/bonjour.rs:41`) is still
    `<hostname -s>.local.` with no address, which points native discovery
    at a name nobody answers when `HostName` differs from
    `LocalHostName`. That predates this change and needs its own fix.

**`README.md`**
- The Layout table gains `web/` and `src/adapters/web.rs`.
- The `switchboard-remote pair "$(pbpaste)"` note (line 277) says the
  clipboard holds the URL, and "Copy JSON invite" is for an older build.

**`spikes/README.md`**
- Row 23.

**`CHANGELOG.md`**
- Under Unreleased: "Open Switchboard in a browser on your local network:
  scan the pairing code with a phone to see your sessions, which ones are
  waiting, and reply. The pairing code is now a link; a client built
  before this release pairs with 'Copy JSON invite'."

## Commit

`Serve a browser control page on the remote port, paired from a URL in the QR code`

Contract additions:
- `src/core/model.rs`: none.
- `src/core/action.rs`: `AppAction::CertificateRenewed { certificate:
  String }`.
- `src/ports/`: none.
- `src/core/remote.rs`: `invite_url`.
- Wire, additive:
  - `GET /api/v1/device` → `{device_id, name, scope}` (read scope);
  - `/pair` body field `cookie: bool`; with it, the reply carries
    `web_key` instead of `token` and sets
    `__Host-switchboard-<8 hex of server_id>`;
  - request header `X-Switchboard-Key`, and WebSocket subprotocols
    `switchboard.v1` and `sbk.<key>`;
  - `/info` capability `"web"`;
  - error code `key_missing` (401);
  - static `GET /`, `/app.js`, `/app.css`, `/icon.svg`.
- Adapter, additive:
  - `RemoteRequest.{web_cookie, web_key, ws_protocol, host}`;
  - `RemoteReply.set_cookie`;
  - `remote::{reissue_if_due, reissue_due, certificate_validity,
    local_host_name}`;
  - `adapters::web::{answer, static_file, headers}`;
  - `SwitchboardApp::renew_remote_certificate`.
- Swift: `PairingInvite.parse` accepts the URL form.
