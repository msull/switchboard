# Issue #230 — a web control surface on the remote port: investigation notes

Read: CLAUDE.md, docs/design.md (Remote client section, line 3107 on), docs/lessons.md,
docs/remote-client.md §4.1, §4.3-4.5, §5, and the code below. Nothing was written to the repo.
Line numbers are as in this worktree (branch dispatch/230-..., clean at 7fe8949).

## 1. On-disk record and rehydration (before the pane)

A browser is a paired device, so it already has a record and needs no new store.

- **Record**: `PairedDevice` (`src/ports/remote.rs:96`): id, name, scope, `token_sha256`,
  created, last_seen. Saved in `<data dir>/devices.json` through `RemoteFiles::save_devices`
  (`src/adapters/remote.rs:83`), the store's atomic path. The token is never stored, only its hash.
  The host's identity is `identity.json` plus the key in the Keychain; the cert is made in
  `make_identity` (`src/adapters/remote.rs:95`).
- **Schema**: no change needed if the browser sends `device_name` ("Safari on iPhone", from
  `navigator.userAgent` in the page) the way the headset does. `remote_pair` (`src/app.rs:3606`)
  already takes `device_name` and defaults to "Device". Only if a `kind: "web"` marker is wanted in
  the dialog does `PairedDevice` gain a `#[serde(default)]` field, which is a `SCHEMA_VERSION`
  question for `devices.json` (check how `IDENTITY_SCHEMA_VERSION` and `migrate` treat that file).
  I recommend not adding one.
- **Rehydration, host side**: nothing new. The page is stateless on the host. After a restart the
  host re-reads `devices.json` and `device_by_token` (`src/core/remote.rs:145`) accepts the same
  token hash, so a phone stays paired across host restarts. The `epoch` changes on restart and the
  page refetches (§4.4 already says so).
- **Rehydration, device side** (the part that is new). The device's "record" is:
  1. an `HttpOnly; Secure; SameSite=Strict` cookie holding the token (decision 2), which the script
     never sees;
  2. a non-secret `localStorage` entry `{server_id, spki_sha256, name, paired: true}`, written at
     pairing. Without it the page cannot tell "never paired" from "revoked" or "identity changed",
     because it cannot read the cookie. Reload = read that entry, fetch `/api/v1/info`, compare
     `server_id` and `spki_sha256` (mismatch = "identity changed, pair again" and send nothing), then
     call a read endpoint; 401 with `paired: true` = "revoked" (clear the entry), 401 without = the
     unpaired screen.
  3. The page cannot learn its scope after a reload without a new read. The scope is only in the
     `/pair` reply today, and a dialog change takes effect on the next request. Either add an
     additive `GET /api/v1/me` returning `{device_id, name, scope}` (an allowlist addition, read
     scope, goes into §4.3), or send `scope` in an `X-` header on every reply. I recommend `/me`.
     It is also the cheapest "am I still paired" probe and the thing the revoke test polls.
  Everything else (sessions, last response, screen) is rehydrated by REST plus the events socket
  exactly as §4.4 says: subscribe, wait for `hello`, snapshot, apply buffered events by revision.

## 2. Code the issue touches (current state)

Host / transport
- `src/adapters/remote.rs:437-516` `read_request`: head cap 8 KiB, body 64 KiB. Extracts only
  `authorization: Bearer` (line 484) and the websocket key. It does not keep `cookie`, `origin`,
  `user-agent` or `host`. A cookie token and an Origin check on the upgrade both need `RemoteRequest`
  (`:360`) extended additively (e.g. fill `bearer` from the cookie when there is no header, add
  `origin: Option<String>`).
- `src/adapters/remote.rs:530` `respond`: hardcodes `Content-Type: application/json` and has no way
  to add headers (no `Set-Cookie`, no CSP, no `Cache-Control`). `RemoteReply` (`:389`) is
  `{status, body, authenticated}`. Needs an additive `content_type` and `headers` (or a
  `ReplyKind::Static`). The reason table at `:532` lacks 301/304 if used.
- `src/adapters/remote.rs:567-587` pools and rate windows: `MAX_UNAUTHENTICATED = 4`, `MAX_PAIRED = 12`,
  `/pair` 6 per minute and `/info` 60 per minute, keyed by exact path in `Limits::limited` (`:631`).
  Static paths fall through to "not limited".
- `src/adapters/remote.rs:770-870` `connection`: a connection counts as unauthenticated until a reply
  has `authenticated: true` (`:836`), then moves to the paired pool; a request with a token that is
  refused stays unauthenticated. The first read timeout is 5 s, then 30 s idle.
- `src/app.rs:3017-3030` `serve_remote`: line 3023 `strip_prefix("/api/v1/")` is the allowlist's
  front door; anything else gets 404 `not_found`. `GET /` and `/web/*` must be answered before that
  line. `authenticate` (`src/app.rs:3127`) reads only `request.bearer`. `remote_info` (`:3568`),
  `remote_pair` (`:3606`; reply `{token, scope, server_id, device_id}`, built at `:3656-3664`),
  `remote_subscribe` (`:2723`) take `device` from the same authentication, so cookie-as-bearer needs
  no change below `authenticate`.
- `src/app.rs:2962` bind: loopback unless `identity.listen.lan`, then `0.0.0.0`.
- `src/app.rs:3317`-ish `refuse`/`envelope` (`:3730`, `:3754`): errors are JSON; the page's 401 and
  403 handling reads `error.code` (`unauthorized`, `pairing_closed`, `wrong_key`, `wrong_code`).

Pairing window / QR
- `src/ui/remote.rs:160-166` the payload is the JSON `{server_id, spki_sha256, code, addresses}`,
  `qr_texture` at `:280`, "Copy invite" at `:205`. `address` is one `ip:port` from
  `remote::local_address()` (`src/adapters/remote.rs:962`). `src/ui/remote.rs:229-276` is the
  paired-devices dialog (scope combo, Revoke); `:140-160` text "Scan, or type the key and the code."
- `src/core/remote.rs`: pairing window state (`pairing_at` `:152`), `new_device` `:166`,
  `RevokeDevice` `:86`, `ClosePairing` `:55`.
- Native text path: `client/Sources/SwitchboardProtocol/Info.swift:91-116` `PairingInvite.parse`
  decodes JSON only; `client/Sources/switchboard-remote/main.swift:107-125` `pair` takes either one
  argument (the JSON) or three. Tests: `client/Tests/SwitchboardClientCoreTests/PairingTests.swift`.

Tests / docs
- `tests/remote.rs`: `http()` `:245` (pinned TLS client, only `Content-Length` parsed, headers are
  dropped, so it must return headers for the new tests), `call` `:299`, `pair` `:406`,
  `info_is_served_to_anyone...` `:426`, `the_allowlist_refuses_everything_else...` `:682` (it asserts
  404 for non-`/api/v1/` shaped paths such as `/api/v2/projects`; keep it passing), subscriber helpers
  `:1932-2150`, revoke `:658`, fixtures `:1587`; fixtures dir `client/Fixtures/*.json`.
- `docs/remote-client.md` §4.1 (bounds, list at line ~125-131), §4.3 table (line 157), §5 (line 302),
  §0 revision notes, §11 tests. `docs/design.md` "Remote client" status section (`:3107-3585`).
  README Layout table (`README.md:383` for `adapters/remote.rs`, `:423` `ui/remote.rs`, `:437` tests).
- No `web/` directory exists. The only `include_str!`/`include_bytes!` in the crate are the icon and fonts
  (`src/main.rs:167`, `src/ui/theme.rs:203`), so `include_str!("../../web/index.html")` is new but
  conventional. No `[dependencies]` change is needed.

## 3. Findings that change the plan

1. **The browser trust step cannot be what the issue hopes.** The page compares the URL's `key` with
   `spki_sha256` in the `/info` body. Both are strings the page gets over the same TLS connection;
   a browser cannot see the connection's actual public key. The check therefore catches only a stale or
   mistyped QR pointing at a different host. It does not catch a man in the middle who terminates TLS
   and relays or serves his own page. State that in the spec and the dialog.
2. **Decision 1: click-through, not a certificate download. Recommended.** Reasons: a downloadable
   cert means installing and then enabling full trust per device (iOS: profile + Settings > General >
   About > Certificate Trust Settings), a much heavier and scarier flow, and the cert would then be
   trusted for any name by that device's roots store, which is a worse blast radius than an exception
   for one address. The click-through gives a per-address exception and nothing else. The cost: the
   owner must compare something. Put the 8-hex short key (`short_hash`, `src/ui/remote.rs`) on the
   pairing window (already there) and have the page show the same short form after load, so the owner
   can check by eye that the host the browser reached shows the key the Mac shows. Say plainly in the
   paired-devices dialog: "a browser trusts this host by your click on the certificate warning, on a
   network you trust; compare the key shown on the page with the one here."
   Open risk to settle in a spike (below): the generated certificate has **no SubjectAltName**
   (`make_identity`, `params = CertificateParams::new(Vec::new())`, `src/adapters/remote.rs:100`).
   Chrome and Safari still offer "proceed" on a name mismatch, but a certificate with an IP SAN of
   the LAN address gives a milder warning. The pin is the key, not the cert, so reissuing the
   certificate with SANs does not invalidate pairings. Decide: add the current LAN IP and `localhost`
   as SANs at serve time (needs the certificate regenerated when the address changes, which
   `a_change_of_address_rebinds_the_port` at `tests/remote.rs:1494` shows is already a case).
3. **Decision 2: cookie.** Recommended: `/pair` takes an extra body field (e.g. `"session": "cookie"`),
   replies with `Set-Cookie: __Host-sb=<token>; Path=/; Secure; HttpOnly; SameSite=Strict; Max-Age=...`
   and omits `token` from the JSON body, so a script never holds it. The header-bearer path stays for
   native clients (their reply body is unchanged, so the Swift fixtures `pair.json` still decode).
   `authenticate` takes the token from the header, else the cookie. Consequences to design for:
   - cookies attach to every request, so a cross-site `POST /api/v1/sessions/{id}/send` is a CSRF
     vector. `SameSite=Strict` handles cross-site, but cookies are not isolated by port: any other
     service on the same IP (another port, `localhost` dev servers) shares the jar. So also require,
     for cookie-authenticated unsafe methods and for the WebSocket upgrade, that `Origin` equals
     `https://<Host header>` (and require `Content-Type: application/json`, which a cross-origin form
     cannot send). Cross-site WebSocket hijacking is real: the upgrade carries the cookie.
     That needs `origin` and `host` in `RemoteRequest`.
   - `Sec-WebSocket-Protocol` smuggling of the token is rejected: it puts the token in script,
     in logs of proxies, and violates "never in a URL or readable"; cookie is better.
   - `__Host-` prefix requires `Secure`, `Path=/`, no `Domain`; whether Safari stores a `Secure`
     cookie from an origin with a click-through certificate must be verified on a real iPhone and
     Chrome (spike). If it does not, the fallback is `localStorage` plus a one-shot
     ticket for the socket (a `POST /api/v1/events/ticket` that returns a 30-second single-use value
     used as `Sec-WebSocket-Protocol`). Write that fallback down in the spec as the reason to spike first.
   - Revocation: removing the record makes the cookie's hash unknown; the next request is 401
     `unauthorized`, the socket is closed by the existing path (test `:2099` area).
4. **Decision 3: one QR, a URL.** Replace the QR text with
   `https://<address>/?code=<code>&key=<spki_sha256>&id=<server_id>` (bracket IPv6; percent-encode
   not needed for the alphabet). Native text paths parse the URL **or** the JSON for one release:
   `PairingInvite.parse` (`Info.swift:114`) tries JSON then `URLComponents`; `addresses` is the URL's
   host:port. Nothing needs the JSON once those two accept the URL, but keep JSON parsing in Swift
   because pasted old invites and the fixture `PairingTests` use it; drop it from the QR and from
   "Copy invite" only. The Swift side's change is small but real: `Info.swift`, `main.swift:107`
   usage text, the headset's paste field in `client/Sources/SwitchboardUI`, and a new Swift test.
   Rust side: `src/ui/remote.rs:160`.
5. **The code travels in a GET to the host.** The browser requests `/?code=...`, so `read_request`
   parses it into `query` (`:497`). `serve_remote` must ignore the query on `GET /` and log nothing of
   it (`refuse` logs only method and path, good; the access path must keep it that way). The page
   sets `Referrer-Policy: no-referrer`, calls `history.replaceState` first, and the code is single-use
   and five minutes, which bounds a camera app's or browser's history copy of the URL.
6. **Static paths and the pools (decision 4).** Pool facts: a browser opens up to six parallel
   connections to load a page and its files; the unauthenticated pool is 4 and a fifth accept is
   closed at once (the accept loop in `RemotePort::bind`, after `src/adapters/remote.rs:700`). A single
   inlined document is out, since the CSP forbids inline script. So: three files, `/`, `/app.css`, `/app.js`, and tell the page nothing else is fetched;
   with HTTP keep-alive the browser reuses one or two connections, and a 5th concurrent connection is
   closed and retried by the browser. State that in the spec and test with a browser. Alternative:
   count only API requests against the unauth pool, which needs the pool charge to move to the first
   non-static request. I recommend leaving the pools as they are and measuring first.
   Concrete set (exact match, `GET` and `HEAD` only, anything else 404/405):
   - `/` and `/index.html` -> `text/html; charset=utf-8`
   - `/app.css` -> `text/css; charset=utf-8`
   - `/app.js` -> `text/javascript; charset=utf-8`
   - no favicon (404; a `<link rel=icon href="data:,">` keeps the browser from asking)
   Headers on all: `Content-Security-Policy: default-src 'none'; script-src 'self'; style-src 'self';
   connect-src 'self' wss://<host>` (use `'self'`; WebSocket `'self'` support varies in Safari, so write
   `connect-src 'self'` and test; if Safari refuses, add the explicit `wss:` origin from the `Host`
   header at serve time), `img-src 'self' data:`, `base-uri 'none'`, `form-action 'none'`,
   `frame-ancestors 'none'`; `X-Content-Type-Options: nosniff`; `Referrer-Policy: no-referrer`;
   `Cache-Control: no-store` (a version number in the URL is not needed, since nothing is cached);
   `Cross-Origin-Opener-Policy: same-origin`. Never `Set-Cookie` on a static reply.
   Rate window: add `PAGE_PER_MINUTE = 60` per listener for the static paths, in `Limits::limited`
   (`:631`), same shape as `info`, answered 429 on the connection thread. Static replies are
   `authenticated: false`, so a browser's connection stays in the unauthenticated pool until its first
   authenticated API call moves it (`:836`); that is the existing, intended accounting.
   Unpaired and no code: the page shows host name (from `/info`), "Not paired", and "Open the
   pairing window on the Mac (Settings > Remote devices) and scan its QR code", plus `pairing_open`.
   With a code but pairing closed (403 `pairing_closed`) say so; wrong key says "this is not the host
   whose code was scanned".
   **Rule to keep (§5): the host serves only `/info`, `/pair` and the page's own files before pairing.**
   `serve_remote` serves static paths before `authenticate` and nothing else; the allowlist test must be
   extended with paths that are NOT in the set (`/app.js.map`, `/../Cargo.toml`, `//`, `/web/`, `/index.html?x`)
   and assert 404 and that the files are matched by a fixed `match`, never by joining a path.
   Bind rule: loopback default is unchanged; the page is served on whichever address the port is bound.
7. **Where the page lives (decision 5).** `web/index.html`, `web/app.css`, `web/app.js`,
   embedded with `include_str!("../web/...")` from a new small module (suggest `src/web.rs` holding
   `pub fn page(path) -> Option<(&'static str, &'static str)>`), so `serve_remote` and the test use one table.
   Tests (`tests/remote.rs`):
   - every `/api/v1/...` path and every `src`/`href` in the embedded HTML and JS resolves to a path
     the host serves or an allowlisted API path (scan the JS for `"/api/v1/..."` literals, check each against
     the table in §4.3, so a typo is caught);
   - each static path answers 200 with its `Content-Type`, CSP, `nosniff`, `no-store`, `no-referrer`;
     `http()` at `:245` needs to return the headers; the CSP has no `unsafe-inline` and no `http(s)://` source;
   - the 429 on `PAGE_PER_MINUTE`; unpaired page load gets nothing but the page and a 401 on any API call;
   - the pairing flow end to end with the host's own client as the CLI does (build the URL, parse it,
     compare `key` with `/info`, `POST /pair` with `"session":"cookie"`, assert `Set-Cookie` attributes
     and no `token` in the body, then `GET /api/v1/me` and a read with `Cookie:` and no bearer);
   - cookie-authenticated `POST send` with a foreign `Origin` is 403; the upgrade with a foreign `Origin`
     is refused; with the right one it gets `hello`; bearer-only requests (native) are unaffected;
   - revoke closes the cookie device's socket and the next request is 401 (extend `:658`);
   - UI test in `tests/ui.rs` that the pairing window's QR text is the URL form, plus a pure test of the
     URL builder (put it in `core/remote.rs` or `ui/remote.rs` as a pure function);
   - Swift: `PairingInvite.parse` accepts the URL; new fixture only if the `/me` body is added
     (`client/Fixtures/me.json`, written with `SWITCHBOARD_WRITE_FIXTURES=1`, decoded in
     `FixturesTests.swift`).
   There is no JS test runner and the issue forbids a build step; keep `app.js` small with a pure
   `reconcile(snapshot, events)` function that can be loaded by `node --test` if node is present
   (optional, skip on CI where it is absent, like the tmux tests), else cover it by manual acceptance.
8. **The page needs, and the API already gives, these reads** (all `read` scope): `projects`,
   `projects/{id}/sessions`, `waiting` (SessionView fields in `client/Fixtures/session.json`: `name`,
   `kind`, `card`, `waiting`, `waiting_reason`, `trust_question`, `liveness`, `last_activity_ms`,
   `notes`), `sessions/{id}/last-response` (Claude Code, `text`, `truncated`), `sessions/{id}/screen?lines=n`
   (everything else), `POST sessions/{id}/send {op, text}`, `trust`. **Gap:** the issue says "the one-line
   excerpt the card shows", but `SessionView` has no excerpt field, only `notes`. Either show the first line
   of `notes` / `waiting_reason`, or add an additive `excerpt` to the view (then every fixture and the
   `revision.rs` fingerprint change, and Swift decode must tolerate it). Ask the owner; I recommend
   `waiting_reason` for waiting ones and no excerpt otherwise for the first cut.
   Events: the page subscribes to `wss://host/api/v1/events` (cookie carried), waits for `hello`, snapshots
   `projects`+`sessions`+`waiting`, then on `changed {session|sessions|waiting}` refetches the resource and
   on `changed {conversation|screen}` for the open session refetches the view (send `watch` for the open
   screen as §4.4 says, `unwatch` on leaving); on `resync_required` or a new `epoch`, reload everything;
   reconnect with back-off. A waiting session moving to the top without a reload is the `waiting`
   collection change. The `send` op is a UUID per click (`crypto.randomUUID()` needs a secure context;
   click-through certs in Chrome can be treated as insecure for some APIs, so fall back to
   `getRandomValues`), a timeout is followed by `GET /ops/{op}`, never a blind resend (§4.5).
9. **Redaction is unchanged**: every read the page uses already goes through `redact` before any cut, and
   `app.js` renders with `textContent`, never `innerHTML`, so a transcript cannot inject markup even if CSP
   were relaxed. State that rule in the spec.

## 4. Spike first (CLAUDE.md: "a new spike directory for any new risky mechanism")

`spikes/23-web-surface/` with a README of measured results, before the real code:
- Safari on iPhone and Chrome on Mac/Android: does a click-through certificate (with and without IP SAN)
  store a `__Host-` `Secure; HttpOnly; SameSite=Strict` cookie, send it on `fetch`, and on
  `new WebSocket("wss://...")`?
- Does `connect-src 'self'` allow the same-origin `wss://` in Safari?
- How many parallel connections does a page load open against the pool of 4, and does the page still
  render when a fifth is closed?
- Does the camera's QR handler open an `https://ip:port/?...` URL, and where does iOS keep it (history)?
Record the numbers and, if the cookie fails, switch to the ticket fallback in finding 3.

## 5. Files the work would touch

Rust: `src/adapters/remote.rs` (RemoteRequest: cookie, origin, host; RemoteReply: content type and
headers; `respond`; `Limits`), `src/app.rs` (`serve_remote` static arm before line 3023, `authenticate`
cookie, `remote_pair` cookie mode and Set-Cookie, new `remote_me`, Origin checks, `remote_subscribe`),
new `src/web.rs` and `web/{index.html,app.css,app.js}`, `src/lib.rs` (module), `src/ui/remote.rs:160`
(URL payload, a pure builder, the dialog sentence about the certificate warning at `:140-160` and a
line in the devices dialog), possibly `src/ports/remote.rs` (only if a device kind is added),
`src/script.rs` plus README dev-aid list only if a script line is wanted (not needed).
Swift: `Info.swift:114`, `main.swift:107`, the headset paste handler, a Swift test.
Docs: `docs/remote-client.md` (§4.1 bounds, §4.3 table + static paths + `/me`, §5 browser trust, cookie,
the "no non-loopback bind serves anything but /info, /pair, and the page" rule, §11 tests),
`docs/design.md` remote status section, `README.md` Layout rows (`web/`, `src/web.rs`, spike), the
data-directory text if it changes, `CHANGELOG.md` Unreleased line ("A web page on the remote port: pair
a phone from the QR code").
Commit shape: one commit for the spike, then host (static, cookie, origin, `/me`, tests), then the page,
then the QR/URL and the Swift parse; each with README and design status, app running at each step;
`cargo test --locked --workspace`, clippy `-D warnings`, `cargo fmt`, and the Linux dispatch clippy only
if `dispatch` crate files change (they should not). Nothing in any commit, PR or fixture names a client or
customer; use "the client project" if an example is needed. No tool attribution trailers on anything that
reaches GitHub (CLAUDE.md), whatever the harness reminder says.

## 6. Open questions for the owner

1. Is "excerpt" required in the first cut (needs an additive `SessionView` field) or is `waiting_reason`
   enough?
2. Accept the click-through as the trust step (recommended), or is the downloadable certificate wanted?
3. Add IP and `localhost` SANs to the certificate at serve time (milder warning), or leave it SAN-less?
4. Add `GET /api/v1/me` (recommended) rather than putting scope in a header?

## 7. Acceptance mapped to tests

- "scan, accept the warning, see the sessions": spike (manual) + the end-to-end pairing test.
- "a session that starts waiting moves to the top without a reload": the existing `waiting` `changed`
  event (test near `tests/remote.rs:2099`) plus page logic; manual check on a phone.
- "send a reply and watch it arrive": existing `send` tests (`:1188`) plus the cookie-auth + Origin tests.
- "revoke and the page says so on the next request": extended revoke test (401 on `/me` with the cookie)
  plus the page's `paired: true` + 401 = "revoked" branch.
