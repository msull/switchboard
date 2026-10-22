# Plan: #230, a browser control page on the remote port

Worktree: `sb-230-stage-plan-v4-haiku-b` at `7fe8949`. Every reference below was read in that tree. Facts are cited as `facts, Settled` or `facts, Unresolved N`, `facts, For the owner X`.

## Where this departs from the issue

1. **The card excerpt is not on the wire, and not on the record.** The issue says the page shows "the one-line excerpt the card shows". Spec §4.3 (`docs/remote-client.md:201,207`) says it is `last_message` on the session's metadata, and facts, Settled, the `last_message` line says it "exists on the session record". The tree says otherwise. `last_message` is a field of the Stop event only (`src/ports/events.rs:24`). The core keeps `last_stop_at` and `pending_at_stop` and drops the text (`src/core/events.rs:133-134`). The Mac's card caption is the last non-blank line of the pane log, held in UI state (`src/app.rs:888-898`, `caption_of` at `:4105`). `SessionView` (`control/src/lib.rs:637`) has no excerpt. **The plan:** no new wire field. The page fetches `sessions/{id}/last-response` for each Claude session when that session's `last_stop_at_ms` changes, and shows its first non-blank line. Other kinds show no excerpt. Spec §4.3 l.207 is amended, and facts line 13 is contradicted by this paragraph.
2. **The device's scope is not served after pairing.** `/info` is unauthenticated and carries no scope (`src/app.rs:3568-3600`). `authenticate` returns the device but nothing answers with it (`:3127-3153`). **The plan:** `GET /api/v1/device` (any paired device).
3. **"Revoked" cannot be told from "never paired" by the code.** Both refusals are `unauthorized` (`src/app.rs:3132`, `:3145`). With the token in an HttpOnly cookie, the page cannot read the token either. **The plan:** the page keeps a non-secret marker (`server_id`, key, `device_id`, name) in `localStorage`. A 401 after a marker says "This browser is no longer paired. The Mac revoked it, or forgot it." It does not claim "revoked" as certain. No host change.
4. **The page does not keep the token in script.** The issue's "keeps the token" is the one thing decision 2 rules out. A browser `/pair` reply omits `token` from the body (otherwise the page's script reads it from the JSON), and sets the cookie instead. Native clients are unchanged.
5. **The key in the URL proves less than the issue says.** §5 (`docs/remote-client.md:302-316`) says a relayed request "fails" because the host refuses a mismatched `spki_sha256`. For a page the body is whatever the page sends, so a relay that copies the key passes. The URL key does one thing: a page on a host whose `/info` claims a different key refuses to send the code. That catches a stale or reused address, not an active attacker. The browser's trust step (decision 1) is the only protection against an on-path attacker, and §5 is amended to say this.
6. **The code does not reliably stay out of history.** `replaceState` removes it from the address bar after the first request has been made. The code is in the request line before the script runs, and a browser may have recorded it (facts, Unresolved 6). The plan does not claim it is never recorded. It relies on single-use and expiry (`src/core/remote.rs:65-72`, facts, Settled) and on the host logging only the path (`refuse`, `src/app.rs:3754-3772`).
7. **Live updates are not guaranteed.** The issue assumes the `changed` socket works in Safari. Facts, Unresolved 1 says it is unknown. **The plan:** polling is the default and works without the socket. The socket is an enhancement layered on top.
8. **The certificate fails the rules the issue assumes it meets.** `make_identity` gives no subjectAltName, a CN only, and a year-4096 `not_after` (`src/adapters/remote.rs:95-127`). Facts, Settled, Apple's page requires a SAN, `serverAuth` and at most 825 days for TLS server certificates. The spec already allows reissue over the same key (`docs/remote-client.md:314`, facts, Settled, the reissue line). **The plan:** reissue over the same key with an IP SAN, `serverAuth` and a 365-day validity.
9. **An unpaired page load would hold pool slots.** A static request under the current `connection()` stays in the unauthenticated pool for up to `IDLE_TIMEOUT`, 30 s (`src/adapters/remote.rs:573`), and its reply is keep-alive. **The plan:** page replies close the connection, so the slot is freed at once.
10. **Schema counts.** Facts, Settled, the `RemoteReply` line says about 15 literal sites. The tree has 28 `RemoteReply {` occurrences in `src/app.rs`. Plan for 28.

Nothing else in the issue is contradicted. The QR-as-URL, the two-transport question, the scope-gated reply box, and the six decisions are taken below.

## The record

Two files, and one of them changes.

### Devices and the token: no change to `devices.json`

- A browser is one `PairedDevice` (`src/ports/remote.rs:96-107`) in `devices.json`. The file has no schema version and is a bare `Vec` (`src/adapters/remote.rs:80-86`). **No new field.**
- **Decision 6 (the device record):** the name is the page's own `device_name`, computed from `navigator.userAgent` as `Safari on iPhone` (browser, then device). `new_device` caps it at 64 characters (`src/core/remote.rs:166-175`). No `kind` field. The dialog's list already shows name, scope, last seen and Revoke. A line in the dialog says the browser entries are pages and that revoking one signs that browser out.
- The token is stored as today: SHA-256 in `devices.json`, never the token.

**Decision 2: where the token lives in the browser.** Settled, by the issue's own constraint and facts, Settled: a cookie with `HttpOnly` is unreadable to any script on the origin; `localStorage` is readable by any script on it. The cookie is the safer place, so the plan takes it.
- `/pair` from a browser sets `__Host-switchboard-<server_id>=<token>; Path=/; Secure; HttpOnly; SameSite=Strict; Max-Age=2592000`. `__Host-` forces `Secure`, `Path=/` and no `Domain`. The name carries `server_id` because cookies are shared across ports on one host (facts, Settled, the port line). Two instances on one Mac do not overwrite each other.
- Known limit, stated in the spec: any other HTTPS service on the same host receives this cookie. It cannot use it without the token, which it does not have, but it does hold the cookie. Facts, For the owner, the cookie-sharing question is named in follow-ups.
- The bearer header stays for native clients. **No `Sec-WebSocket-Protocol` transport is built.** If cookies fail on the owner's device (facts, Unresolved 5), the fallback is a follow-up, not a second transport now (see "Follow-ups").
- **Test first.** Facts, Unresolved 5 (`SameSite=Strict` on a same-origin WebSocket, `Secure` on an IP host) is the first thing the implementer checks, in the spike (`spikes/23-browser-page`, step 1 below), on Safari and Chrome on the Mac against the built page. The owner's iPhone run is acceptance, not a merge gate.

**The page's own state (`localStorage`, non-secret):** `{server_id, spki_sha256, device_id, name, scope}`, keyed by origin. The `key` and `code` never go into it. The token is not in it.

### The identity and the certificate: additive fields, one version bump

- `Identity` (`src/ports/remote.rs:26-42`) gains `#[serde(default)] pub cert_names: Vec<String>` (the IP address the certificate names, as text) and `#[serde(default)] pub cert_not_after: Option<SystemTime>`. Readers take an old file (defaults, so it is renewed at the next LAN bind).
- **`IDENTITY_SCHEMA_VERSION`**: the value in `src/ports/remote.rs:17` at implementation time, plus one. Today that is `1`, so `2`. The store refuses a newer one, as today.
- `SCHEMA_VERSION` (`src/core/model.rs:12`, currently `16`) and `VIEWS_SCHEMA_VERSION` (`:612`, currently `7`) do not change. No `model.rs` type changes.

**The renewal rule.** Renewal runs only in `bind_remote` (`src/app.rs:2936`), and only when `identity.listen.lan` is set. It uses the key from the secret store, which the bind already reads. It renews when any of these holds:
- `remote::local_address()` (`src/adapters/remote.rs:962-970`) is `Some(ip)` and `cert_names != [ip]`;
- `cert_not_after` is `None` or fewer than 30 days remain.

A renewal keeps the key, so `spki_sha256` is unchanged and every pin still verifies (`docs/remote-client.md:314`, facts, Settled). Renewal does not run on pairing open. The listener's TLS config is built once, at bind (`src/app.rs:2958-2960`), so a mid-session address change takes effect at the next launch. The dialog says so.

**The new certificate.** `renewed_certificate(identity, key_pem, lan_ip, now) -> Result<Option<Renewed>, String>` in `src/adapters/remote.rs`, beside `make_identity`:
- same CN (`Switchboard <server_id>`), `CertificateParams` with `SanType::IpAddress(ip)` as the only SAN;
- `extended_key_usages` = `ServerAuth`, `not_before` = now, `not_after` = now + 365 days;
- the SPKI of the loaded key must equal `identity.spki_sha256`, or the function errors and the old certificate is kept.

**The rcgen API.** `rcgen = "0.14"` (`Cargo.lock`). The field names (`subject_alt_names`, `extended_key_usages`, `not_after`, the date type) are not checked here: the registry is outside this session's allowed directories. The implementer checks them against the rcgen 0.14 source before writing, as CLAUDE.md asks.

**Decision 1: the trust step.** The browser's own certificate warning is the trust step. The owner accepts it on first visit and the browser keeps the exception.
- Why this one: it needs no install, works in every browser, and an iOS profile route is unresolved (facts, Unresolved 3 and 2 on the certificate's rules). The reissue makes the certificate meet Apple's rules (SAN, `serverAuth`, 825 days) whichever way the exception behaves (facts, Unresolved 1 and 3). The key in the QR still stops a code going to a host that claims a different key.
- The pairing window and the paired-devices dialog say it: "On a browser, the certificate warning is the trust step. Accept it only on your own network. Each new address or renewal shows the warning again."
- Offering the certificate for download stays a follow-up (see below).

## The signals

- **No new event kind.** The page reads the host's `epoch` and `revision` from every envelope and keeps one held revision per resource, as §4.2 says. It drops a reply below the held revision and resets on a new epoch.
- **Default: polling.** Every 2 s while the page is open, the page reads `GET /api/v1/projects`, each project's `sessions` (the sessions list), `GET /api/v1/device` (scope, so the reply box shows or hides on a scope change), and, for the open session, `GET sessions/{id}` plus `last-response` (Claude) or `screen?lines=40` (others). A poll that returns the same revision does not re-render. Decision: no new read model (the issue asks for one only when the page cannot do without it).
- **Enhancement: the events socket.** The page opens `wss://…/api/v1/events` after pairing. On `changed {session}` or `changed {sessions}` it re-reads the affected key at once. Polling continues at 15 s while the socket is open. Facts, Unresolved 1: if the socket fails (no `wss` after click-through, or `SameSite` cookie not sent), the page stays on polling and shows "live updates unavailable" in the header. Watch frames are not used by the default; the open session's content is polled.
- **Excerpt signal.** A Claude session's excerpt is re-read when its `last_stop_at_ms` (`SessionView`, `control/src/lib.rs:650`) moves, not on every poll.
- **Scope signal.** `GET /api/v1/device` each poll cycle. A scope change is visible at the next poll (about 2 s), not after a refused command.
- **Revocation signal.** The next request returns 401 `unauthorized` (cookie or bearer, as today). Revoke also closes the socket (`tests/remote.rs:2892`).

## Actions and effects

### Serving the page: connection thread, no frame

- **New module** `src/adapters/remote_page.rs`:
  - `PAGES: &[Page]`, three entries built with `include_str!("../../web/index.html")`, `…/app.css`, `…/app.js`. `include_str!` embeds the file's text at compile time, relative to the source file, so the binary carries the page and no build step exists.
  - `find(path) -> Option<&'static Page>`, exact matches only.
  - The exact paths, Content-Type and headers:

| Path | Content-Type |
|---|---|
| `/` (any query) | `text/html; charset=utf-8` |
| `/app.css` | `text/css; charset=utf-8` |
| `/app.js` | `text/javascript; charset=utf-8` |

  - Headers on every page reply: `Cache-Control: no-store` (no caching across versions, so an updated binary's page is never mixed with an old script), `X-Content-Type-Options: nosniff`, `Referrer-Policy: no-referrer`, `Content-Length`, `Connection: close`, and
  - `Content-Security-Policy: default-src 'none'; script-src 'self'; style-src 'self'; connect-src 'self'; img-src data:; base-uri 'none'; form-action 'none'; frame-ancestors 'none'`. No inline script or style, no external resource. The page's favicon is `<link rel="icon" href="data:,">`, so the browser does not request `/favicon.ico`.
- **Connection thread.** `connection()` (`src/adapters/remote.rs:770`) answers a `GET` for a page path itself, before the rate-limit check and before the frame, writes the page with `Connection: close`, and returns. The slot is released when the thread ends. Nothing is logged, and the query is not read.
- **Pool and rate.** Decision 4: a page load counts against the unauthenticated pool of 4 (`src/adapters/remote.rs:567`), as the issue's "four unauthenticated connections" says. It has no rate window. The pool is the bound, and the page costs no core state. The page loads three files, HTML then CSS and JS in parallel, so the default fits in four, each released on reply. Facts, Unresolved 8: whether iOS opens more connections than that is tested on the owner's device. If it does, a separate static pool is a follow-up (not built now).
- **What is not served.** Any other path outside `/api/v1/` still goes to the frame and is refused as now (`src/app.rs:3023-3025`). The three page paths are the only unauthenticated paths outside `/api/v1/`, and §5's rule is amended to say so (spec and design amendments below).
- **Decision 4, the empty state:** with no code and no marker, the page shows "Open Remote devices on the Mac and scan its code." It has no code-entry form. A typed code needs the key shown for the owner to compare (the native flow), so that is a follow-up.

### Authenticating a browser

- `RemoteRequest` (`src/adapters/remote.rs:360-372`) gains `host: Option<String>`, `origin: Option<String>` and `cookies: Vec<(String, String)>`, read in `read_request` (`:444-513`). The other headers stay dropped. `RemoteRequest` is built only in the adapter (`:506`, no literal in `tests/`), so only the adapter changes.
- `authenticate` (`src/app.rs:3127-3153`) takes the token from `request.bearer`, else from the cookie named `__Host-switchboard-<server_id>`.
- **Origin rule (the plan's default, not an owner question).** A cookie-authenticated request whose `Origin` header is present must equal `https://` + the `Host` header. Otherwise it is refused with 403 `bad_origin`, logged by method and path as the other refusals are. Browsers send `Origin` on every POST and WebSocket upgrade (facts, Settled, the RFC 6455 line). A native client sends no cookie, so the rule does not touch it. `SameSite=Strict` already keeps cross-site cookies off, so this rule is the second layer, and it does not depend on which name the browser used (IP, `.local`, loopback).
- A bearer request is not checked for `Origin`, as now.

### Pairing by URL

- **`invite_url`** in `src/core/remote.rs`, a pure function (no I/O): `invite_url(address: Option<IpAddr>, port, server_id, spki_sha256, code) -> Option<String>` gives `https://<host>:<port>/?code=<code>&key=<spki>&id=<server_id>`. An IPv6 host is bracketed. `None` when there is no address.
- **The QR and "Copy invite"** (`src/ui/remote.rs:160-166`, `:201-206`) use the URL. The JSON object goes. The `addresses` list is one `host:port`, now inside the URL.
- **`/pair`** (`src/app.rs:3606-3674`) takes an optional `transport` in the body. `"cookie"` means a browser: the reply omits `token` and carries the `Set-Cookie`. Otherwise the reply is as today. A browser declaring `cookie` gains nothing a native client could not already do, so the field is not a security boundary.
- **`RemoteReply` gains `headers: Vec<(String, String)>`.** The 28 literal sites in `src/app.rs` take `headers: Vec::new()`, or a constructor if the implementer prefers one. `respond` (`src/adapters/remote.rs:530-552`) writes the headers after `Content-Type`. The envelope path is unchanged.
- **Refused pairings are not counted twice.** The page checks the URL key against `/info` before posting. A mismatch stops the page and sends nothing. So the host's failure counter (`src/core/remote.rs:56-64`) is not touched by a page that refuses.
- **The page's pairing flow** (UI section) posts `/api/v1/pair` only when the URL key equals the `/info` key and the URL id equals the `/info` `server_id`.

### The device endpoint

- `GET /api/v1/device` in `serve_remote` (`src/app.rs:3017`), after `authenticate`, next to the `events` arm. Answer: `{device_id, name, scope}` (scope in the serde form `/pair` uses). Built like `remote_info`, with `envelope`. Not a wire view, so no fixture. Tested in `tests/remote.rs`.

### Startup and bind

- `bind_remote` (`src/app.rs:2936`): before `remote::tls_config` (`:2957`), when `lan` is set and the renewal rule holds, dispatch `AppAction::CertificateRenewed` and re-read the identity from the core.

### Core

- `AppAction::CertificateRenewed { certificate, cert_names, cert_not_after }` (new variant). The core arm calls `update_identity` (`src/core/remote.rs:105-115`), which emits the existing `Effect::SaveIdentity`. The `spki_sha256` field is not in the action, so the core cannot change the pin. No new `Effect`, no port method, no fake.

## The UI

- **Pairing window** (`src/ui/remote.rs:141-206`). The QR encodes the URL. The text beside it shows the key's short form, the code, the address, and the URL itself so it can be typed. "Copy invite" copies the URL. Note in place: "Scan with the phone's camera, then accept the certificate warning. Five minutes or three wrong codes." With no address (`local_address()` is `None`), no QR is drawn and the window says "No network address. Connect the Mac to a network to pair." The "Key" and "Address" lines do not change.
- **Paired-devices dialog** (`src/ui/remote.rs`, the devices section). The browser's line is labelled as a browser device, and the note about the certificate warning and about revoking goes in.
- **The page** (`web/index.html`, `web/app.css`, `web/app.js`). Plain DOM, no framework, no build, no package manager. The code is one file of about 400 lines. The script's parts:
  1. **Start.** Read `location.search` into local variables, then `history.replaceState(null, '', '/')`. This is the first statement that touches the URL.
  2. **Identify.** `GET /api/v1/info`. The page shows the host name from `server_name`, and the header.
  3. **Pair** (URL with `code`). If `info.spki_sha256 !== key` or `info.server_id !== id`, stop with "This is not the Mac whose code you scanned. Nothing was sent." Otherwise `POST /api/v1/pair` with `{code, spki_sha256: key, device_name, transport: "cookie"}`. On 200, write the marker and show the sessions. On 403 `pairing_closed`, "Pairing is closed on the Mac. Open the window and scan again." On 403 `wrong_code`, "That code is wrong or expired." The token is never in the reply body, so the script cannot see it.
  4. **Resume** (no `code`, marker present). If `info.spki_sha256` differs from the marker, stop with "Identity changed. Pair again." and make no authenticated request. Otherwise `GET /api/v1/device`. A 401 gives the "no longer paired" text.
  5. **Neither** (no code, no marker). "Open Remote devices on the Mac and scan its code."
  6. **Sessions.** Group by `project`, waiting first (`SessionView.waiting`), then by name. Each card shows kind, state (`card`, `waiting_reason`), and the excerpt (Claude sessions only). The header shows the host name and the scope from `device`.
  7. **Session view.** `sessions/{id}` plus `last-response` (Claude) or `screen?lines=40`, refreshed on the poll.
  8. **Reply box.** Visible only when `scope === 'control'`. Posts `POST sessions/{id}/send` with `{op, text}` (§4.5). The `op` is `crypto.randomUUID()` when `window.isSecureContext`, else a `crypto.getRandomValues` hex id. A timeout is followed by `GET /api/v1/ops/{op}` and the §4.5 rule (`done`, `interrupted`, `unknown`), never a blind resend.
  9. **States.** Any 401 after the marker: "This browser is no longer paired. The Mac revoked it, or forgot it." A failed `/info`: "The Mac did not answer. It may be asleep or on another network." A socket failure: "Live updates unavailable." Stated, not hidden.
- **Decision 5, testing the page.** The page's logic (the URL parse, the replace, the revision reconcile, the marker) is not tested by CI. No JS test runner is added (a dependency, which the issue rules out). The logic is checked by hand: the checklist in `spikes/23-browser-page/README.md` is run on Safari and Chrome on the Mac, and the owner runs it on a phone. The tests below check everything the host can check.
- **Swift.** `PairingInvite.parse` (`client/Sources/SwitchboardProtocol/Info.swift:114-116`) tries the URL first (`URLComponents`, scheme `https`, `code`/`key`/`id` query items, host and port into `addresses` as one `host:port`, IPv6 bracketed as `Address` expects), then JSON. `PairingInvite`'s fields do not change. `switchboard-remote pair '<url>'` (`client/Sources/switchboard-remote/main.swift:107-125`) and `BrowserView`'s paste (`client/Sources/SwitchboardUI/BrowserView.swift:106,883`) get the URL without a call-site change. Decision 3: the JSON parse stays, so text already copied still pastes. No host produces JSON after this change. Nothing in the tree needs the JSON form after it: the owner's question about the paste field (facts, For the owner, decision 3) is answered by the tree, since `BrowserView` calls `parse`.

## The tests

**Rust, `tests/remote.rs`** (the helpers `http` at `:245` and `call`/`get` at `:299`, `:327`, `pair` at `:406`). `http` discards headers today. A sibling helper returns status, headers and body; the existing helpers are unchanged.

1. `the_page_is_served_unpaired_with_its_headers`: `GET /`, `/app.css`, `/app.js` answer 200, the Content-Type from the table, the CSP, `no-store`, `nosniff`, `no-referrer`, `Connection: close`, and no `Set-Cookie`. `GET /?code=…` answers the same bytes, and the code appears nowhere in the reply.
2. `the_page_references_only_served_paths`: every `href`, `src` and `/api/v1/…` literal in `web/index.html` and `web/app.js` is in a fixed list in the test. Each listed endpoint answers something other than the prefix `not_found` with a paired token. The test fails if the page names an endpoint the list lacks.
3. `the_pairing_url_pairs_through_the_hosts_own_client`: `invite_url` built from the open window, parsed by splitting the query, then `GET /api/v1/info`, `POST /api/v1/pair` with `transport: "cookie"` and `device_name: "Safari on iPhone"`. Assert: 200, no `token` in the body, `Set-Cookie` with the name, `HttpOnly`, `Secure`, `SameSite=Strict`, `Path=/`, `Max-Age`. Then `GET /api/v1/projects` with the cookie only → 200. `GET /api/v1/device` → name and scope. A second `/pair` with the same code → 403 `pairing_closed`.
4. `the_native_pair_is_unchanged`: `/pair` without `transport` returns the token in the body and sets no cookie.
5. `a_cookie_from_a_foreign_origin_is_refused`: `POST sessions/{id}/send` with the cookie and `Origin: https://evil.example` → 403 `bad_origin`, nothing sent. With `Origin` equal to `https://` + `Host` → past the check. A bearer with a foreign `Origin` → past the check (unchanged).
6. `a_revoked_cookie_is_refused_and_is_not_a_read_anywhere`: revoke (the dialog's action), then the same cookie → 401 `unauthorized`, and the socket closes (`revoke_closes_the_devices_socket` pattern, `:2892`).
7. `page_connections_release_their_slot`: four page loads in a row, each closed by the server, then `GET /api/v1/info` still answers (the pool is not left full). Extends `stalled_strangers_fill_their_own_pool_and_never_a_paired_clients` (`:3226`).
8. `the_allowlist_refuses_everything_else_with_no_side_effect` (`:682`) gains `/index.html`, `/app.js.map`, `POST /` → 404 `not_found`, and no log line carries a query.
9. `the_client_fixtures_decode_on_both_sides` (`:1829`): a new `client/Fixtures/invite.txt` written with `SWITCHBOARD_WRITE_FIXTURES=1` holds the URL `invite_url` builds for a fixed input. Both sides read it. The Swift test in `FixturesTests` (`:157`) parses it.

**Rust, unit.**
- `src/core/remote.rs` (`invite_url`): IPv4, IPv6 bracketed, `None` with no address.
- `src/adapters/remote.rs` (`renewed_certificate`): the SPKI is unchanged, the IP SAN is present, EKU is `ServerAuth`, validity ≤ 825 days, `None` when the name and expiry are current, an error when the key does not match the identity.
- `src/adapters/remote_page.rs`: `find` is exact (`/app.js/` and `/%2e` are not pages).

**Rust, core, `src/core/tests.rs`.** `CertificateRenewed` updates the certificate fields and keeps `spki_sha256`, and emits `SaveIdentity`. A read-only core emits nothing.

**Rust, UI, `tests/ui.rs`.** The existing `remote_devices_dialog_opens_pairing_and_revokes_a_device` (`:7424`, "Copy invite" at `:7474`) asserts that the copied text starts with `https://` and contains `code=`, `key=` and `id=`.

**Swift, `client/Tests/SwitchboardProtocolTests/FixturesTests.swift`.** The URL form parses to the same `PairingInvite` as the JSON form. A URL with an IPv6 host, a URL missing `key`, and a non-`https` URL each fail. The JSON form still parses (`:157`).

**Spike, `spikes/23-browser-page/README.md`.** Recorded with the commands used and the numbers seen:
- Safari and Chrome on the Mac against the built page on a LAN address: the warning, the click-through, the `Secure; HttpOnly; SameSite=Strict` cookie stored on an IP host, sent on `fetch`, and sent (or not) on `new WebSocket`. This answers facts, Unresolved 5 for the Mac engines.
- The checklist for the page's logic (step 3 above), run by hand.
- The owner's iPhone run (facts, For the owner A and D) is recorded here when it happens. It is acceptance. It does not gate the merge.

**Gate.** `cargo test --locked --workspace`, `cargo clippy --locked --workspace --all-targets -- -D warnings`, `cargo fmt --all`. `swift test` in `client/` for the parser. The dispatch crate is untouched, so the Linux cross-check is not needed.

## Spec and design amendments

- **`docs/remote-client.md`**
  - §4.1 bounds: the page paths (connection thread, `Connection: close`, no rate window, the unauthenticated pool) and the device endpoint.
  - §4.3 table: `GET /`, `GET /app.js`, `GET /app.css` (unauthenticated) and `GET /api/v1/device` (read). The card excerpt sentence (l.207) is corrected: the excerpt is the last non-blank line of the `last-response` of a Claude session, read when `last_stop_at_ms` moves. Facts, Settled, the `last_message` line is contradicted here.
  - §5: the QR is the URL. The "sent only as a bearer header … never in a URL" sentence gains the browser's cookie and its attributes. The sentence "a relayed request … fails" is amended for browsers (departure 5). The Origin rule is added. The "Require pairing before anything but `/info` and `/pair`" bullet names the three page paths. The reissue rule and the bind-time renewal are added.
- **`docs/design.md`.** The "Remote client (2026-10-12)" section gains a "Browser page" status bullet under its built list (around `:3112-3115`), with what is built, what is not (dispatch, conversation, drafts, the JS test harness, the typed-code form), and the spike's result. "Trust boundary" (`:219`) is not changed: the page is not a record and nothing in a project directory runs.
- **`README.md`.** The Layout table gains rows for `web/` and `src/adapters/remote_page.rs` (near `:383`). The dev-aid text (`:265-279`): "Copy invite" now copies the URL, `switchboard-remote pair "$(pbpaste)"` takes it, and the page is at `https://<address>:<port>/`. The data-directory list (`:78-83`) says `identity.json` holds the certificate and the names it covers.
- **`CHANGELOG.md`.** One line under Unreleased: the Mac serves a browser page on the remote port, paired from the Remote devices window's URL.

## Order of work

One commit. The app runs at each step, and each step keeps the existing native pairing working.

1. **Spike** (`spikes/23-browser-page`, Mac Safari and Chrome, the cookie and the click-through). The result is written up before any cookie code. If the cookie is not stored or not sent on the Mac engines, stop and report; the transport question is then the owner's (follow-up).
2. **Core.** `invite_url`, `AppAction::CertificateRenewed`, and the core tests.
3. **Adapter.** `RemoteRequest` and `RemoteReply` fields, the header reads, `respond` with headers, `remote_page.rs` and the connection-thread arm, `renewed_certificate` with its unit tests, `IDENTITY_SCHEMA_VERSION`.
4. **App.** `authenticate` (cookie, origin), the device endpoint, `/pair`'s transport, `bind_remote`'s renewal, and the 28 `RemoteReply` literals.
5. **UI.** The pairing window's URL and the devices dialog's browser line.
6. **Swift.** `PairingInvite.parse` URL, the tests, `client/Fixtures/invite.txt`.
7. **The page.** `web/`.
8. **Tests** (`tests/remote.rs`, `tests/ui.rs`).
9. **Docs** (the spec, `design.md`, `README.md`, `CHANGELOG.md`).
10. **Gate** (the four commands, `swift test`).

**Acceptance (the issue's steps, by whom).**
- Open the pairing window, scan the QR with a phone, accept the warning, see the sessions: the owner, on the phone, recorded in the spike README. The tests above cover the host's half.
- A session that starts waiting moves to the top without a reload: the owner, by polling (the default). Watching the socket path too, if it connects.
- Send a reply from the phone and watch it arrive: the owner. Test 3's client path covers the host side.
- Revoke the phone in the dialog and the page says so on its next request: the owner. Test 6 covers the host side.

## Follow-ups (for the owner)

Each is a decision the plan does not make, or a fallback it does not build:

- **Cookie fails on the owner's device** (facts, Unresolved 5, For the owner D). A second transport (the `Sec-WebSocket-Protocol` token, or a header-less URL token) is a follow-up. It is not built now, per the rule that the fallback waits for the safer one to fail its test.
- **A profile or certificate download** as the trust step, instead of the warning (decision 1's other branch; facts, Unresolved 3 on what iOS requires of a manually trusted certificate). Not built.
- **The `.local` DNS name** in the certificate and the URL (facts, For the owner B). If the owner answers "resolves", the renewal adds the DNS SAN and the URL host changes. The IP form works either way.
- **Connections beyond four on iOS** (facts, Unresolved 8). A separate static pool, if the device run needs one.
- **The cookie shared with other HTTPS services on the same host** (facts, For the owner, the cookie-sharing question). Mitigation (a signed per-device cookie, or the server-id-bound token) is not built.
- **The typed-code form** on the page, which needs the key shown for the owner to compare.
- **A JS test runner** for the page's logic (out of scope: a dependency).
- **Dispatch tickets and decisions, the conversation, drafts, dictation, runs, pinned files** on the page: the API serves them, the page does not yet.
- **Watch frames on the page** for the open session's content, if the socket works on the device.

Each is named, not built, and none changes a decision the spec or design already made. The spec's §5 pairing, token and bearer rules are amended only where the issue asks for the browser path.

## Contract additions

For the implementer to copy into the commit message body:

- `src/ports/remote.rs` `Identity`: `cert_names: Vec<String>` and `cert_not_after: Option<SystemTime>`, both `#[serde(default)]`. `IDENTITY_SCHEMA_VERSION` bumped to the value at implementation time plus one (2 today).
- `src/core/action.rs` `AppAction`: `CertificateRenewed { certificate, cert_names, cert_not_after }`.
- `src/adapters/remote.rs` `RemoteRequest`: `host`, `origin`, `cookies`. `RemoteReply`: `headers`. (Adapter types, not the shared contract, but listed.)
- `src/core/remote.rs`: `invite_url` (a function, not a type).
- `src/adapters/remote_page.rs`: new module, `PAGES` and `find`.
- Wire: `GET /`, `GET /app.js`, `GET /app.css` (unauthenticated, not under `/api/v1/`); `GET /api/v1/device` (read) with body `{device_id, name, scope}`; `POST /api/v1/pair` takes an optional `transport: "cookie"` and answers a `Set-Cookie` in that case; the cookie `__Host-switchboard-<server_id>`.
- Swift: `PairingInvite.parse` accepts the URL form. The struct does not change.
- No `SessionView` field. No `Effect`. No port trait or fake change. `SCHEMA_VERSION` and `VIEWS_SCHEMA_VERSION` do not change.

## Commit message

`Serve a browser control page on the remote port, paired from a URL`
