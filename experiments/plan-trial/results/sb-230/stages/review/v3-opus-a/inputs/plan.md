# Plan: a web control surface on the remote port (#230)

Line numbers are as they are in this worktree at 7fe8949. The facts file
is cited as `facts.md` with its section; "Settled" lines are used as
written, "Unresolved" and "For the owner" lines are named where a decision
rests on one.

The default everywhere below is the design that works whichever way the
open questions are answered: the page pairs and reads over `fetch` with a
bearer header, and treats the events socket as an upgrade it may lose. The
cookie transport and the socket are built, and switch on by a constant
once the owner has run the page on a phone.

## Where this departs from the issue

- **"The one-line excerpt the card shows" does not exist.** The notes and
  `facts.md` (Settled, The tree) say `last_message` is on the session
  record. It is not. It is a field of the hook event
  `EventKind::Stopped { last_message, .. }` (`src/ports/events.rs:24`) and
  the core drops it: `src/core/events.rs:132` matches
  `Stopped { pending, .. }`. `SessionRecord` has no such field and
  `SessionView` (`control/src/lib.rs:637-688`) has none either. So the
  list shows what the wire has: kind, `card` (the card's word), `waiting`
  and `waiting_reason`. The last response appears only in the session view.
  No new read model, no `SessionView` field, no fixture change.
- **"The device's scope in the header" has no source after pairing.** No
  endpoint returns it (`remote_info`, `src/app.rs:3568`, is unauthenticated).
  The page keeps the scope from the `/pair` reply and corrects it when a
  `send` is refused with code `scope` (`src/app.rs:3166-3171`).
- **"Revoked" cannot be told from "never paired" by the code.** Both
  refusals in `authenticate` (`src/app.rs:3127-3152`) are `unauthorized`.
  The page keeps a non-secret marker (server id, key hash, device id, name,
  scope). A 401 with a marker says "this device is no longer paired"; a
  401 without one says "open the pairing window on the host". The host
  adds no code.
- **"Any path outside `/api/v1/` is refused" is true, but late.**
  `serve_remote` does it (`src/app.rs:3023`) after the request took a slot
  in the frame's inbox of 64. The static paths and the 404 for every other
  non-API path are now answered on the connection thread.
- **§5's relay sentence does not hold for a browser.**
  `docs/remote-client.md` §5 says the host's own key check in `/pair` makes
  a relayed request fail. `remote_pair` (`src/app.rs:3636`) compares the
  key in the request body, and under a relay the body comes from the
  impostor's page. The page's `key` check catches a genuine page talking to
  the wrong genuine host (another installation at that address, a rotated
  key). It cannot see the certificate it is on. The spec text is amended
  to say so.
- **`facts.md` says `RemoteReply` is built at about 15 sites.** The tree
  has 11 struct literals: ten in `src/app.rs` (`:1664`, `:1679`, `:2365`,
  `:2371`, `:2376`, `:2754`, `:3570`, `:3723`, `:3736`, `:3767`) and one in
  `src/adapters/remote.rs:829`. No file under `tests/` builds one.
- **An existing test changes.** `tests/ui.rs:7474-7490` parses the copied
  invite as JSON; it becomes a URL.

Decisions the issue asks the plan to settle:

1. **Trust: the click-through is the trust step, said in the dialog; the
   certificate is not offered for download.** A download only helps if the
   device then trusts a manually installed certificate, which needs the
   certificate to meet Apple's rules (`facts.md`, Settled, Apple's TLS
   rules; the tree confirms the current one meets none of SAN, `serverAuth`
   or 825 days: `make_identity`, `src/adapters/remote.rs:102-110`). Whether
   Safari lets the owner proceed at all, and whether the exception then
   covers `fetch` and `wss`, is Unresolved 1 and 2 and For the owner A; the
   plan does not wait on them. It reissues the certificate over the same
   key with the IP and the `.local` name as SANs, `serverAuth` and a
   validity of at most 825 days (Unresolved 1, 2, 3: helps in every
   branch, costs nothing if unneeded). A profile or a download is a later
   issue if A says the click-through is not enough. §5's rule becomes: no
   non-loopback bind serves anything but `/info`, `/pair` and the page's
   three files before pairing; the files are the same bytes for everyone
   and carry no secret.
2. **Token transport: the bearer header on `fetch`, a subprotocol value on
   the socket, the cookie built behind them.** The default needs no browser
   behaviour that `facts.md` leaves open: headers on `fetch` work
   everywhere, and a base64url token is a legal subprotocol token (Settled,
   RFC 6455). The cookie needs Unresolved 5 and For the owner D to be
   favourable, so it is the enhancement (`TRANSPORT` in `app.js`, `'token'`
   by default). The token sits in `localStorage` under the default,
   readable only by this origin's scripts: the CSP (no inline script, no
   external one) leaves the page's own script as the only reader. Under the
   cookie it is not stored at all.
3. **One QR, the URL.** Nothing generates the JSON any more. The Swift
   parser keeps decoding JSON (the visionOS paste field and old pasted
   text), so no client breaks.
4. **Static paths** are in The signals below.
5. **Where the page lives and how it is tested:** `web/`, embedded; the
   tests are in The tests.
6. **The device record: the name only.** The page sends `device_name` from
   its user agent ("Safari on iPhone"); `new_device` already cuts it to 64
   characters (`src/core/remote.rs`, `new_device`). No `kind` field.

## The record

- **`identity.json` gains two additive fields**, `certificate_names:
  Vec<String>` and `certificate_expires: Option<SystemTime>`, both
  `#[serde(default)]`, so the host knows what the stored certificate
  covers without parsing it. `IDENTITY_SCHEMA_VERSION`
  (`src/ports/remote.rs:17`) is bumped: **the value in that file at
  implementation time, plus one** (it is 1 today; a ticket landing in
  parallel may take the next number first). An older file reads with empty
  names, which means "reissue at the next bind". `RemoteFiles::load_identity`
  (`src/adapters/remote.rs:74`) already refuses a newer file. `model.rs`
  and its `SCHEMA_VERSION` (16 today) are untouched; `devices.json` is
  unversioned and unchanged.
- **Rehydration.** The key stays in the Keychain under
  `Identity::key_account`. `bind_remote` (`src/app.rs:2936`) already reads
  the key and builds the TLS config; it now first compares the identity's
  names and expiry with what this machine answers to (the outward IP from
  `local_address()`, `127.0.0.1`, `localhost`, and `<LocalHostName>.local`)
  and a 30-day margin. On a miss it reissues over the same key with
  `rcgen::KeyPair::from_pem` (as `spki_sha256_of_key` does,
  `src/adapters/remote.rs:138-141`), so `spki_sha256` and every pairing are
  unchanged (`facts.md`, Settled: the certificate can be reissued over the
  same key). It saves `identity.json` through the existing atomic path
  directly, as `make_identity` in `app.rs` does at load, then dispatches
  `AppAction::CertificateReissued`. It does not go through
  `Effect::SaveIdentity`: that arm rebinds when `remote_bound` differs
  (`src/app.rs:2895-2898`), and `bind_remote` has just cleared it.
- An address that moves after the bind is not covered until the next
  bind. The pairing dialog says so when `local_address()` is not in
  `certificate_names` and names the fix (turn "Local network" off and on,
  which rebinds); the page's warning interstitial is then the one the owner
  already knows (Unresolved 7: an address change is another warning).
- The page keeps one non-secret marker in `localStorage`: server id, key
  hash, device id, name, scope, host name. The token is a second entry
  under the default transport.

## The signals

No new event. The page speaks the existing protocol: `GET info`, `POST
pair`, `GET waiting`, `GET projects`, `GET projects/{id}/sessions`, `GET
sessions/{id}/last-response` (`screen` for a non-Claude kind), `POST
sessions/{id}/send`, `GET ops/{op}`, and the `events` socket with `watch`
frames for the content key (`conversation` for `last-response`, `screen`).

- **Reads and revisions.** Subscribe first, then snapshot (§4.4). Each
  reply carries `epoch` and `revision`; a reply older than what the page
  holds for that resource is dropped, a new `epoch` refetches everything,
  `resync_required` reconnects and refetches.
- **Polling is the floor, the socket the upgrade** (`facts.md`
  Unresolved 1; the plan never waits on it). The list and the open
  session are refetched every 3 s when no socket is connected and every 30
  s when one is (§4.4's visible-view refetch). A socket that fails to
  open, or closes, shows "live updates unavailable" and the page carries
  on polling; it retries the socket every 30 s. Acceptance step 4 (a
  session moves to the top without a reload) therefore holds on either
  branch, within 3 s without the socket and at once with it.
- **Operation ids.** `crypto.randomUUID()` only when
  `window.isSecureContext`, else 16 bytes of `getRandomValues` as hex
  (Unresolved 4). A `send` that times out is checked with `GET ops/{op}`,
  never resent (§4.5).
- **The pairing URL.**
  `https://<ip>:<port>/?code=<code>&key=<spki_sha256>&id=<server_id>`, an
  IPv6 host in brackets. The host part is the outward IP from
  `local_address()`; the `.local` name goes in only if the owner answers
  For the owner B favourably, a one-line change in the dialog, since the
  certificate already covers both (Unresolved 9). On load the page, before
  any request: reads the query, calls `history.replaceState(null, '',
  location.pathname)`, then `GET /api/v1/info`, then compares `key` with
  `spki_sha256` and `id` with `server_id`, and posts `/pair` only on a
  match. On a mismatch it says "this is not the host whose code was
  scanned" and sends nothing. With a marker whose server id matches but
  whose key differs from `/info`'s, it says "identity changed, pair again"
  and sends nothing, token included (§5).
- **The code in history.** Unresolved 6 and For the owner C: whether the
  browser records the URL before `replaceState` is unknown. The plan
  assumes it does. The code is single use and the window closes at the
  first pairing, three wrong codes or five minutes (`facts.md`, Settled:
  `src/core/remote.rs:46-72`), so a recorded URL is dead after pairing; the
  dialog says that. The connection-thread handler never logs a query
  (`read_request` has already split it off, `src/adapters/remote.rs:495`)
  and `refuse` logs path only (`src/app.rs:3762-3766`).
- **Static paths, on the connection thread** (the frame does core state
  only, §4.1): `GET /` (`text/html; charset=utf-8`), `GET /app.js`
  (`text/javascript; charset=utf-8`), `GET /app.css` (`text/css;
  charset=utf-8`). Each answers with `Cache-Control: no-store`,
  `X-Content-Type-Options: nosniff`, `Referrer-Policy: no-referrer`,
  `Connection: close`, and `Content-Security-Policy: default-src 'none';
  script-src 'self'; style-src 'self'; connect-src 'self'; base-uri 'none';
  form-action 'none'; frame-ancestors 'none'`. No inline script or style,
  no external resource. If a browser's `'self'` does not cover `wss:`, the
  socket is blocked and the page is on its polling floor, which it already
  handles. Other methods on these paths: 405 with `Allow: GET`. Any other
  path outside `/api/v1/` (a `/favicon.ico` included): 404 with the
  existing `not_found` body, also on the thread. Paths under `/api/v1/`
  go to the frame as today.
- **Pool and rate (`facts.md` Unresolved 8).** A static request is
  answered with `Connection: close`, so its slot in the pool of four
  (`MAX_UNAUTHENTICATED`, `src/adapters/remote.rs:567`) is freed as soon as
  the reply is written; a page load is three short requests. Static paths
  get their own window, `STATIC_PER_MINUTE = 120` in `Limits`
  (`:623-639`), so a reloading browser cannot spend `/pair`'s six a minute
  or `/info`'s sixty. Whether an iPhone's parallel connections still
  overrun the pool at accept (`:699-706`) needs a device; it is on the
  owner's run list, and the plan raises no limit on a guess.
- **Without a code and without a token** the page says: "Open the pairing
  window on the host (Settings, Remote devices, Allow pairing) and scan
  its QR code." No code-entry form: pairing needs the key, which only the
  link carries.
- **Origin.** Cross-site hijack needs an ambient credential, and the
  default transports are not ambient. The host still checks `Origin`
  where an ambient one is in play: on the events upgrade, and on any
  request authenticated by cookie, `Origin` when present must equal
  `https://<Host header>` (the same origin by IP, `.local` name or
  loopback alike), else 403 `origin`. Native clients send no `Origin` and
  are unaffected.

## The actions and effects

- `AppAction::CertificateReissued(Identity)`: the core replaces the
  stored identity's certificate, names and expiry, keeping `server_id`,
  `server_name`, `listen` and `spki_sha256`. It pushes no effect: the file
  is already written (above). A core test in `src/core/tests.rs`.
- No new `Effect` and no new port method: the reissue uses
  `services.remote.save_identity` and `services.secrets.get`, which exist.
- **Adapter (`src/adapters/remote.rs`).**
  - `RemoteRequest` (`:360`) gains `host`, `origin`, `subprotocols:
    Vec<String>` and `cookie_tokens: Vec<String>`. `read_request`
    (`:444-514`) fills them: `Sec-WebSocket-Protocol` split on commas, a
    `bearer.<token>` entry taken as the token when no `Authorization`
    header is present, and every cookie named `sb_*` collected (the rest of
    the `Cookie` header is dropped, never stored or logged, since cookies
    are shared across ports on a host, RFC 6265 per `facts.md`). Only the
    adapter builds `RemoteRequest` (`facts.md`, Settled), so nothing else
    changes. It is not in `ports/` or `model.rs`, so CLAUDE.md's additivity
    rule does not name it; the additions are additive anyway.
  - `RemoteReply` (`:389`) gains `headers: Vec<(String, String)>` and a
    constructor `RemoteReply::new(status, body, authenticated)`; the 11
    literals above call it. Only `/pair` sets a header (`Set-Cookie`).
  - `respond` (`:530`) takes extra headers; a new `respond_static` writes
    the three files. `respond_upgrade` (`:519`) echoes
    `Sec-WebSocket-Protocol: switchboard.v1` when the request offered it
    (RFC 6455: the client fails on a subprotocol it did not offer, and the
    server never echoes the token entry; `facts.md`, Settled). A native
    client offers none and gets none.
  - `connection` (`:770`) answers static and non-API paths before the
    frame, after the rate check.
  - `make_identity` (`:95`) and a new `reissue_certificate(identity, key_pem,
    names, now)` share one certificate builder: SANs, `serverAuth`,
    `not_after` = now + 825 days minus a day. `time` may need to become a
    direct dependency for `OffsetDateTime`; check whether `rcgen` re-exports
    it. `certificate_names()` and `invite_url(ip, port, code, key,
    server_id)` are pure functions beside `local_address()` (`:962`).
- **Router (`src/app.rs`).**
  - `authenticate` (`:3127`) reads `request.bearer`, then
    `cookie_tokens`, and applies the origin rule for the cookie case.
  - `remote_subscribe` (`:2723`) applies the origin rule.
  - `remote_pair` (`:3606`) honours an optional `"cookie": true` in the
    body: the reply keeps the token in the body (native clients read it) and
    adds `Set-Cookie: sb_<first 8 hex of server_id>=<token>; HttpOnly;
    Secure; SameSite=Strict; Path=/api/v1/; Max-Age=31536000`. The name
    carries the server id so two instances on one Mac do not clash.
  - `bind_remote` (`:2936`) reissues first.
  - The `/api/v1/` prefix arm (`:3023`) stays as the backstop.
- **New files:** `web/index.html`, `web/app.js`, `web/app.css`, and
  `src/adapters/web.rs` (`include_str!("../../web/...")`, one lookup
  function from method and path to a file, and the header set). No build
  step, no dependency for the page.

## The UI

- **The page** (`web/`): the header shows the host name and the device's
  scope; sessions are grouped by project, waiting ones first and marked,
  each row with its kind, `card` word and `waiting_reason` (`hide_workers`
  already applies, `src/app.rs:2687`); a session view shows the last
  response (Claude Code) or the screen (anything else), refreshed per The
  signals; a reply box posts `send` when the scope is `control` and is
  hidden otherwise. Plain statements for "identity changed, pair again",
  "this device is no longer paired", "live updates unavailable". A
  `prompt_refusal` or `trust_question` session shows its reason and no
  reply box; resume and the trust answer are not in this cut.
- **The pairing window** (`src/ui/remote.rs:155-206`): the QR and "Copy
  invite" carry the URL, the `json!` call at `:160-166` is replaced by
  `remote::invite_url`, and the address label shows the URL's host and
  port. Under it, three sentences: a browser shows a certificate warning
  once, and accepting it on the local network is the trust step (the page
  compares the key but cannot see the certificate it is on); the link holds
  the code, which works once and for five minutes, so a copy left in the
  browser's history is dead after pairing; the certificate is renewed over
  the same key, so a renewal does not unpair anyone. When
  `local_address()` is not in `certificate_names` it adds the restart note.
- **The paired-devices list** (`:230-275`) needs no change: a browser is a
  row with a scope selector and Revoke, and revoking closes its socket
  (existing test `revoke_closes_the_devices_socket`, `tests/remote.rs:2892`).
- **Swift text path:** `PairingInvite.parse`
  (`client/Sources/SwitchboardProtocol/Info.swift:93-116`) tries a URL first
  (`URLComponents`: host and port into `addresses`, `code`, `key`, `id`)
  and JSON second, so `main.swift:111`, `BrowserView.swift:106,883` and
  every `PairingInvite(...)` call (`facts.md`, Settled: it is built as a list
  of one) work unchanged. The CLI usage string (`main.swift:13`) and the
  README's "Copy invite" text say the QR holds a URL.

## The tests

Rust (the commands in CLAUDE.md; Linux CI runs them):

- `src/adapters/remote.rs` unit tests: a reissued certificate keeps
  `spki_sha256` (`spki_sha256_of_cert` equals the key's hash); its DER
  holds the SAN names and the `serverAuth` OID; `certificate_expires` is at
  most 825 days out; reissue is skipped when names and margin are met;
  `invite_url` for IPv4 and a bracketed IPv6 host; `read_request` takes
  the three token transports and drops other cookies.
- `src/core/tests.rs`: `CertificateReissued` replaces certificate fields
  and keeps `spki_sha256`, `server_id` and `listen`, and returns no
  effect.
- `tests/remote.rs`: extend `http` (`:245`) to return headers (a thin
  wrapper keeps current callers). Then: the three paths answer with the
  headers named above; a path with `?code=` answers the page and reaches no
  frame; other methods get 405; `/favicon.ico` gets 404 without a frame
  pass; the static window answers 429 past 120; four sequential static
  loads leave the pool free (model: `stalled_strangers_fill_their_own_pool_and_never_a_paired_clients`,
  `:3226`); the embedded `index.html` names only `/app.js` and `/app.css`,
  has no inline script and no `http` URL, and every `/api/v1/...` literal
  in `app.js` is an endpoint the host serves. The pairing flow as the page
  does it, with the host's own client as the CLI does: `/info`, key
  compare, `/pair` with a browser-style `device_name`, a read with the
  bearer header; a socket upgrade offering `switchboard.v1, bearer.<token>`
  gets 101 echoing `switchboard.v1` and not the token (extend
  `subscribe_at`, `:1960`); `/pair` with `"cookie": true` sets the cookie
  with `HttpOnly; Secure; SameSite=Strict`, a cookie-authenticated read
  works, a cookie-authenticated request with a foreign `Origin` gets 403,
  a bearer request with a foreign `Origin` is unaffected; after
  `RevokeDevice` the next request is `unauthorized` and the socket closes.
  `pair_is_rate_limited_per_listener` (`:1406`) and
  `the_allowlist_refuses_everything_else_with_no_side_effect` (`:682`) stay
  as they are and still pass.
- `tests/ui.rs:7474`: the copied invite is a URL with `code`, `key` and
  `id` equal to the window's code, the identity's key hash and server id;
  the dialog shows the trust sentences.
- No fixture change: no view or wire body changes. The Swift fixtures stay
  as they are.

Swift (`client/`, run as `client/README.md` says; not run by CI on
Linux): `FixturesTests.swift:156` gets URL cases beside the JSON one, an
IPv6 host, and a URL with an unknown query key.

The page's own logic (query parse, `replaceState`, reconcile) has no
runner and the issue rules out a JS dependency, so it is checked by the
manual run below, and the embedded-page test above is the guard against
the page naming a path the host does not serve.

Manual acceptance, against a test data dir and tmux socket
(`SWITCHBOARD_DATA_DIR`, `SWITCHBOARD_TMUX_SOCKET`, script lines
`remote-listen lan <port>` and `pairing <code>`): open the window, scan
the QR with a phone, accept the warning, see the sessions; start a session
waiting with `switchboard-hook PermissionRequest` and watch it move to the
top without a reload (within 3 s if the socket is unavailable); send a
reply from the phone with `control` scope and watch it arrive; revoke the
phone in the dialog and the page says so on its next request. None of
these steps depends on the socket, the cookie or the `.local` name. The
owner's run also answers For the owner A through D, recorded in the design
status section: whether the page loads after the warning, whether `wss`
connects, whether `.local` resolves, whether the cookie works. A, B and D
each flip one constant (`TRANSPORT`, the host in the URL) and add no code.

## Docs, in the same commit

- `docs/remote-client.md`: §4.1 gets the static-file bullet and the
  thread it runs on; §4.3 lists the three paths; §5 changes "Require
  pairing before anything but `/info` and `/pair`" to add the page's files,
  the QR line to the URL, the token line to the three transports, and the
  relay sentence as in the departures; the certificate line gets the SANs,
  `serverAuth` and the 825-day bound, with the pin unchanged by a reissue.
- `docs/design.md`: a "Web control surface" item in the Remote client
  section (starts `## Remote client (2026-10-12)`, `docs/design.md:3107`),
  with the Known gaps at the end of that section (`:3573`) extended: no
  excerpt on rows, scope stale until a refused send, no certificate
  download or profile, no JS tests, an address change needs a rebind,
  Unresolved 1 through 9 and A through D as open until the phone run.
- `README.md`: Layout rows for `src/adapters/web.rs` and `web/*`; the
  `SWITCHBOARD_REMOTE_PORT` paragraph's "Copy invite" text now says URL, and
  that opening it in a browser pairs it; the data directory entry for
  `identity.json` mentions the certificate names.
- `CHANGELOG.md`, under Unreleased: "The remote port serves a web page:
  open the pairing window's QR code on a phone or any browser on the local
  network to see the sessions, which are waiting, and reply."

## Order inside the one commit (the app builds and runs after each)

1. `web/`, `src/adapters/web.rs`, the connection-thread static answers,
   the static window, `RemoteReply::new`, the extended `http` helper.
2. The `RemoteRequest` fields, the three token transports, the subprotocol
   echo, the origin rule, the cookie on `/pair`.
3. The certificate builder, `reissue_certificate`, the two `Identity`
   fields and the version bump, `CertificateReissued`, the `bind_remote`
   call.
4. `invite_url`, the dialog, the Swift parser.
5. The tests and docs above; `cargo fmt --all`, `cargo clippy --locked
   --workspace --all-targets -- -D warnings`, `cargo test --locked
   --workspace`.

## Commit message

Serve a browser page from the remote port, paired from a URL in the pairing QR

## Contract additions (for the commit message body)

- `AppAction::CertificateReissued(Identity)` in `src/core/action.rs`; no
  new `Effect`, no new port method.
- `Identity.certificate_names: Vec<String>` and
  `Identity.certificate_expires: Option<SystemTime>` (`#[serde(default)]`)
  in `src/ports/remote.rs`; `IDENTITY_SCHEMA_VERSION` set to the value in
  that file at implementation time, plus one. No change to `model.rs` or
  `SCHEMA_VERSION`; `devices.json` unchanged.
- `RemoteRequest` fields `host`, `origin`, `subprotocols`, `cookie_tokens`;
  `RemoteReply.headers` and `RemoteReply::new` (both in
  `src/adapters/remote.rs`, outside the shared contract).
- `/pair` body: optional `cookie` (bool). `/pair` reply: optional
  `Set-Cookie`, the body unchanged.
- Events upgrade: a `Sec-WebSocket-Protocol` of `switchboard.v1` is echoed;
  `bearer.<token>` is accepted as a token carrier. Refusal code `origin`
  (403) for a cookie-authenticated request or an upgrade whose `Origin` is
  not the host's own.
- Served paths: `GET /`, `/app.js`, `/app.css`; every other non-API path
  is 404 on the connection thread.
- New files `src/adapters/web.rs`, `web/index.html`, `web/app.js`,
  `web/app.css`; new pure functions `remote::invite_url`,
  `remote::certificate_names`, `remote::reissue_certificate`.
- Swift: `PairingInvite.parse` accepts a URL as well as JSON; no new type
  and no changed initializer.
- Wire bodies, `control/`, and the shared fixtures: no change.
