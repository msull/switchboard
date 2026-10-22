# Issue #230: a web control surface on the remote port (investigation notes)

Read: `CLAUDE.md`, `docs/design.md` (Durable store, Trust boundary, Remote
client status from line 3107), `docs/lessons.md`, `docs/remote-client.md`
§4.1–§5. Line numbers below are from this worktree at `7fe8949`.

## Record first: what is on disk and how it is rehydrated

**On the host, nothing new is durable.** A browser is a `PairedDevice` like
the headset: `{id, name, scope, token_sha256, created, last_seen}` in
`<data dir>/devices.json` (`src/ports/remote.rs:96-107`), written through the
store's atomic path via `Effect::SaveDevices` (`src/core/remote.rs:117-121`).
The name comes from the page (built from `navigator.userAgent`, for example
"Safari on iPhone") and is cut to 64 chars by `AppCore::new_device`
(`src/core/remote.rs:163-175`). **There is no schema change and no
`SCHEMA_VERSION` bump.** The dialog doesn't need a "browser" flag: the name
already says what the device is. The page's HTML, CSS and JS are bytes fixed
at compile time (`include_str!`), not a record.

One durable change I'd consider: **reissue the certificate from the same key**
with SANs and a bounded lifetime, if the spike (below) shows browsers need it.
§5 already allows a reissue: the pin is the SPKI hash, so the
`identity.json` `certificate` field changes and `spki_sha256` does not. Same
field, same shape, so no schema bump either. Today `make_identity`
(`src/adapters/remote.rs:95-127`) makes a certificate with **no SANs**
(`CertificateParams::new(Vec::<String>::new())`, line 102), CN
`Switchboard <id>`, and rcgen's default `not_after` of year 4096.

**On the device:**
- The token is an `HttpOnly; Secure; SameSite=Strict` cookie set by the
  `/pair` reply. The page's script never sees it (decision 2).
- A non-secret marker in `localStorage`:
  `{server_id, spki_sha256, device_id, scope, server_name}`. Its only job is
  to tell three states apart:
  - never paired (no marker);
  - revoked (marker present, 401);
  - identity changed (marker present, but the `/info` key differs from the
    stored one).

  Reads and writes go in try/catch. If the marker is gone, the page reads as
  "not paired", which is safe.

**Rehydration of a page load (processes are a cache):**
1. `GET /`, then `app.js` and `app.css`.
2. `GET /api/v1/info`. If its key is not the marker's key, show "identity
   changed, pair again" and send nothing else.
3. `GET /api/v1/device` (new, below) gives the scope and name for the header.
   A 401 here means "revoked" when there is a marker.
4. Open `/api/v1/events` and wait for `hello`. Buffer the events, as §4.4
   says.
5. Take the snapshots: `waiting`, `projects`, then each project's `sessions`.
6. Apply the buffered events by revision.
7. Enable the reply box.

The page keeps nothing else. A reload is the reconnect. `epoch` changes mean
refetch everything; `resync_required` means reconnect.

## Code the issue touches

### `src/adapters/remote.rs` (listener, HTTP, pools)
- `RemoteRequest` (360-372) keeps method, path, undecoded query, body,
  `bearer` and `upgrade`, and drops every other header. The web surface also
  needs these headers read from the request:
  - `cookie`: the token, by name;
  - `origin`;
  - `sec-fetch-site`;
  - `host`, to build the expected origin;
  - optionally `user-agent`.

  The token must stay out of anything logged, as `bearer` does today.
- `read_request` (444-516) lowercases the headers; the extraction goes in
  next to the `bearer` lines (487-494). The query stays undecoded: the
  page's `?code=` reaches `GET /` and the host must ignore it and never log
  it. `refuse` (`src/app.rs:3754-3772`) logs `request.path`, which has no
  query, so that already holds.
- `respond` (530-553) hard-codes `Content-Type: application/json`, with no
  other headers. It needs a content type plus extra headers: `Set-Cookie`,
  CSP, `Cache-Control`, `X-Content-Type-Options`, `Referrer-Policy`.
  `RemoteReply` (389-394) gains additive fields, either `headers` or a
  narrow `set_cookie: Option<String>`, so the frame's `/pair` reply can set
  the cookie. This is an adapter type, not the `core/action.rs` contract.
- `respond_upgrade` (519-527) writes the 101. With the cookie choice it does
  not need to echo a `Sec-WebSocket-Protocol`.
- Pools: `MAX_UNAUTHENTICATED = 4` and `MAX_PAIRED = 12` (564-568). The
  accept loop closes the fifth unauthenticated connection at once (699-707).
  A connection moves pools only on its first authenticated reply (836-848).
  `IDLE_TIMEOUT` is 30 s (573).
- Rate windows: `Limits::limited` (623-639) keys on `PAIR_PATH` and
  `INFO_PATH` (586-587). `PAIR_PER_MINUTE = 6` and `INFO_PER_MINUTE = 60`
  (582-584). The connection thread answers 429 before the frame (787-795).
- `connection` (770-876) is where static files are answered from the
  connection thread, before the frame channel, in the same place the 429
  is.

### `src/app.rs` (frame side)
- `serve_pending` (1572-1615) drains `DRAIN_PER_PASS` requests into
  `serve_remote`.
- `serve_remote` (3017-3123):
  - 3023-3025: any path without `/api/v1/` is a 404 here, and that is the
    prefix match the issue names.
  - 3027-3031: `info` and `pair` are answered unauthenticated.
  - 3032-3038: `authenticate`, then the `events` upgrade.
  - The allowlist `match` follows. `/api/v1/device` is added there.
- `authenticate` (3127-3149) reads only `request.bearer`. Add the cookie as
  a second source, accepted only under the origin rule (decision 2).
- `remote_subscribe` (2723-2758).
- Revocation closes the device's subscribers in `dispatch_inner` (486-499).
- `remote_info` (3568-3600) is the `/info` body with `spki_sha256`. Add a
  `web` capability.
- `remote_pair` (3606-3673):
  - It checks the key and the code.
  - It mints the token, and the reply carries it in the JSON body (3660-3665).
  - The browser variant (a body with `"cookie": true`) sets the cookie
    instead and leaves `token` out of the body.
- The scope refusal `"scope"` (3166-3174) is what the page hides the reply
  box on, as `ServerConnection.swift:675` treats it.

### `src/ui/remote.rs` (dialog)
- The QR payload is built at 160-165 as `{server_id, spki_sha256, code,
  addresses}` JSON, from `remote::local_address()` and the bound port
  (156-159).
- The texture is cached by code (166-173). "Copy invite" copies the same
  text (199-206). The hint text is at 191-197.
- The devices list with scope and Revoke is at 230-276. The dialog's
  sentence on what the click-through means goes here.

### `src/core/remote.rs`
- The pairing window, failures and `Paired` transitions are at 44-73. They
  don't change: a browser's `/pair` is the same `PairingFailed` and
  `Paired` actions.

### Swift text path
- `client/Sources/SwitchboardProtocol/Info.swift:92-117`:
  - `PairingInvite.parse` decodes JSON only (line 114).
  - Teach it the URL form: `URLComponents`, host and port into
    `addresses`, then `code`, `key` and `id`.
  - Keep JSON as the fallback for an invite copied from an older host.
- `client/Sources/switchboard-remote/main.swift:12-13`: the usage string.
  `main.swift:107-125`: the `pair` command (one argument goes through
  `parse`, so it gets the URL form for free).
- The headset paste goes through the same `parse`. Its call site is in
  `SwitchboardClientCore/ConnectionManager.swift` and
  `SwitchboardUI/BrowserView.swift`.
- Swift test: a case in `client/Tests/SwitchboardProtocolTests` for the URL
  form.

### Tests: `tests/remote.rs`
- `http()` (245-296) returns only status and body. It needs to return the
  headers and to send `Cookie` and `Origin`. Every call goes through
  `call`/`get` (299-330).
- `info_is_served_to_anyone_...` (426) asserts `/other` is a 404. That
  stays true: only the named static paths are served.
- `revoke_and_a_scope_change_...` (658) and `the_allowlist_refuses_...`
  (682) are the models for the browser tests.
- Fixtures: `write_fixtures` (1587) and
  `the_client_fixtures_decode_on_both_sides` (1829). Add a `device`
  fixture if `/api/v1/device` is added.

### The card excerpt is not on the wire
`SessionView` (`control/src/lib.rs:637-688`) has the `card` word, `waiting`,
`waiting_reason` and `kind`, but no excerpt. The Mac's card computes the
excerpt from the transcript (`src/ui/cards.rs:174-184`, `agent_excerpt`
558-575), and §4.3 says the same. The headset fetches `last-response` per
listed session (`BrowserView.swift:149-163`). The page does the same, which
needs no new read model:
- an agent row's excerpt is the first line of its `last-response`, fetched
  for the rows on screen and refreshed on `changed {conversation}`;
- any other row shows `card`.

## Decisions

### 1. Browser trust: the click-through is the trust step
Pick the click-through on the local network, said plainly in the dialog. Do
not offer the certificate for download. My reasons:
- On iOS, trusting a downloaded certificate means installing a profile
  (Settings → Profile Downloaded → Install), then a second switch in
  General → About → Certificate Trust Settings. That's heavy for the
  owner, and it puts a trust anchor on the phone.
- A download path would be another thing served before pairing on a
  non-loopback bind. §5 wants that list as short as possible.
- The download buys nothing the click-through doesn't against an active
  attacker at first contact, which is the only case either one matters.

What the `key` in the URL buys, and what it does not:
- The genuine page, served by the genuine host, refuses to post the code if
  `/info` presents another key. That covers a stale QR, or a URL pointing at
  the wrong instance.
- It does **not** stop an impostor that answers the first visit. The
  impostor serves its own page, reads the code and the key from the URL, and
  relays `/pair` to the real host with the real key. The host's
  `spki_sha256` check (`src/app.rs:3639-3647`) passes, because the key is in
  the URL. So for a browser, first contact is trust on first use. The
  native clients' pin is stronger, and the dialog says so.
- After first contact, the browser keeps the exception for that
  certificate. A **second** certificate warning for the same address means
  something else is answering. The dialog says "accept the warning once,
  when you pair; a warning later means stop".
- The stored marker also lets the genuine page notice a rotated key and
  show "identity changed".

The amendment to §5 reads: "Require pairing before anything but `/info`,
`/pair` and the page's fixed files". The files are compile-time bytes and
carry no state.

**This needs a spike before it is built:** `spikes/23-web-surface`. Measure:
- Whether Safari on iOS, Safari on macOS and Chrome on macOS offer a
  click-through at all for today's certificate. It has no SANs and a
  validity to the year 4096. Apple's TLS policy limits validity to 825
  days and expects a SAN and the serverAuth EKU. I remember Chrome on macOS
  reporting such certificates as a **non-bypassable** `ERR_CERT_REVOKED`,
  but I have not checked that. If it is true, the host reissues the
  certificate from the same key: SANs for the LAN address, `localhost` and
  `<host>.local`, the serverAuth EKU, and 825 days or less. The reissue is
  renewed on listen when it nears expiry. The pin does not move.
- Whether the browser remembers the exception across a reload, a relaunch,
  and an address change.
- Whether `connect-src 'self'` covers `wss:` on the same origin in the
  Safari versions that matter. If it does not, the CSP names
  `wss://<Host>` from the request's `Host` header.
- How many parallel connections a cold page load opens against the pool of
  4 (decision 4).

### 2. Token to the WebSocket: an HttpOnly cookie
Pick the cookie that `/pair` sets.
- The cookie is named per installation:
  `__Host-switchboard-<first 8 of server_id>`. Cookies are scoped by host,
  **not by port**, so two instances on one Mac at ports 43187 and 43188
  share a cookie jar for that IP.
- Attributes: `Secure; HttpOnly; SameSite=Strict; Path=/; Max-Age=34560000`.
  That's 400 days, Chrome's cap. The host re-sends it on `GET /api/v1/device`
  so it slides.
- The host keeps accepting `Authorization: Bearer` for the native clients.

Why the cookie:
- No script can read it, and the page renders text an agent printed.
- Safari deletes script-writable storage (`localStorage`) after 7 days
  without a visit. A cookie the server set is exempt.
- `new WebSocket()` carries it.
- `Sec-WebSocket-Protocol` would have to live in `localStorage` and be
  echoed in a header.

The rule that goes with the cookie: **cookie auth is accepted only when the
request is same-origin.**
- `Origin` must equal `https://<Host>`, or `Sec-Fetch-Site` must be
  `same-origin` (for a GET that sends no `Origin`).
- A WebSocket upgrade always carries `Origin` and must match.
- A cookie request that fails the rule gets a 403 `cross_origin` and no
  `DeviceSeen`.

Why the rule is needed:
- `SameSite` compares sites, and for an IP address the site is the IP
  without the port. Another service on the same Mac is therefore
  same-site.
- WebSocket handshakes are not covered by CORS.

The cost of the cookie:
- A different HTTPS service on the same IP, any port, also receives the
  cookie. On the owner's own Mac that's a known risk, and the spec should
  say so.
- Two Switchboard instances on one Mac each receive the other's cookie. It
  matches no device there, so it is harmless.

On a 401 that came with a cookie, the host answers `Set-Cookie ...;
Max-Age=0`, so a revoked phone forgets the token. The spec's "never in a
URL" stands.

### 3. One QR: the URL
The QR and "Copy invite" both become
`https://<address>:<port>/?code=<code>&key=<spki_sha256>&id=<server_id>`.
- `PairingInvite.parse` accepts the URL, and still accepts JSON for an
  invite copied from an older host. Nothing else produces the JSON, so the
  dialog drops it.
- The query is about 120 characters, and the QR stays readable at
  `QR_SCALE = 4` (`src/ui/remote.rs:42`).

On the page:
- `history.replaceState(null, "", "/")` is the first statement of
  `app.js`, before any fetch.
- `Referrer-Policy: no-referrer` on every response.
- The camera app's own history is outside our control, but the code is
  single-use and dies with the five-minute window.
- If the port is bound to loopback only, the dialog already shows a LAN
  address it can't reach. It should say "turn on local network access"
  instead of offering an unusable URL.

### 4. Static paths
Exactly these four paths, `GET` only, any query ignored. They are answered
by the **connection thread** from embedded bytes and never reach the frame,
so a page load costs no frame time and no inbox slot.

| Path | Type |
|---|---|
| `/` | `text/html; charset=utf-8` |
| `/app.js` | `text/javascript; charset=utf-8` |
| `/app.css` | `text/css; charset=utf-8` |
| `/icon.svg` | `image/svg+xml` |

The page declares `<link rel="icon" href="/icon.svg">`, so the browser does
not ask for `/favicon.ico`. That path stays a 404.

Headers on all four:
- `Cache-Control: no-store`. The files are a few KiB, so there is never a
  stale page across versions.
- `X-Content-Type-Options: nosniff`
- `Referrer-Policy: no-referrer`
- `X-Frame-Options: DENY`
- the CSP: `default-src 'none'; script-src 'self'; style-src 'self';
  img-src 'self'; connect-src 'self'; base-uri 'none'; form-action 'none';
  frame-ancestors 'none'`. Adjust `connect-src` if the spike says so.

Add `nosniff` and `no-store` to the API responses too.

Pools and rate windows:
- Static loads count in the **unauthenticated pool**, as today. A browser
  that opens more than four parallel connections would have the extras
  closed at accept.
- The static replies send `Connection: close`, so a page load holds a slot
  only for the time of one file.
- Static files get a rate window of their own, **page: 60 a minute per
  listener**, alongside `/info`'s 60.
- The spike measures a cold load on iOS Safari against the pool of 4. If
  it fails, the fallback is to inline the CSS into the HTML, which leaves
  two files, rather than to raise the pool.

What the page shows:
- No code and no marker: "Open the pairing window on the Mac (Remote
  devices → Allow pairing) and scan its code".
- A marker but a 401: "This device was revoked on <host>. Pair again".
- A key mismatch: "<host>'s identity changed. Pair again".
- `pairing_closed`, `wrong_code` and `rate_limited` are each said in
  words.

### 5. Where the page lives and how it is tested
Files:
- `web/index.html`, `web/app.js`, `web/app.css`, `web/icon.svg`, embedded
  with `include_str!` in a small `src/adapters/web.rs`. It holds
  `fn static_file(path) -> Option<(&'static str, &'static [u8])>` and the
  header set.
- The JS is plain ES2020 in one file. Text is set with `textContent` only:
  never `innerHTML`, because agent output is untrusted.
- No framework, no build step, no dependency.

Tests in `tests/remote.rs`:
- **Every reference is served.** Scan `index.html` for `src=` and `href=`,
  and `app.js` for `fetch(`, `new WebSocket(` and `/api/v1/` literals.
  Assert each static path answers 200 with the type and headers above, and
  each API path is in the allowlist.
- **Pre-pairing on a LAN bind.** The four paths answer before pairing.
  Nothing else does.
- **The cookie flow end to end** with the test's own TLS client:
  - `POST /api/v1/pair` with `{"cookie":true}` and a matching `Origin`
    returns `Set-Cookie` with every attribute, and the body has no
    `token`.
  - `GET /api/v1/projects` with that cookie and a matching `Origin` returns
    200.
  - Without `Origin` or `Sec-Fetch-Site`, or with a wrong `Origin`, it
    returns 403 `cross_origin`.
  - A WebSocket upgrade with the cookie and a wrong `Origin` is refused.
  - With the right `Origin` it gets `hello`.
  - After `RevokeDevice`, the subscriber is closed, and the next request
    gets a 401 that clears the cookie.
- The `/` query is never logged.
- Extend `http()` (245) to send arbitrary headers and return the response's
  headers.
- `/api/v1/device` goes into the fixture set (1587) and the Swift fixture
  test.

There is no browser automation. The acceptance at the end of the issue is
a manual run, recorded in the spike README and the design status.

### 6. Device record
Unchanged schema; see the record section. The page sends `device_name`.
The host could fall back to `User-Agent` when the name is empty, but that's
optional.

The page needs the device's **current** scope for its header and reply
box. Scope can change in the dialog, and no reply says it today; the
headset learns of it only through a 403 `scope`. So add **`GET
/api/v1/device`**, with read scope, answering `{device_id, name, scope}`.
It is additive, on the allowlist, and listed in §4.3. The page reads it on
load, on every WebSocket reconnect, and after a 403 `scope`. Pushing a
`changed {device}` to that device's subscribers on `SetDeviceScope` is
optional and can come later.

## Order of work (one commit each, app runs at every step)
1. Spike 23. Record the browser results; decide on the certificate reissue
   and the CSP `wss` form.
2. If needed: reissue the certificate from the same key (SANs, EKU, at
   most 825 days, renewed on listen). Tests: the SPKI hash is unchanged and
   the native pin still verifies.
3. Adapter changes:
   - header extraction (cookie, origin, sec-fetch-site, host);
   - `respond` with a content type and extra headers;
   - static paths on the connection thread with the page window;
   - `RemoteReply` header additions.
4. Frame changes:
   - cookie auth with the origin rule in `authenticate`;
   - `/pair` `{"cookie":true}`;
   - 401 clears the cookie;
   - `/api/v1/device`;
   - the `web` capability in `/info`.
5. QR and invite as the URL in `src/ui/remote.rs`, the dialog's trust
   sentence, and `PairingInvite.parse` taking the URL, with a Swift test.
6. `web/` page and its tests. Then update the spec (§4.1 bounds, §4.3
   table and the `/device` row, §5 pairing and the browser trust
   paragraph), the remote status in `design.md`, the README Layout
   (`web/`, `src/adapters/web.rs`), and a CHANGELOG line.

## Risks to settle in the spike, not by reasoning
- Whether today's certificate can be clicked through at all in Chrome on
  macOS, and in Safari on iOS and macOS.
- A cold page load against the pool of 4 unauthenticated connections.
- How long iOS Safari keeps the certificate exception, and whether it
  holds after the device's address changes.
- CSP `'self'` and `wss:` in Safari.
- A WebSocket left on a sleeping phone. The 30 s idle timeout and the
  queue of 256 already bound it. On `visibilitychange` the page
  reconnects and refetches.
