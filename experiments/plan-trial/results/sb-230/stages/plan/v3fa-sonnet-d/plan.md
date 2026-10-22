# Plan for #230: a browser page on the remote port, paired from a URL

References are to the worktree at 7fe8949 and were re-read there, not
copied from the notes. "Settled", "Unresolved" and "For the owner" are
the sections of the facts file. The page is built so that its basic
behaviour (pair, list, read, reply) does not depend on any Unresolved
line; where a line is Unresolved the decision says so.

## Where this departs from the issue

- **"The page opens `/api/v1/events` directly" and the token as a header.**
  `new WebSocket()` takes no headers (Settled, WHATWG), so the events
  socket carries the token as a subprotocol value, decision 2.
- **"The one-line excerpt the card shows" is not on the wire.** The spec
  (`docs/remote-client.md` §4.3) says the excerpt is `last_message` from
  the Stop hook "a field of the session's metadata". The hook event carries
  it (`src/ports/events.rs:24`) but nothing in `src/core` or `src/ports`
  stores it, and `SessionView` (`control/src/lib.rs:636-688`) has no such
  field (facts, Settled: tree). The plan adds no field. The page fetches
  `last-response` for the sessions it shows an excerpt for (below), as the
  headset does; an additive `SessionView.excerpt` is the later change if
  that costs too much, and it is the owner's call (For the owner 9).
- **"The device's scope in the header."** No reply after `/pair` carries
  it: `envelope` (`src/app.rs:3730-3741`) is epoch, revision, body, and
  `/info` is unauthenticated (`src/app.rs:3568-3600`). Spec §5 l.316,
  "the host answers with the device's scope", is not what the code does.
  The page shows the scope from the `/pair` reply and corrects itself on
  the first `403 scope` (`src/app.rs:3166-3172`), hiding the reply box. No
  endpoint is added (For the owner 8); the stale header is a known gap.
- **"Revoked said plainly."** Both refusals in `authenticate`
  (`src/app.rs:3125-3153`) are `401 unauthorized`, so the code cannot say
  revoked. The page does not need it: it holds a token, so a 401 means the
  host no longer knows the token, and it then reads `/info`: a different
  `spki_sha256` than the one it stored is "identity changed, pair again",
  the same key is "revoked". No new code (For the owner 7).
- **The `key` check does not defend the pairing the way §5 says.** Spec
  §5 (`docs/remote-client.md:309`) claims the host's `spki_sha256` check
  makes a relayed pairing fail. For a browser the body is built by
  whatever page the browser loaded, so under an impostor that check is
  fed by the impostor. What the page's comparison (URL `key` against
  `/info`) catches is a genuine page on the wrong genuine host: a second
  installation at that address or port, or a rotated key. The plan
  amends §5 to say so. The issue says roughly this already.
- **"Any path outside `/api/v1/` is refused"** holds (`src/app.rs:3023`),
  but after the request was queued to the frame. The static paths are
  answered before that, on the connection thread, so a page load costs the
  frame nothing (spec §4.1, "the frame does core state only").
- **"The browser then trusts that certificate for that address" is not
  safe to assume of this certificate.** `make_identity`
  (`src/adapters/remote.rs:95-127`) issues no subjectAltName, a CN only,
  and rcgen's year-4096 `not_after`. Apple's rule is a SAN and at most 825
  days; Chrome's is a SAN (Settled, HT210176 and the Chromium intent). Whether
  a no-SAN certificate can be clicked through on iOS is Unresolved. So the
  certificate is reissued with a SAN before any page is served (decision 1).
- **"Cookie versus subprotocol" leaves out where the token sits.** Neither
  option removes `localStorage`: a fetch from script needs the token in a
  header, and a cookie that `fetch` carries on its own rests on the
  Unresolved `SameSite`-on-WebSocket line. Decision 2 takes the path that
  does not.

## The record

One change, to `identity.json`; `devices.json`, `model.rs` and the workspace
records are untouched.

- **`Identity` gains two fields**, both `#[serde(default)]`
  (`src/ports/remote.rs:26-42`): `cert_names: Vec<String>`, the IP
  addresses and names the stored certificate carries as subjectAltName
  entries, and `cert_issued: Option<SystemTime>`. They exist so the host
  can tell a stale certificate without parsing DER (the adapter parses only
  the SPKI, `spki_sha256_of_cert`, `src/adapters/remote.rs:247`).
- **Version.** `IDENTITY_SCHEMA_VERSION` (`src/ports/remote.rs:17`, 1 today)
  becomes the value there at implementation time plus one, because a ticket
  landing in parallel may take the next number first. `SCHEMA_VERSION`
  (`src/core/model.rs:12`, 16 today) and `VIEWS_SCHEMA_VERSION` are not
  touched: nothing in `Workspace` or `Views` changes. `identity.json` has no
  `migrate` step (the identity is loaded by `load_identity`, `src/app.rs:2783`,
  not by `adapters::store::migrate`); an older file reads with an empty
  `cert_names`, which is "stale", which is the migration.
- **Rehydration.** Identity loads as today. At bind, and when a pairing
  window opens, a certificate whose `cert_names` lack the address
  `remote::local_address()` returns (`src/adapters/remote.rs:962-970`), or
  that is older than 400 days, is reissued over the **stored key**
  (`KeyPair::from_pem`, `src/adapters/remote.rs:138-140`; Settled, tree: the
  same `self_signed(&key)` call with a loaded key). `spki_sha256` does not
  move, so every native pin and every paired device holds (spec §5 l.308),
  and `devices.json` is not cleared, unlike the missing-key branch of
  `load_identity` (`src/app.rs:2806-2822`). The key stays Keychain-only.
- **The certificate is issued inside Apple's limits** so the unresolved
  "does the rule bind a user-trusted certificate" does not matter: SAN with
  the IP (and `<host>.local` when the machine's short name is known),
  `serverAuth` EKU, validity 800 days, P-256 as today.
- **On the device the page keeps** one `localStorage` entry:
  `{token, device_id, scope, spki_sha256, server_name}`. `localStorage` is
  per origin and script-readable (Settled, MDN); the CSP below allows no
  script but the page's own. The browser's own certificate exception is the
  browser's; the host keeps nothing per browser beyond the device record.
- **A browser is a `PairedDevice` like the others** (decision 6): the page
  sends a `device_name` made from `navigator.userAgent` ("Safari on
  iPhone"), which `new_device` cuts to 64 characters
  (`src/core/remote.rs:166-175`). No `kind` field; the name is enough.

## The signals

- **The URL.** `https://<host>:<port>/?code=<code>&key=<spki_sha256>&id=<server_id>`.
  The host is `local_address()`'s IP, bracketed when IPv6; every value is
  hex, Crockford or a UUID, so nothing needs escaping. The page's first
  statement reads `location.search` and calls `history.replaceState`
  (Settled, MDN: it rewrites the current history entry) before any
  request. Whether the code reaches history or iCloud tab sync before the
  script runs is Unresolved. The plan does not depend on it: the code is
  single-use and closes the window on `Paired`
  (`src/core/remote.rs:65-66`, Settled), and the failure counter closes it at
  three. The static replies carry `Referrer-Policy: no-referrer` so the
  `?code=` URL is not sent as a `Referer` to `/app.js`.
- **Pairing.** The page reads `/info`, refuses if `key` differs from
  `spki_sha256` (a `wrong_key` post would also count as one of the three
  failures, `src/app.rs:3639-3647`, so it never posts one), posts
  `/api/v1/pair {code, device_name, spki_sha256}`, stores the reply. With
  no code and no token the page shows how to open the window and a form for
  the typed code; it shows `/info`'s key in the dialog's short form for the
  owner to compare by eye, the headset's path (spec §5 l.309).
- **Token carrier.** `fetch` sends `Authorization: Bearer`. The socket
  offers `new WebSocket(url, ["switchboard.v1", "bearer.<token>"])`: both
  are `token` characters (Settled, RFC 6455 §4.1; a base64url token is
  `A-Za-z0-9-_`). The host echoes `switchboard.v1` and never the bearer
  value (Settled: only an offered value may be echoed, and the browser
  fails the handshake with none). A request with no subprotocol is the
  native client's and gets no echo. The token is taken from the
  `Sec-WebSocket-Protocol` header **only on the upgrade to `events`**; on any
  other path only the `Authorization` header counts. The cookie route is not
  built: it rests on the Unresolved `SameSite` question. A device run that
  clears it makes a later change.
- **Cross-origin.** The host checks `Origin` (RFC 6455 §10.2, Settled) on the
  upgrade and on every non-GET: when `Origin` is present it must equal
  `https://<Host header>`, else `403 cross_origin`. This compares against
  the address the browser used, so it holds whichever allowlist the owner
  wants (IP, `.local`, loopback; For the owner 4). Native clients send no
  `Origin` and pass. No cookie is ambient, so this is a second line.
- **Live changes.** The page opens the socket, waits for `hello`, snapshots,
  then reconciles by revision (spec §4.4 "Subscribe first, then snapshot"):
  `waiting`, `projects`, each project's `sessions`; an event whose revision
  is past a snapshot's refetches that resource; a different `epoch`
  refetches everything. `resync_required` reconnects. It sends `watch` for
  the one open session's key (`conversation` or `screen`) and `unwatch` when
  it closes, so it holds one watch, not one per listed session, and does not
  press the 128-key cap (`src/app.rs:1789-1795`; whether many watches evict
  the headset's is the unmeasured one in For the owner 9, avoided).
- **Polling floor.** While the socket is down, or whenever it fails to
  open (a CSP `connect-src 'self'` that a browser does not match to `wss`,
  or a click-through that does not cover `wss`: both Unresolved), the page
  refetches `waiting` and the list every 10 s and says "live updates off".
  Acceptance does not depend on the socket.
- **Excerpts.** The list shows the card word, `waiting_reason` and the
  excerpt for waiting sessions and the first 16 listed, fetched from
  `last-response` unwatched, refetched when that session's `changed` arrives
  or on the 10 s tick.
- **Op ids** for `send` are 16 random bytes from `crypto.getRandomValues`
  in hex, not `randomUUID` (Settled: secure-context only; whether a
  bypassed page is one is Unresolved). A timed-out `send` is checked at
  `/ops/{op}` before it is repeated (spec §4.5).
- **States said plainly.** "Identity changed, pair again" and "revoked" as
  above; "this device reads only" when `scope` was refused; "the host did
  not answer" otherwise.

## The actions and effects

- **Static paths on the connection thread**, before the rate limiter and
  the frame, in `connection` (`src/adapters/remote.rs`): `GET /` (and the
  same with a query), `/app.js`, `/app.css`, `/favicon.ico` (204, no body).
  Anything else outside `/api/v1/` still reaches the frame and is a 404.
  The three files are `web/index.html`, `web/app.js`, `web/app.css`, held
  as `include_str!` constants in a new `src/adapters/web.rs`. Content types
  `text/html; charset=utf-8`, `text/javascript; charset=utf-8`,
  `text/css; charset=utf-8`. Every static reply carries
  `Cache-Control: no-store`, `X-Content-Type-Options: nosniff`,
  `Referrer-Policy: no-referrer` and
  `Content-Security-Policy: default-src 'none'; script-src 'self'; style-src 'self'; connect-src 'self'; img-src 'self'; base-uri 'none'; form-action 'none'; frame-ancestors 'none'`,
  so no inline script and no external resource. The reply says
  `Connection: close`: a page load is three short connections that leave
  the pool of four (`MAX_UNAUTHENTICATED`, `src/adapters/remote.rs:567`)
  at once rather than idling 30 s. Static GETs get a rate window of their
  own, 120 a minute per listener, in `Limits` (`:623-640`), answered 429.
  Whether iOS exhausts the pool anyway is Unresolved; the page's `fetch`
  calls retry once on a network error, and nothing else depends on it.
  The code in `?code=` is never logged: the connection thread logs nothing,
  and `refuse` (`src/app.rs:3754-3772`) already logs path only.
- **`read_request`** (`:444-513`) also reads `Origin`, `Host` and
  `Sec-WebSocket-Protocol`: `RemoteRequest` gains `cross_origin: bool` and
  `offered_protocol: bool`, and `bearer` is filled from the subprotocol
  only for an upgrade to `/api/v1/events`. `respond_upgrade` (`:519-527`)
  echoes `Sec-WebSocket-Protocol: switchboard.v1` when it was offered.
  `RemoteReply` is **not** changed (about 20 literals in `app.rs`, Settled:
  tree), because static replies never reach the frame.
- **`serve_remote`** (`src/app.rs:3017`): a request with `cross_origin` that
  is an upgrade or not a GET is refused `403 cross_origin` before
  authentication.
- **Certificate.** `remote::reissue_certificate(identity, key_pem, names, now)
  -> Result<Identity, String>` in the adapter, and a pure
  `remote::needs_reissue(identity, names, now) -> bool` in `core/remote.rs`.
  `AppAction::CertificateReissued { certificate, names, issued }` replaces
  those fields through `update_identity` (`src/core/remote.rs:109-119`),
  which emits `Effect::SaveIdentity`; the rebind in `run_remote_effect`
  (`src/app.rs:2891`) today fires only when `listen` changed and also fires
  when `certificate` changed. `bind_remote` (`:2936`) asks first, reissues if
  stale, then builds `tls_config` from the new identity. The pairing window's
  open (the arm for `OpenPairing` in `SwitchboardApp::dispatch`) does the same
  check, so a phone on a new address gets a certificate that names it; the
  rebind drops connected devices, which reconnect.
- **Export.** `AppAction::ExportCertificate` and `Effect::ExportCertificate
  { der }`: `app.rs` writes `<data dir>/switchboard-host.cer` (public, atomic
  write) and calls the opener port's `reveal` (`src/ports/opener.rs:8`) so
  the owner can AirDrop it. A fake exists for the opener already.
- **Invite.** `core::remote::invite_url(host, port, key, id, code) -> String`,
  a pure function the pairing window and the tests share.
- **Swift.** `PairingInvite.parse`
  (`client/Sources/SwitchboardProtocol/Info.swift:93-116`) tries a URL
  first (`URLComponents`: `code`, `key`, `id`; `addresses` is the one
  `host:port`), then the JSON as before, so a pasted older invite still
  parses. `switchboard-remote pair` takes either as its one argument
  (`client/Sources/switchboard-remote/main.swift:12-13`, usage text too).
  `addresses` stays a list in the type; every call site (`ConnectionManager`,
  `BrowserView`, `main.swift:120`, eight test files) keeps compiling.
  Decision 3: one QR, the URL; the JSON form is kept only in `parse`.
- **Script line** `SWITCHBOARD_SCRIPT`'s existing `pairing <code>` and
  `remote-listen lan <port>` (`src/script.rs`) already put a test instance
  on the network with a known code; no new line.

## The UI

- The pairing window (`src/ui/remote.rs:155-215`) builds the QR from
  `invite_url`; "Copy invite" copies the URL. Under it, one plain line says
  what the first visit asks of a browser: "The browser will warn about the
  certificate: this host makes its own. Continue past it once, on the
  network you are on now. For a device that refuses, save the certificate
  and install it first." and a "Save certificate" button beside "Copy
  invite" (`ExportCertificate`).
- **Decision 1.** The trust step is the click-through, said in the dialog,
  because it is what the acceptance and the phone's camera do with no
  extra step. What it buys: the browser trusts whichever host answered at
  that address, once. It does not authenticate the host; the `key` check
  above catches a wrong genuine host only. The exported certificate (moved
  from this Mac by AirDrop, so by a channel the owner controls) is the
  stronger step and the fallback if iOS refuses the click-through, which is
  Unresolved (Q1 and Q7 conflict). Both need the SAN certificate, which is
  why it comes first. §5's rule is amended: before pairing, a non-loopback
  bind serves `/info`, `/pair` and the page's four static paths and nothing
  else.
- The page is plain HTML, CSS and JS with no framework or build step. A
  header with host name and scope; sessions grouped by project, waiting
  first and marked; a session view with the last response (Claude Code) or
  the screen (anything else), a reply box under `control` scope, hidden
  otherwise. A session sitting at the folder-trust question or with a
  `prompt_refusal` shows the card's word and no reply box; resume is not in
  this cut (For the owner, last question; the default hides it).
- The paired-devices dialog lists the phone like any device, with Revoke,
  and its text says a browser device holds its token in the browser's
  storage.
- Docs, same commit: `docs/remote-client.md` §5 (the page, the amended
  pre-pairing rule, the corrected relay claim, the subprotocol carrier, the
  Origin rule, the URL invite), §4.1 (the static rate window), §4.3
  (static paths); `docs/design.md` remote status section ("Remote client
  (2026-10-12)", l.3107; its known gaps list: stale scope, the excerpt, the
  unchecked device questions); README Layout (`web/`,
  `src/adapters/web.rs`), the "Copy invite" and `pbpaste` text
  (README l.274-278), and a dev-aid line for opening
  `https://127.0.0.1:<port>/` with `pairing <code>`; `client/README.md` for
  the URL argument; a `CHANGELOG.md` line under Unreleased.

## The tests

- **Core** (`src/core/tests.rs`, `Clock::at`, no sleeping):
  `needs_reissue` for an empty `cert_names`, a missing address, an old
  `cert_issued` and a fresh one; `CertificateReissued` replaces the fields
  and emits one `SaveIdentity`; `invite_url` for IPv4, bracketed IPv6.
- **Adapter** (`src/adapters/remote.rs` unit tests): a reissue over the
  stored key keeps `spki_sha256` (`spki_sha256_of_cert` equals
  `spki_sha256_of_key`) and `tls_config` accepts it; the certificate names
  the IP; an older identity file without the new fields reads and is stale.
- **`tests/remote.rs`**: the `http` helper (`:245-296`) is extended to
  return headers. Static paths answer with the named content types, CSP,
  `no-store`, `no-referrer`; `GET /?code=x` answers the page; `/favicon.ico`
  is 204; `/other` is the frame's 404; static replies work unpaired and do
  not enter the paired pool; the static window answers 429 past 120. Every
  `/api/v1/` literal in the embedded `app.js` (placeholders filled with
  seeded ids), requested with a paired token, is not the frame's 404
  `not_found`, and every `src`/`href` in `index.html` is a served static
  path. The pairing flow end to end with the host's own client: open the
  window, build the URL with `invite_url`, parse its query as the page does,
  read `/info`, compare, post `/pair`, read `waiting`. The events upgrade:
  with `Sec-WebSocket-Protocol: switchboard.v1, bearer.<token>` the reply
  is 101 and echoes `switchboard.v1` only; the bearer value is refused on a
  non-events path; a wrong token is 401 before the handshake; a revoked
  device's socket closes (existing `revoke_closes_the_devices_socket`,
  `:2892`) and its next `fetch` is 401; `Origin` equal to the Host passes,
  any other is 403 on the upgrade and on POST, absent passes. Existing:
  `stalled_strangers_fill_their_own_pool_and_never_a_paired_clients`
  (`:3226`) must still pass unchanged.
- **UI** (`tests/ui.rs`, extending
  `remote_devices_dialog_opens_pairing_and_revokes_a_device`, `:7424`): the
  copied invite is now a URL whose `code` and `key` match; "Save
  certificate" dispatches `ExportCertificate`.
- **Swift** (`swift test` under `client/`; CI does not run Swift): `parse`
  takes the URL and still takes the JSON, with `addresses` holding the one
  `host:port`, and a bracketed IPv6 host.
- **Not tested by CI**: the page's own logic (query removal, reconcile,
  excerpt batching). There is no JS runner and the issue rules out a
  dependency. It is checked by hand on the acceptance run and the plan says
  so in `design.md`.
- **Acceptance** (by hand, with `SWITCHBOARD_DATA_DIR`,
  `SWITCHBOARD_REMOTE_PORT` and `remote-listen lan <port>` + `pairing
  <code>` against a test instance): open the pairing window, scan the QR with
  a phone, continue past the warning **or** install the saved certificate,
  see the sessions; a session that starts waiting moves to the top without
  a reload (by the socket, or within 10 s by the poll if the socket is
  refused); send a reply and watch it arrive; revoke the phone and the page
  says so on its next request. The device-only answers (click-through on
  a SAN certificate, `wss`, `SameSite`, pool of four, history) are
  recorded in `design.md` as found.

## Commit

One commit, README, `docs/remote-client.md`, `docs/design.md` status and
`CHANGELOG.md` updated, the app running at every step (static paths and
the URL invite are additive; a build before the page lands still pairs the
headset with the JSON).

Commit message: `A web page on the remote port, paired from a URL in the QR, over a certificate that names the host`

Contract additions (copy into the commit message):

- `Identity.cert_names: Vec<String>` and `Identity.cert_issued: Option<SystemTime>`
  (both `#[serde(default)]`); `IDENTITY_SCHEMA_VERSION` from the value in
  `src/ports/remote.rs` at implementation time, plus one.
- `AppAction::CertificateReissued { certificate, names, issued }` and
  `AppAction::ExportCertificate`; `Effect::ExportCertificate { der }`.
- `RemoteRequest.cross_origin: bool` and `RemoteRequest.offered_protocol: bool`
  (`adapters/remote.rs`; `RemoteReply` unchanged).
- `core::remote::invite_url`, `core::remote::needs_reissue`,
  `adapters::remote::reissue_certificate`; new `src/adapters/web.rs` and
  `web/index.html`, `web/app.js`, `web/app.css`.
- Wire: static `GET /`, `/app.js`, `/app.css`, `/favicon.ico`; a
  `Sec-WebSocket-Protocol: switchboard.v1` echo and `bearer.<token>` accepted
  on the events upgrade only; refusal code `cross_origin` (403). No
  `SessionView`, endpoint or fixture change.
- Swift: `PairingInvite.parse` accepts a URL and the JSON.
