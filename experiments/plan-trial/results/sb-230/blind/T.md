# Issue #230 plan: a web control surface on the remote port

Worktree `/Users/sbgragent/.dispatch/worktrees/340ccb28`, base `7fe8949`. Every file and line below was read in this worktree. "Facts" means the facts file; its Settled bullets are cited by their opening words.

## Where this departs from the issue

- **"The QR carries `{server_id, spki_sha256, code, addresses}`; the text path takes the URL too, or the JSON as before."** The JSON is parsed in three Swift places (`client/Sources/SwitchboardProtocol/Info.swift:114-116`, `client/Sources/switchboard-remote/main.swift:107-111`, the headset's paste in `client/Sources/SwitchboardUI/BrowserView.swift`) and built inline in `src/ui/remote.rs:160-166`, with no test. Plan: the QR and "Copy invite" carry the URL only; the Swift parser accepts both forms; the builder moves to core as a pure function (`core::remote::pairing_link`). `addresses` leaves the QR, because a URL carries one host.
- **"The one-line excerpt the card shows."** There is no such field. `SessionView.card` is a label word (`control/src/lib.rs:645-646`), and `last_message` exists only on hook events (facts, Settled: "`last_message` exists only on the hook event"). Plan: the session list shows the state word and `waiting_reason`. The excerpt appears only in the session view, where the last response or the screen is already on screen.
- **"The page's script never sees the token."** True only if the browser's `/pair` reply leaves the token out. Today the reply body carries it (`src/app.rs:3661`, `"token": token`). Plan: a browser pairs with `carrier: "cookie"`, and the reply sets the token as an `HttpOnly` cookie with no token in the body. The subprotocol carrier is ruled out (facts, Settled: a `Sec-WebSocket-Protocol` value is readable by script).
- **"The code never stays in the address bar or the history."** `history.replaceState` replaces the current entry (facts, Settled), but the request that carried `?code=` already reached the host, and any subresource `Referer` carries the query unless the page sends `Referrer-Policy: no-referrer` (facts, Settled). Plan: the static responses send `no-referrer`, and the claim is narrowed to the address bar and the current history entry. Whether Safari's own history or a share sheet keeps the scanned URL is unresolved (facts, Unresolved) and is not claimed.
- **"The key in the URL lets a genuine page refuse to send the code to a host whose `/info` key differs."** This holds only as an address check. `/info` is answered before authentication (`src/app.rs:3028`), and the browser has no pin (facts, Settled: "The `/info` answer is not an authentication of the server"). An intermediary on the LAN that copies the key into its own `/info` passes the check. Plan: the dialog says that the click-through is the only trust step, and that a code accepted on a warning goes to whoever is behind it. This weakens the threat model the spec states: `docs/remote-client.md:309` promises a pinned key before the code is sent, and `docs/design.md:3111-3112` says the client pins. The trade is owner call 6, and both lines are amended under Docs and status.
- **Pool.** The unauthenticated pool is 4 (`src/adapters/remote.rs:567`), and a fifth connection is dropped at accept (`:704`). Static paths have no rate window: `Limits::limited` answers "not limited" for every path but `/pair` and `/info` (`:631-640`). Plan: static paths count against the pool only, as the issue asks in item 4.

## The record

- **`devices.json`, `PairedDevice`** (`src/ports/remote.rs:96-107`): one field added, `pub revoked: Option<SystemTime>` with `#[serde(default)]`, the same way `last_seen` is added. Revoke sets it and keeps the row. The token's SHA-256 stays on the row, so a revoked token can still be recognised.
- **Schema versions, as they are today:** `SCHEMA_VERSION` = 16 (`src/core/model.rs:12`), `VIEWS_SCHEMA_VERSION` = 7 (`src/core/model.rs:612`), `IDENTITY_SCHEMA_VERSION` = 1 (`src/ports/remote.rs:17`). **No bump.** `PairedDevice` is not in `model.rs`, and `devices.json` is a bare array with no version (`src/adapters/remote.rs:80-85`; facts, Settled). The new field is read as `None` when absent.
- **Downgrade hazard.** A build older than this one ignores `revoked`, so downgrading after a revoke re-admits the token. Nothing in the tree guards this today, since `devices.json` has no refuse-to-write-newer check. The spec §11 row says so.
- **Device name:** the page sends `device_name` from its user agent (`Safari on iPhone`). `new_device` already caps it at 64 characters (`src/core/remote.rs:169`). No `kind` field is added; the name is the label.
- **Rotation** still deletes every row, tombstones included.

**For the owner: tombstone or not (issue acceptance; facts For the owner 6).** Plan: tombstone, because the acceptance names "revoked". Owner call 4 has the fallback.

## The actions and effects

**Core (`src/core/remote.rs`, `src/core/action.rs`)**
- `AppAction::RevokeDevice(id)` keeps its variant. Its effect changes from removing the row to setting `revoked`, and it saves `devices.json` as today. A second revoke is a no-op. The variant still closes the device's socket (`src/app.rs:486-494`), so the close is unchanged.
- `DeviceSeen` and `SetDeviceScope` ignore a revoked row.
- `device_by_token` (`:145`) returns the row whether or not it is revoked. `authenticate` reads the flag (see App).
- New pure function `pairing_link(host: &str, port: u16, server_id: Uuid, spki_sha256: &str, code: &str) -> String`, returning `https://{host}:{port}/?code={code}&key={spki}&id={server_id}`. `host` is an IPv4 or IPv6 literal, from `remote::local_address()` (`src/adapters/remote.rs:962`). The function adds the brackets for an IPv6 host. All four strings are from charsets that need no escaping (hex, a UUID, the Crockford alphabet); the test pins this.
- No new effect and no new port.

**Adapter (`src/adapters/remote.rs`)**
- `RemoteRequest` (`:360`) and `read_request` (`:444`) gain the request fields listed under Contract additions. The cookie value is taken out of the header, as the bearer is, so it is never logged with the headers.
- `RemoteReply` (`:389`) gains the headers and a constructor (Contract additions). `respond` (`:530`) writes them, with JSON as the default. `respond_upgrade` (`:519`) is unchanged.

**App (`src/app.rs`)**
- `serve_remote` (`:3017`): an arm for `GET /` and `GET /app.js`, checked before the `/api/v1/` strip at `:3023` (the strip refuses every other path). It sits beside `/info` and is answered before `authenticate`. Decision 4 has the detail.
- `serve_remote`: `("GET", ["device"])` after `authenticate`, answering `{device_id, name, scope}`.
- `authenticate` (`:3127`): the bearer is checked first, as today. With no bearer, the `switchboard_token` cookie is checked. A row found by cookie is subject to the Origin rule. A row found with `revoked` set answers `401 revoked` with "this device was revoked; pair it again", before `DeviceSeen` runs.
- `remote_pair` (`:3606`): when the body's `carrier` is `"cookie"`, the reply is `{scope, server_id, device_id}` with the header `Set-Cookie: switchboard_token=<token>; HttpOnly; Secure; SameSite=Lax; Path=/; Max-Age=2592000`. Without `carrier` the reply is as now, with the token in the body, so the native clients do not change.

**Decision 1 (certificate trust). Click-through is the trust step.** The host's certificate has no SAN and a 4096 `notAfter` (facts, Settled). Whether Safari offers an override for that certificate is unresolved (facts, Unresolved), and it needs a phone. Plan: the warning is accepted on the device, and the dialog says so. There is no certificate download, since that adds a step the owner must take on each device.
- *For the owner: the phone check gates the merge.* If step 0 shows no override, re-issue is new work with its own renewal design (owner call 1). The current code generates a new key (`src/adapters/remote.rs:101`), so a re-issue would change `spki_sha256` (`:112`) and break every pin; the re-issue must reuse the Keychain key `remote-key:<server_id>`.

**Decision 2 (token carrier). `HttpOnly; Secure; SameSite=Lax` cookie, `Path=/`, `Max-Age` 30 days.**
- The subprotocol carrier is out (facts, Settled): script can read a `Sec-WebSocket-Protocol` value. The cookie is then the only carrier script cannot read.
- Whether the cookie rides the WebSocket handshake is unresolved in the facts (Unresolved). The WHATWG WebSockets spec, §2.2 step 2, sets the handshake's credentials mode to `include` (checked for this revision), so the cookie is eligible on the handshake, subject to SameSite. Step 0 settles it in practice.
- **Gate, in step 0** (owner call 2). If the cookie is not stored after a click-through, the carrier is `localStorage`, which script can read, and "script never sees the token" is withdrawn. If step 0 refuses the handshake, poll the session view.
- **Why `Lax`, not `Strict`.** `Strict` would also drop the cookie on a top-level navigation from a link in another app. `Lax` keeps that navigation and still withholds the cookie from cross-site POSTs and subresources. The Origin rule below does not depend on the site definition.
- **The Origin rule** (cookie-authenticated requests only; bearer requests skip it). If an `Origin` header is present, it must equal `https://` followed by the `Host` header; otherwise `403 origin`. A non-GET cookie request with no `Origin` is also `403 origin`. This is the defence against a cross-site POST, since a cookie makes every `control` POST ambient (`send`, `resume`, `trust`, `draft`, dictation, ticket decide, close, new session).
- **Rebinding is not stopped by the Origin rule.** A page on a rebound name sends a matching `Origin` and `Host`. What stops it is cookie scope: the cookie is set for the IP address the URL carried, and a request to any other host name carries no cookie, whatever SameSite does. The rebound page gets `401` and no session. This argument does not depend on the SameSite answer in the facts.
- **GETs under the cookie.** `Lax` still sends the cookie on a cross-site top-level GET (RFC 6265bis §5.6.7.1, checked for this revision). Such a GET runs with the device's authority, but the attacker's page cannot read the response, so the exposure is side effects only. Two GETs have one: `authenticate` dispatches `DeviceSeen` (`src/app.rs:3150`, `last_seen` only), and the draft read calls `ensure_editor` (`src/app.rs:3361`), which changes UI state only. Both are accepted. A GET added later is checked against this list.
- **Bearer:** unchanged, and never in a URL.

**Decision 3 (one QR). The URL only.** The QR and "Copy invite" carry `pairing_link`. The Swift `PairingInvite.parse` (`Info.swift:114-116`) accepts the URL and, unchanged, the JSON, so pasted text and the dev aids keep working. Nothing generates the JSON any more. The CLI's three-argument form (`main.swift:112-124`) is unchanged.

**Decision 4 (static paths). `GET /` and `GET /app.js`, answered by the frame.**
- An arm of `serve_remote`, checked before the `/api/v1/` strip and before `authenticate`, like `/info`. It takes a frame pass (the frame drains eight requests per pass) and counts only against the unauthenticated pool. It has no rate window. The bytes are `include_str!` constants.
- Headers on both: `Content-Type` (`text/html; charset=utf-8` and `text/javascript; charset=utf-8`), `Cache-Control: no-store`, `X-Content-Type-Options: nosniff`, `Referrer-Policy: no-referrer`, and
  `Content-Security-Policy: default-src 'none'; script-src 'self'; style-src 'unsafe-inline'; connect-src 'self' wss:; img-src data:; base-uri 'none'; form-action 'none'; frame-ancestors 'none'`.
  The CSS is inline in `index.html`, so no stylesheet request is made. `connect-src` names both `'self'` (for `fetch`) and `wss:` (for the events socket): whether `'self'` alone covers `wss:` in Safari is unresolved (facts, Unresolved), so `wss:` is named. The scheme source is wider than one host, but `script-src 'self'` allows only the page's own script, so nothing an attacker injects can run.
- The page's icon is `<link rel="icon" href="data:,">`, which stops the browser fetching `/favicon.ico`.
- **Connection count for a cold load:** `/`, `/app.js` and `/api/v1/info`, three connections if the browser opens them in parallel. That fits in 4. Step 0 measures it.

**Decision 5 (the pairing link). `?code=` stays, as the issue writes it.** The request line carries the code, but the host never logs a query (`refuse` logs `request.path` only, `src/app.rs:3754`), and the static answer does not log either. `no-referrer` closes the `Referer` path.

**Decision 6 (scope and name).** The page reads scope from `GET /api/v1/device` and hides the reply box unless it is `control`. The name is derived in JavaScript from `navigator.userAgent`: browser (Edge, Chrome, Firefox, Safari, in that order of checks) and device (iPhone, iPad, Android, Mac). Fallback: an iPad in desktop-site mode is labelled Mac. The name is only the label on the device's row in the page header and the dialog; nothing reads it for a decision, so nothing else depends on it, and no detection is built.

**The page's pairing sequence, in order.** It reads `location.search` into memory, calls `history.replaceState(null, '', '/')` before anything else, calls `GET /api/v1/info`, and compares `spki_sha256` and `server_id` with the URL's `key` and `id`. A mismatch stops with "This address answers with a different key. The code was not sent." A match sends `POST /api/v1/pair` with `{code, spki_sha256, device_name, carrier: "cookie"}`. On success the page drops the code from memory and loads the sessions. `403 wrong_code` and `403 pairing_closed` each show their own line.

**Decision 7 (the page's stored state).** Nothing in `localStorage`. The page keeps the cookie (set by the host) and, per session, the send's op id in `sessionStorage`, so a reload mid-send resends with the same op and the host answers from its log (`remote_command`, `src/app.rs:3159`). Every storage access is wrapped in try/catch, and the page renders without storage.

## The UI

- **Pairing window (`src/ui/remote.rs:141-206`).** The QR encodes `core::remote::pairing_link(&ip.to_string(), port, identity.server_id, &identity.spki_sha256, &window.code)`, with `ip` from `remote::local_address()`. The text beside it and "Copy invite" carry the same URL. The JSON builder at `:160-166` goes. When `local_address()` returns `None` (no LAN route, `src/adapters/remote.rs:962`), there is no host to put in a URL: the window shows no QR and no copy button, and says "This Mac has no network address. Connect it to a network, then open pairing again." The code is still shown, so the owner can type it on the device. Two lines are added:
  - When `identity.listen.lan` is off, a warning: "Only this Mac can reach the host while devices on the local network are off. Turn them on to pair a phone." The window still shows the URL, since `local_address()` returns the LAN address whether or not `lan` is on (`:962`, facts notes F10). Warn, not refuse, because the CLI on the same Mac works on loopback.
  - "A browser shows a certificate warning for this Mac. Accept it on the device; pairing then continues in the page."
- **Paired devices dialog (`src/ui/remote.rs:229-275`).** It lists `devices()` with `revoked` rows filtered out. Revoke is unchanged. The dialog's trust sentence sits beside the redaction text at `:71-72`: "A browser trusts this Mac's certificate when its owner accepts the warning on that device. Revoking ends its access on its next request."
- **The page (`web/index.html`, `web/app.js`).** Plain HTML, CSS and JS, no build, no dependency, phone width with a 16 px gutter and no horizontal scroll.
  - Header: device name and scope from `/api/v1/device`, host name from `/info`.
  - Sessions, grouped by project, waiting ones first, each row with the name, kind, state word (`card`) and a "Waiting on you" mark from `waiting_reason`.
  - Session view: the last response (`GET /api/v1/sessions/{id}/last-response`) for a `Claude` session, and the screen (`GET /api/v1/sessions/{id}/screen?lines=40`) for `Codex`, `Shell`, `Command` and `Service`, as the issue says. The page watches the open session's key (the `watch` frame of §4.4) and refetches on `changed`.
  - Reply box, only when the scope is `control`: `POST /api/v1/sessions/{id}/send` with `{op, text}` (`src/app.rs:3179`, `3192`). The op is a random id kept in `sessionStorage` until the send succeeds.
  - States: "Not paired, or this Mac's key changed: pair it again" for `401 unauthorized`; "This device was revoked on the Mac. Pair it again." for `401 revoked`; "The Mac is busy; try again" for `429` and `503`; the pairing-link messages above.
  - Pairing states from `/info`'s `pairing_open`: "Scan the code on the Mac's screen" when open, and "Open Remote devices on this Mac and allow pairing" when closed.

## The tests

- **Core (`src/core/tests.rs`)**: `RevokeDevice` keeps the row with `revoked` set and `devices()` still lists it; `device_by_token` returns the revoked row with `revoked` set; `DeviceSeen` and `SetDeviceScope` leave it alone; a re-pair makes a new row. `pairing_link` with a fixed host, port, UUID, key and code gives the exact URL, and an IPv6 host is bracketed.
- **Remote port (`tests/remote.rs`)**, with a new raw helper that returns status, headers and body (the `http` helper at `:245` reads neither headers nor non-JSON bodies, so `call` at `:299` cannot be used for the page):
  - `GET /` is `200` with the Content-Type, the exact CSP, `no-store` and `no-referrer`, with no token. `GET /app.js` likewise. Twenty requests in a row are all `200` (no rate window).
  - `GET /index.html`, `GET /favicon.ico` and `POST /` are `404 not_found`, and `info_is_served_to_anyone_and_every_other_read_is_refused_unpaired` (`:426-475`) still passes.
  - **The page references only served paths.** The test reads `web/app.js` and collects each `/api/v1/` literal; each must answer something other than `404`, unauthenticated.
  - **Pairing end to end, with the host's Rust client as the CLI does:** `GET /info`, compare the key, `POST /pair` with `carrier: "cookie"` returns `200`, a `Set-Cookie` with `HttpOnly; Secure; SameSite=Lax`, and a body with no `token`. Then `GET /api/v1/device` with the cookie is `200` and reports the window's scope. A `GET /api/v1/events` upgrade with the cookie is `101`. A `POST` with the cookie and `Origin: https://evil` is `403 origin`. The same `POST` with the matching Origin reaches the scope check (a read device gets `403 scope`). A `POST` with a cookie and no Origin is `403 origin`.
  - **Revocation:** after `RevokeDevice`, a request with the cookie and one with the bearer both answer `401 revoked`. `revoke_and_a_scope_change_take_effect_on_the_next_request` (`tests/remote.rs:657`) changes two assertions. Line 674, `devices.is_empty()`, becomes a check that the row stays with `revoked` set. Line 676, `(401, "unauthorized")`, becomes `(401, "revoked")`. `revoke_closes_the_devices_socket` (`:2892`) is unchanged, since the close comes from the action.
- **Swift (`client/Tests/SwitchboardProtocolTests`)**: `PairingInvite.parse` of a URL gives the server id, the key, the code, and `addresses = ["host:port"]`, with IPv6 hosts unbracketed. The JSON still parses.
- **UI (`tests/ui.rs`, the dialog test at `:7424`)**: three assertions change. Lines 7487-7492 parse "Copy invite" as JSON and check `code`, `spki_sha256` and `addresses`; they become a parse of the URL, checking the `code`, `key` and `id` query values and the host. Line 7500, `devices().is_empty()` after Revoke, becomes a check that the row stays with `revoked` set. The dialog's list filters `revoked` rows, so "No device is paired." (the check after it) still shows. Revoke still dispatches `RevokeDevice`.
- **The page's script:** the implementer runs `node --check web/app.js` locally. Node is not a project dependency, so CI does not run it.

## Docs and status

- `README.md`: a Layout row for `web/` (the page, embedded with `include_str!`); the rows for `src/adapters/remote.rs`, `src/core/remote.rs` and `src/ui/remote.rs` updated; the data directory note for `devices.json` gains `revoked`. No new dev aid.
- `CHANGELOG.md`, under Unreleased: one line that a browser on the local network can pair from the pairing window's link and control sessions from a page the Mac serves.
- `docs/remote-client.md`:
  - §4.1: static paths are served by the frame; the pool counts them.
  - §4.3: `GET /`, `GET /app.js` and `GET /api/v1/device` added, and the line "v1 remote is the table above and nothing more" (`:194`) is amended to list them.
  - §5: amends lines 306, 309 and 312; owner call 6.
  - §11: the revocation row and the downgrade note.
- `docs/design.md`: `:3111-3112` ("pairing with a pinned host key") is amended to say native clients pin the key and a browser trusts by click-through. A status entry under "Remote client (2026-10-12)" (`:3107`) names what is built and what is open: the phone run, the Safari and iOS questions in facts Unresolved, and the mislabelled iPad.

## Order inside the commit

The app must run at each step, and the change is one commit.

0. **The spike, first.** `spikes/23-browser-pairing/` with a README, run on the real TLS listener from a phone. It answers: whether Safari stores and sends a `Secure; HttpOnly; SameSite=Lax` cookie set after a click-through, on `fetch` and on the WebSocket handshake; whether the click-through covers the SAN-less certificate; whether the CSP and `no-referrer` hold on a static answer; and how many connections a cold load opens. If that count is more than four, the fallback is `MAX_UNAUTHENTICATED` at 6 (`src/adapters/remote.rs:567`; spec §4.1; the pool test at `tests/remote.rs:3226`). The README records each result and which fallback, if any, the plan takes. No step below is written until step 0's results are in the README.
1. The record and core: tombstone, `pairing_link`, `GET /api/v1/device`.
2. The adapter headers and `RemoteReply::new`.
3. Static answers and `carrier: "cookie"`.
4. The Origin rule and `authenticate`.
5. The UI.
6. Swift.
7. The docs and `spikes/23-browser-pairing/README.md`.

The phone run on the finished build follows, before the change is merged.

## Owner calls, for one pass

1. **Certificate trust by click-through (Decision 1).** Fallback, if step 0 shows no override: a re-issue over the existing Keychain key with a DNS name, as new work with its own renewal design. The costs are: the certificate expires after 825 days, and each re-issue is a new browser exception, so the owner clicks through again; a `.local` name change in a rename changes the origin, so the cookie is lost and the owner pairs again.
2. **Cookie carrier (Decision 2).** Fallback if the cookie is not stored: `localStorage`, which script can read. Fallback if only the WebSocket refuses the handshake: poll the session view.
3. **`?code=` or `#` (Decision 5).** Fallback: `#code=...&key=...&id=...`, which never reaches the host or any `Referer` (facts, Settled). The page reads `location.hash` before it calls `replaceState`, and the Rust builder and the Swift parser change to match.
4. **Tombstone or "not paired or revoked" (The record).** Fallback: no tombstone; the page says "not paired or revoked" for any 401, and the acceptance wording changes.
5. **`GET /api/v1/device` or learn scope by sending (Decision 6).** Fallback: the page sends, shows the reply box, and reports `403 scope` inline.
6. **The browser's trust trade.** A browser trusts the Mac by click-through, and the key in the URL is an address check only (facts, Settled). It lands in three places, and each contradicts a settled or spec line: the browser trust model (`docs/remote-client.md:309` and `docs/design.md:3111-3112` promise a pinned key before the code is sent); the static paths answered before authentication (facts, Settled: only `/info` and `/pair`; `src/app.rs:3028-3029`); and the cookie against spec §5 (`docs/remote-client.md:312`: "sent only as a bearer header"). Accept all three, or drop the browser path and keep pairing native-only. No fallback restores the pin in a browser.
7. **Downgrade (The record).** An older build reads a revoked row as live, because `PairedDevice` has no `deny_unknown_fields` (`src/ports/remote.rs:95`) and `devices.json` has no version. Fallback: a versioned wrapper `{schema_version, devices}`, which reads both the bare array and the wrapper, and refuses to write a newer version, as the store rule in `docs/design.md:125-128` requires. The first save after the upgrade writes the wrapper.

## Commit message

Subject, one line: `Serve a browser control page on the remote port and pair from the QR's link`

Body: the list under Contract additions, copied as written. It names each addition and the one behaviour change, `AppAction::RevokeDevice` now sets `revoked` on the row instead of removing it. The spike directory `spikes/23-browser-pairing/` and its README are part of this commit.

## Contract additions

- `RemoteRequest` (`src/adapters/remote.rs:360`): `host: Option<String>`, `origin: Option<String>`, `cookie: Option<String>`.
- `RemoteReply` (`:389`): `headers: Vec<(&'static str, String)>`; constructor `RemoteReply::new(status, body, authenticated)`.
- `PairedDevice` (`src/ports/remote.rs:96`): `revoked: Option<SystemTime>`, `#[serde(default)]`.
- `core::remote::pairing_link(&str, u16, Uuid, &str, &str) -> String`.
- Routes: `GET /` and `GET /app.js` (static, unauthenticated, answered by the frame); `GET /api/v1/device` (`{device_id, name, scope}`).
- `POST /api/v1/pair` body field `carrier` (`"cookie"` for a browser: the reply sets `switchboard_token` and has no `token`).
- Cookie `switchboard_token`, set by `/pair` for `carrier: "cookie"` and read by `authenticate`.
- Refusals: `403 origin` and `401 revoked`.
- Swift: `PairingInvite.parse` accepts the URL form, in addition to the JSON.
- No `AppAction` variant and no `Effect` is added. Behaviour change to an existing variant: `AppAction::RevokeDevice` now sets `revoked` on the row and no longer removes it (`src/core/remote.rs:86-92`).
- No schema version changes: `SCHEMA_VERSION` 16, `VIEWS_SCHEMA_VERSION` 7, `IDENTITY_SCHEMA_VERSION` 1.
