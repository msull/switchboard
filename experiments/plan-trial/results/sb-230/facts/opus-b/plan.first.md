# Plan for #230: the browser page on the remote port

Worktree at 7fe8949. Line references are to this tree. `SCHEMA_VERSION`
is 16 (`src/core/model.rs:12`) and `IDENTITY_SCHEMA_VERSION` is 1
(`src/ports/remote.rs:17`); neither moves. "Facts" is
`facts/1/repo/facts.md`.

## Where this departs from the issue

- **The code travels in the fragment, not the query.** The QR is
  `https://<addr>:<port>/#code=<code>&key=<spki_sha256>&id=<server_id>`.
  A browser never sends a fragment to the server, so the code never
  reaches a request target, `RemoteRequest::query` or a `Referer`. It
  reaches the host only in the `/pair` body. That makes the notes'
  questions about logging the query, refusing a `GET /?code=`, and
  `Referer` leakage go away. This rests on RFC 3986 §3.5 (the user agent
  handles the fragment). That is not in the facts file, and
  `tests/remote.rs` cannot show it, because the host's client is not a
  browser. The page still calls `history.replaceState(null, "", "/")`
  before anything else, but facts list whether Safari or the camera
  records the URL first as Unresolved (facts:46). The fragment does not
  change that, and the plan accepts it: the code is single-use, and the
  window closes on the first pairing (`src/core/remote.rs:64`).
- **The card's one-line excerpt is not on the wire.** `agent_excerpt`
  derives it in the UI from the parsed conversation (facts:17,
  `src/ui/cards.rs:558-574`), and `SessionView` has no such field
  (`control/src/lib.rs:637-688`). The page's list shows
  `waiting_reason` when the session is waiting, otherwise the card word
  (`card`), plus the first line of `notes`. Facts leave the excerpt to
  the owner (facts:83). If the owner wants it, the fallback is an
  additive `#[serde(default)] excerpt: Option<String>` on `SessionView`,
  computed on the host by the `agent_excerpt` rule. That means a
  fixture rewrite and a `revision.rs` fingerprint change, so it is a
  separate commit.
- **The current certificate is not one a phone is sure to accept.** It
  has no SAN, validity to 4096, and no serverAuth EKU (facts:13, :33).
  The host now reissues the leaf at every bind over the same Keychain
  key (details under Signals), so the pin and every pairing survive
  (facts:12, spec §5 "a certificate reissued from the same key").
- **"Revoked" is told apart by the page, not the host.** A revoked
  token and an unknown token get the same 401 (`src/app.rs:3142-3148`).
  The page keeps `{server_id, spki_sha256}` per origin in
  `localStorage`. On a 401 it says "revoked" when `/info` still shows
  that key, "identity changed, pair again" when the key differs, and
  "not paired" when it kept nothing. Facts leave this to the owner
  (facts:91). If the owner says otherwise, the fallback is a distinct
  401 code, which needs a record of revoked token hashes. Safari wipes
  script storage after 7 days without a visit (facts:32), so after that
  a revoked browser hears "not paired, scan the QR". That is still the
  right instruction.
- **"The screen for anything else" has no screen when the session is
  stopped.** `screen` refuses a session that is not running (spec
  §4.3). The page says "Not running; nothing to show". Facts leave this
  to the owner (facts:94). If the owner wants more, the fallback is
  `runs` then `runs/{n}/output` for a command or service.
- **The cookie is not scoped by port** (facts:28). A cookie set by
  `https://<ip>:43187` is sent to every port on that address. So the
  cookie name carries the port. Every cookie-authenticated request also
  passes an origin check, because `SameSite=Strict` does not stop
  another HTTPS service on the same IP: that service is same-site.

## Decisions

1. **Trust: the click-through is the trust step.** The dialog says so
   in one sentence. It does not offer a downloadable certificate.
   Reason: a download is one more path before pairing, and on iOS it
   needs a CA certificate and the Certificate Trust Settings step
   (facts:34). That is more for the owner to do than accepting a warning
   once on a network they trust. What the URL's `key` buys: a genuine
   page compares it with `/info`'s self-reported `spki_sha256` and
   posts nothing on a mismatch. That catches a QR for another host and
   a stale QR. It does not catch an impostor, which serves its own page
   and reports the real key. Say both in the dialog sentence and in
   §5. Facts list this as Unresolved (facts:38, owner item facts:53):
   whether iOS Safari opens `wss://` after the click-through. **The
   build does not depend on the answer.** The page falls back on its
   own to polling REST every 5 s when the socket fails before `hello`.
   A "No" from the owner only makes polling the usual path on iOS. If
   the owner then wants live events there, the next step is the
   download route of facts:55 as its own issue. Spike 23 records the
   answer (Tests).
2. **Token: an `HttpOnly; Secure; SameSite=Strict; Path=/` cookie, no
   `Domain`, `Max-Age=34560000`.** The page's script never sees it. It
   survives Safari's 7-day storage cap where `localStorage` does not
   (facts:32). The same-origin WebSocket handshake sends it (facts:25).
   Name: `switchboard-<port>`, so two instances on one Mac never
   overwrite each other's cookie (facts:28). Native clients keep the
   bearer header, which is checked first. Whether a page reached through
   a certificate override is a secure context, and so keeps a `Secure`
   cookie, is Unresolved (facts:45). If Safari drops the cookie, the
   fallback is the subprotocol route: `/pair` returns the token in the
   body, the page keeps it in `localStorage`, sends it as
   `Authorization` on `fetch` and as the offered `Sec-WebSocket-Protocol`,
   and `respond_upgrade` echoes it (facts:18, :26, :27). Re-pairing
   after 7 days unused is the known cost.
3. **One QR: the URL.** The JSON form leaves the QR and "Copy invite".
   `PairingInvite.parse` (`client/Sources/SwitchboardProtocol/Info.swift:114`)
   accepts the URL and still accepts the JSON, because
   `SWITCHBOARD_INVITE` lines and older pasted invites are JSON. That
   one function covers every caller (facts:14). The URL carries one
   address: `local_address()`'s routed address, bracketed when it is
   IPv6. The dialog already shows only that address. Facts leave which
   address to the owner (facts:71). If the owner says otherwise, the
   fallback is a `.local` name, which is Unresolved (facts:49) and not
   built.
4. **Static paths.** `GET /` (`text/html; charset=utf-8`),
   `GET /app.js` (`text/javascript; charset=utf-8`), `GET /app.css`
   (`text/css; charset=utf-8`). Nothing else: `/index.html`,
   `/favicon.ico` and every other non-API path answer 404 `not_found`
   as today. Each of the three carries `Content-Security-Policy:
   default-src 'none'; script-src 'self'; style-src 'self'; connect-src
   'self'; img-src 'self'; base-uri 'none'; form-action 'none';
   frame-ancestors 'none'` as a header. `connect-src 'self'` covers the
   same-origin `wss:` (facts:30), and `frame-ancestors` must be a
   header (facts:31). They also carry `Cache-Control: no-store`,
   `X-Content-Type-Options: nosniff` and `Referrer-Policy: no-referrer`.
   Every JSON reply also gains `Cache-Control: no-store` and `nosniff`.
   The connection thread answers static files without the frame, the
   way it answers 429. The bytes are compile-time constants with nothing
   of the host in them, so they need no core state and never take one
   of the 8 drains per pass. Facts leave the thread to the owner
   (facts:74). The fallback is answering them in `serve_remote` before
   the `/api/v1/` strip. Static loads count in the unauthenticated pool
   (4) and in a new `page` window of 60 a minute per listener, shared
   by the three paths. A connection moves to the paired pool on its
   first cookie- or bearer-authenticated API request, as today. How many
   connections iOS Safari opens at once is Unresolved (facts:48). The
   page loads three files and then `/info`, so at most four. If the
   device check shows drops, the fallback is `MAX_UNAUTHENTICATED` 4 →
   8 and the spec line with it. §5's rule becomes: before pairing,
   nothing but `/info`, `/pair` and the page's three files, which carry
   nothing of the host. With neither a fragment nor a cookie, the page
   shows `/info`'s `server_name`, whether pairing is open, and "On the
   Mac, open Remote devices, allow pairing and scan the QR" (the owner
   question at facts:77; the fallback is static text alone).
5. **Where the page lives and how it is tested.** `web/index.html`,
   `web/app.js` and `web/app.css` are embedded with `include_str!`
   (the precedent is `src/main.rs:167`). The test checks a fixed list
   of referenced paths. There is no HTML parser (facts:21) and no JS
   test harness, since the page has no build step and no dependency.
6. **Device record.** The page sends `device_name` from
   `navigator.userAgent`: "Safari on iPhone", "Safari on iPad",
   "Safari on Mac", "Chrome on Android", and so on, else "Browser".
   `/pair` already takes it (`src/app.rs:3650`) and caps it at 64
   characters (`src/core/remote.rs:166`). Scope comes from the window as
   today. The device is listed and revocable as any other. There is no
   `kind` field; the name tells devices apart. Facts leave this to the
   owner (facts:80). If the owner wants one, the fallback is an additive
   `#[serde(default)] kind` on `PairedDevice`, with no version
   (facts:16).
7. **The page's scope after a reload: `GET /api/v1/device`.** It
   answers `{device_id, name, scope}` under any scope. The page cannot
   do without it: the cookie is unreadable, and storage is wiped after
   7 days. It is also the request the page uses to learn it was
   revoked. Facts leave this to the owner (facts:87). If the owner says
   no, the fallback is to keep the scope from the `/pair` reply in
   `localStorage`, show the reply box when the scope is unknown, and
   hide it on a `403 scope`.
8. **The cookie reaches every endpoint the bearer does.** There is no
   second allowlist, because scope is the authority and a browser is a
   paired device like any other. Facts leave this to the owner
   (facts:65). If the owner says otherwise, the fallback is a match on
   how the request authenticated, limited to the page's paths.
9. **`/info` lists a `web` capability.** It is additive: the Swift
   client checks `capabilities.contains(..)` (for example
   `client/Sources/SwitchboardClientCore/ServerConnection.swift:431`),
   so an unknown word is ignored.

## The record

No change. A browser is a `PairedDevice` in `devices.json`. The token's
hash is the same whether the token travels as a bearer header or as a
cookie. `identity.json` keeps the certificate `make_identity` wrote:
the served leaf is derived from it at bind and never saved. Nothing new
is rehydrated. A browser's only state is its cookie and the
`{server_id, spki_sha256}` it keeps in storage for that origin. A
change of the Mac's address is a new origin, so the browser pairs again
from a fresh QR. Write that under Known gaps.

## The signals

These change in `src/adapters/remote.rs`.

- `read_request` (:444) keeps three more headers on `RemoteRequest`
  (:360). `cookie: Option<String>` is only the value of the cookie
  named `switchboard-<port>`; other cookies are dropped. `origin:
  Option<String>` and `host: Option<String>` are the raw values.
  `page: bool` is true when `X-Switchboard-Page: 1` is present. The
  adapter learns its port from `Shared`, which gains `port: u16`,
  filled from `listener.local_addr()` in `bind` (:678).
- `connection` (:770) answers the three static paths itself, after the
  rate check, which gains the `page` window in `Limits::limited`
  (:629). They never reach the frame.
- `respond` (:530) takes a content type and extra header lines.
  `RemoteReply` (:389) gains `cookie: Option<String>`, the token to set
  as `Set-Cookie`. The existing constructors gain `cookie: None`.
- **The certificate reissue.** A new `pub fn serving_certificate(identity,
  key_pem, names: &[String], now: SystemTime) -> Result<Vec<u8>, String>`
  makes the leaf over the Keychain key with `rcgen::KeyPair::from_pem`
  (facts:12). It keeps the CN of `make_identity`. The SANs are
  `localhost`, `127.0.0.1`, `::1` and the routed address. rcgen turns
  an IP string into `SanType::IpAddress` (facts:13). It sets
  `extended_key_usages = [ServerAuth]`, `not_before = now − 1 day` and
  `not_after = now + 800 days`, under Apple's 825 (facts:33).
  `tls_config` (:144) gains `names` and serves that leaf. `time` is
  already in the tree at 0.3.55 behind rcgen. Add it as a direct
  dependency (`time = "0.3"`) for `OffsetDateTime::from(SystemTime)`,
  and commit the updated `Cargo.lock`. `bind_remote`
  (`src/app.rs:2936`) passes `local_address()` into the names. If the
  owner answers "no" on naming the IP (facts:59), the SANs drop the
  routed address and keep the rest.

## The actions and effects

There is no new `AppAction` and no new `Effect`. `Paired` and
`RevokeDevice` carry a browser unchanged. The frame changes in
`src/app.rs`:

- `serve_remote` (:3017) routes `("GET", ["device"])` after
  `authenticate` to a new `remote_device`, which returns `{device_id,
  name, scope}` in the envelope.
- `authenticate` (:3127) checks `request.bearer` first, unchanged. Then
  it checks `request.cookie`. A cookie-authenticated request passes only
  when `origin` is absent or equals `https://{host}`, and either it is
  an upgrade whose `origin` equals `https://{host}`, or it carries
  `page`. Otherwise the reply is 403 `cross_origin`, "this request did
  not come from the host's page". The reasoning: a browser always sends
  `Origin` on a WebSocket handshake (RFC 6455 §4.1). A custom request
  header forces a CORS preflight, which the host never answers (Fetch
  standard). Neither is in the facts file. The tests prove the host's
  side of the rule only.
- `remote_pair` (:3606): when the body has `"cookie": true`, the
  request must carry `page`, else 403 `cross_origin`. The reply omits
  `token` and sets `cookie: Some(token)`. Without `cookie` it is
  unchanged.
- `remote_info` (:3568) adds `"web"` to `capabilities`.
- The invite moves out of `src/ui/remote.rs:160-166` into a pure
  function in `src/core/remote.rs`: `pub fn invite_url(identity:
  &Identity, code: &str, address: Option<SocketAddr>) -> Option<String>`.
  It returns `None` without an address. `SocketAddr`'s `Display`
  brackets IPv6.

## The UI

- **The pairing window** (`src/ui/remote.rs:141`) draws its QR from
  `invite_url`. The address is looked up once per code, and the QR
  cache is keyed by the URL, not the code (:168-172). "Copy invite" copies
  the URL. A new line under the code says: "Scan with a phone's camera
  to open the page in its browser. The browser warns that the
  certificate is not trusted. Accepting it on a network you trust is
  the trust step. The key in the code stops the page pairing with
  another host by mistake, not an impostor." The stale hover on Control
  (:218) becomes "Reads, and sending replies into a session".
- **The page** (`web/`, plain JS, no framework):
  - **Load.** Read `location.hash`, call
    `history.replaceState(null, "", "/")`, then `GET /api/v1/info`. If
    the fragment's `key` and `id` match `/info`, post `/pair` with
    `{code, spki_sha256: key, device_name, cookie: true}`; on a
    mismatch, say "This code is for another host" and post nothing.
    Keep `{server_id, spki_sha256}` in storage. Every `fetch` carries
    `X-Switchboard-Page: 1`.
  - **Header.** `server_name`, plus the device's name and scope from
    `/device`.
  - **Live data.** Open `wss://${location.host}/api/v1/events`. After
    `hello`, snapshot `projects`, each project's `sessions` and
    `waiting`. Apply buffered and later `changed` events by per-key
    revision as §4.4 says: refetch the resource the event names when
    its revision is past what the page holds. On `resync_required` or a
    close, reconnect and resync. Do the same on `visibilitychange` to
    visible (facts:47). If the socket fails before `hello`, poll the
    same reads every 5 s.
  - **The list.** Projects with a waiting session come first. Within a
    project, waiting sessions come first and are marked. Each row shows
    kind, liveness and the line from the departure above.
  - **The session view.** A Claude Code session shows `last-response`.
    A running session of any other kind shows `screen`, after a `watch`
    frame and its `watching` answer, and sends `unwatch` on leave.
    Refetch on the key's `changed` and every 30 s while visible, as
    §4.4 says. A stopped non-Claude session shows the note.
  - **Reply.** Shown only under `control`. It posts
    `sessions/{id}/send` with `{op, text}`, where `op` is 16 random
    bytes from `crypto.getRandomValues` as hex. `randomUUID` needs a
    secure context, which facts list as Unresolved (facts:45). A `403
    scope` hides the box.
  - **States.** "Not paired", "Revoked", "Identity changed, pair again"
    and "Pairing is closed" are each said in a sentence. Resume, Trust,
    drafts, tickets and the conversation beyond the last response stay
    out (the issue's Not in this cut).

## The tests

- **Core** (`src/core/tests.rs`): `invite_url` for an IPv4 and an IPv6
  address, and for no address.
- **Adapter unit tests** (`src/adapters/remote.rs` tests):
  - `read_request` keeps only `switchboard-<port>` out of a `Cookie`
    with several cookies, and keeps `Origin`, `Host` and the page
    header.
  - `serving_certificate` over a key has the key's SPKI hash, a SAN for
    each name, the serverAuth EKU, and `not_after` at most 825 days
    out.
- **`tests/remote.rs`**:
  - Change `http` (:245) into `http_with(.., headers: &[(&str, &str)],
    ..) -> (u16, Vec<(String, String)>, String)` and keep `http` as a
    wrapper.
  - New: `the_page_is_served_before_pairing_with_its_headers`. The
    three paths return 200 with their content types, the exact CSP,
    `no-store`, `nosniff` and `no-referrer`. `/index.html`,
    `/favicon.ico`, `POST /` and `/other` return 404 `not_found`.
    `index.html` references exactly `/app.js` and `/app.css`, with no
    inline `<script>`, no `style=`, and no `http:` or `https:` URL.
    Every `"/api/v1/…"` literal in `app.js` begins with one of `info`,
    `pair`, `device`, `projects`, `waiting`, `sessions/` or `events`.
  - New: `a_browser_pairs_by_cookie_and_reads_and_subscribes_with_it`:
    - `/pair` with `cookie: true` and the page header returns a
      `Set-Cookie` with each attribute and a body without `token`.
      Without the header it returns 403 `cross_origin`.
    - `/device` with the cookie returns the scope. Without the page
      header, or with a foreign `Origin`, it returns 403
      `cross_origin`.
    - An events upgrade with the cookie and a matching `Origin` gets
      `hello`. A foreign `Origin` is refused. `subscribe_at` (:1960)
      gains a headers argument.
    - A cookie named for another port is not a token.
    - After `RevokeDevice`, the cookie gets 401 and the socket closes.
  - The existing bearer tests and
    `stalled_strangers_fill_their_own_pool_and_never_a_paired_clients`
    (:3225) pass unchanged. `info_is_served_to_anyone…` (:426) gains
    the `web` capability.
  - No fixture changes: no wire view changes, and the Swift client does
    not read `/device`.
- **`tests/ui.rs`**: `remote_devices_dialog_opens_pairing_and_revokes_a_device`
  (:7424) also finds the trust sentence while the window is open.
- **Swift**: `FixturesTests.swift` gains `testInviteParsesTheURL` next
  to the JSON test at :157. Cover IPv4, `[v6]:port`, and refusal of a
  URL missing `code`, `key` or `id`.
- **Spike 23** (`spikes/23-browser-page/README.md`) records the owner's
  device check, run before the commit: scan, accept the warning, check
  whether `wss://` opens, whether the cookie is kept across a Safari
  relaunch, and how many connections the load opened (facts:53, :45,
  :48). The page works either way, so the commit does not wait on a
  "No". The README says which path iOS took.
- **Acceptance** is on a real phone with the signed bundle. It is not
  in CI.

## Docs

- **`docs/remote-client.md`**:
  - §4.1 bounds: the page window, and static files on the connection
    thread.
  - §4.3: add rows for `/`, `/app.js`, `/app.css` ("none, before
    pairing") and `GET /api/v1/device` (read).
  - §5: the pre-pairing rule, the URL invite with the fragment, the
    browser's trust step and what the key does and does not buy, the
    cookie with its port name and origin check. "never in a URL" stays
    true; "sent only as a bearer header" gains "or, to the host's page,
    as an `HttpOnly` cookie".
- **`docs/design.md`**, Remote client section (from :3107): a new
  bullet, "The browser page (2026-10-xx)", before "Known gaps". Known
  gaps gain:
  - a changed address means re-pairing a browser;
  - iOS socket behaviour per Spike 23;
  - the excerpt is not on the page.
- **`README.md`**:
  - Layout lines for `web/` and `spikes/23-browser-page`.
  - The `tests/remote.rs` line (:437) gains the page and the cookie.
  - The adapter line (:383) gains the static files and the reissue.
  - :277 says "Copy invite" copies the page's URL, which
    `switchboard-remote pair` also takes.
- **`client/README.md:180-190`**: `SWITCHBOARD_INVITE` takes a URL or
  JSON per line.
- **`CHANGELOG.md`** Unreleased: "The pairing QR code opens a page in a
  phone's or tablet's browser that lists the sessions, shows a
  session's last response or screen, and sends a reply."

## Commit

`The remote port serves a browser page, paired from a URL in the QR code`

Contract additions:

- **`src/core/model.rs`, `src/core/action.rs`, `src/ports/`**: none.
- **Protocol** (`docs/remote-client.md`):
  - static `GET /`, `/app.js`, `/app.css`;
  - `GET /api/v1/device` → `{device_id, name, scope}`;
  - `/pair` body field `cookie: bool`; with it, the reply omits `token`
    and sets `Set-Cookie: switchboard-<port>`;
  - request header `X-Switchboard-Page: 1`;
  - refusal code `cross_origin`;
  - `"web"` in `/info` `capabilities`;
  - `Cache-Control: no-store` and `X-Content-Type-Options: nosniff` on
    every reply;
  - the invite as `https://<addr>:<port>/#code=&key=&id=`, with the JSON
    still accepted by `PairingInvite.parse`.
- **Adapter** (not contract):
  - `RemoteRequest.{cookie, origin, host, page}`;
  - `RemoteReply.cookie`;
  - `remote::serving_certificate`;
  - `tls_config` gains `names`.
- **Core helper** (not contract): `core::remote::invite_url`.
