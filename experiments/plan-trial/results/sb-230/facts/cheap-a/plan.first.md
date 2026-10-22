# Plan: #230, a web control surface on the remote port

Written against HEAD `7fe8949` on `dispatch/230-a-web-control-surface-the-remote-port-se`, tree clean. Every file and line below was checked in this worktree. Facts are cited from `facts.md` by their section; "Settled" lines are relied on as given, "Unresolved" and "For the owner" lines are decided here and say so.

## Where this departs from the issue

- **The card's one-line excerpt is not on the wire.** The issue's list rows show "the one-line excerpt the card shows". In the tree the excerpt is computed in the UI: an agent card takes it from the UI's conversation cache (`src/ui/cards.rs:176–180`, `agent_excerpt` at `cards.rs:558`), other kinds take `cx.state.captions`. `last_message` exists only on the hook event (`src/ports/events.rs:24`), not on `SessionRecord`. `docs/remote-client.md:207` says it is "a field of the session's metadata"; the tree contradicts that line. This cut shows kind, the card word (`SessionView.card`), `waiting` and `waiting_reason` on each row, and no excerpt. The excerpt follows in its own issue, which persists `last_message` on the record and bumps `SCHEMA_VERSION`. *Owner call.* If the excerpt is wanted now, that is the change to make; the alternative, a `last-response` read per row, parses a transcript per session through the content worker, and I do not recommend it.
- **The device's own scope has no read route.** The issue's header shows the scope. Scope appears only in the `/pair` reply (`src/app.rs:3662`); no arm of `serve_remote` (`src/app.rs:3028–3110`) returns it. This cut adds `GET /api/v1/device`. *Owner call.* The fallback is to store the scope from the `/pair` reply in the browser and label it as possibly stale.
- **"Revoked" and "rotated" get the same answer.** Both get 401 "the token is not known; pair again" (`src/app.rs:3146`). The page tells them apart by comparing `/info`'s `spki_sha256` with the key it pinned. A different key is "identity changed". The same key with a 401 means the device's record is gone, and on this host a record goes only by revoke or by key rotation (rotation clears `devices.json`, `src/app.rs:2809–2817`). No tombstone. *Owner call* (facts "For the owner" 6). The fallback is a tombstone; `devices.json` is a bare array with no version (`src/adapters/remote.rs:52–90`, Settled), so a tombstone also needs a versioned file.
- **The certificate trust the issue assumes is unproven.** The issue says the browser "then trusts that certificate for that address". Nothing in the facts settles that for iOS Safari or Chrome (Unresolved). The first step of the plan is a phone spike that checks it. See Certificate trust below.
- **"The code never stays in the address bar or the history" is not established.** `history.replaceState` replaces the current entry and adds none (Settled), so the page can remove the code. Whether iOS Safari drops the entry the first request made is Unresolved, and that first request already carries the code in its query to whichever host answers. The plan promises that the page removes the code before anything else, and the phone spike checks the back list.
- **The headset's pairing parses a pasted invite.** The issue says the headset pairs "from the Bonjour record and a typed code". In the tree the headset takes the whole invite as pasted text and parses it (`client/Sources/SwitchboardUI/BrowserView.swift:883`). The parse is the part of the headset path this plan changes.

## The record

No record changes in this cut.

- A browser is a `PairedDevice` (`src/ports/remote.rs:96`), saved to `devices.json` like any native device. The page builds its name from the user agent (`Safari on iPhone`, `Chrome on iPad`, and `Browser` when neither matches). The core trims the name to 64 characters (`src/core/remote.rs:170`). The page does not depend on exact user-agent strings: the fallback covers any it does not recognise, and the strings themselves are Unresolved.
- No schema is bumped. Today: `SCHEMA_VERSION` is 16 (`src/core/model.rs:12`), `VIEWS_SCHEMA_VERSION` is 7 (`src/core/model.rs:612`), `IDENTITY_SCHEMA_VERSION` is 1 (`src/ports/remote.rs:17`). `devices.json` is unversioned (Settled), so a browser record needs no change.
- Static files are not records. They are compiled in (see Transport).
- The browser's own storage is a per-viewer convenience, not a record and not trust. Three `localStorage` keys: `switchboard.server_id`, `switchboard.pin` (the SPKI the page pinned at pairing, used only to tell "identity changed" from "revoked"), and `switchboard.token`. Token Transport below says why the token sits there.
- The pairing window stays in memory (`PairingWindow`, Settled).

## Signals

No new signal. The page uses the events that exist on `/api/v1/events`: `hello`, `changed {session}` (the metadata diff; a session that starts waiting moves to the top because the page sorts on `waiting`), `changed {waiting}` for membership, `changed {conversation}` and `changed {screen}` for the open session, `removed`, and `resync_required` (`docs/remote-client.md` §4.4). The page keeps one held revision per resource key and applies a reply only at or past it (§4.2).

## Actions and effects

No new `AppAction`, `Effect` or port method. `GET /api/v1/device` is answered on the frame from core state, the way `remote_set` is (`src/app.rs:3676`).

### Routes

- `GET /`, `GET /app.css`, `GET /app.js`: the page. No authentication. Answered on the connection thread, not the frame (see Pool). Routing ignores the query; the query is never logged.
- `GET /api/v1/device` (new; any scope): `wire::Body::Device { id, name, scope }` from the authenticated `PairedDevice`. Added to the match in `serve_remote`, after the existing `ops` arm.
- Existing routes unchanged. The page reads `/api/v1/projects`, `/projects/{id}/sessions`, `/waiting`, `/sessions/{id}`, `/sessions/{id}/last-response` (Claude Code sessions), `/sessions/{id}/screen` (other kinds), and posts `/sessions/{id}/send` (control scope only). The open session's content is watched with the socket's `watch` frame, as §4.4 describes.
- `GET /api/v1/info` is unchanged; the page reads `server_name`, `spki_sha256`, `server_id` and `pairing_open` from it.

### Transport (`src/adapters/remote.rs`, new `src/adapters/remote_page.rs`)

- `read_request` (`src/adapters/remote.rs:444`) keeps two more headers. `Host` goes into a new `RemoteRequest.host: Option<String>`. `Sec-WebSocket-Protocol` goes into `RemoteRequest.protocols: Vec<String>`. When there is no `Authorization` header, the first offered protocol that is not `switchboard.v1` becomes `bearer`. Like `bearer`, it is never logged; the field doc says so. `RemoteRequest` is built in one place (`src/adapters/remote.rs:506`).
- `respond_upgrade` (`src/adapters/remote.rs:519`) takes `protocol: Option<&str>` and echoes `switchboard.v1` only when the client offered it. With no protocol offered it sends no protocol header (RFC 6455 §4.2.2, Settled). One call site (`src/adapters/remote.rs:853`).
- Page files go through a new `remote_page::respond`, not `respond`. `respond` (`src/adapters/remote.rs:530`) hard-codes JSON and has four call sites; leaving it alone means no signature churn. The file table is built with `include_str!` (or `include_bytes!`), a macro that reads a file at compile time into a `&'static str`, so a missing file fails the build rather than a request. The paths are `/` → `web/index.html`, `/app.css` → `web/app.css`, `/app.js` → `web/app.js`.
- Headers on every page reply:
  - `Content-Type`: `text/html; charset=utf-8`, `text/css; charset=utf-8`, `text/javascript; charset=utf-8`.
  - `Cache-Control: no-store`. The page is three small files, so nothing is cached across builds.
  - `Content-Security-Policy: default-src 'none'; script-src 'self'; style-src 'self'; connect-src 'self' wss://{host}; base-uri 'none'; form-action 'none'; frame-ancestors 'none'`, where `{host}` is the request's `Host` value. `connect-src 'self'` does not cover websockets in all browsers (Settled), so the socket is named.
  - `X-Content-Type-Options: nosniff`, `Referrer-Policy: no-referrer` (a referrer would carry the query), and `Connection: close`.
- Only `GET` on the three paths is answered. Any other request still goes to the frame and gets today's 404 (`src/app.rs:3023`). A page request without a `Host` gets 400.
- The HTML has no inline script and no `style` attribute. It declares `<link rel="icon" href="data:,">` so browsers do not request `/favicon.ico`. That the iOS browsers honour this is unverified and is checked in the spike.
- The page's path check and the spec's allowlist: the three paths are the only unauthenticated paths besides `/info` and `/pair`.

### Certificate trust (decision 1, *owner call*)

Plan: **the click-through is the trust step.** The browser warns on the first visit; the owner continues; the pairing window and the paired-devices dialog say so.

Why click-through and not a downloaded certificate:
- The native pin is the SPKI and ignores names (`src/adapters/remote.rs:189–205`). A browser cannot check an SPKI, so the browser has no equivalent of the native pin either way.
- The certificate has no SAN, only a CN (Settled, `src/adapters/remote.rs:102`), and Chrome requires a SAN (Settled). The warning therefore covers the name and the issuer. Neither route removes that warning.
- The download route needs a second pre-pairing path (a §5 amendment) and an iOS profile install nobody has verified (Unresolved). It buys nothing a browser can check.

What it buys: the code and token go only to a host the owner accepted after reading the warning, on the network they paired on. The URL's `key` makes the page refuse to pair with a host whose `/info` key differs. What it does not buy: a host that answers the address is the host that receives the code in the first request, and a page served by an impostor is not a genuine page. The key check is advisory. `/info` is self-reported, and anyone who has seen the QR can copy the key into an impostor's `/info`. The check catches a different installation at a reused address. The code expires in five minutes, and three wrong codes close the window (existing behaviour).

Fallback, if the spike shows the exception is not kept per address on iOS: the owner chooses between re-accepting the warning on every visit (the native app stays the path) and a downloaded certificate (the §5 amendment above). This cut does not reissue the certificate. A reissue with an IP SAN from the same key keeps the SPKI pin (Settled, `src/adapters/remote.rs:131–155`) and is the only route to a warning about the issuer alone; it is a later change.

The §5 rule holds: before pairing, a non-loopback bind serves only `/info`, `/pair` and the three page paths.

### Token transport (decision 2, *owner call*)

Plan: **no cookie.** The page sends `Authorization: Bearer` on `fetch`. The socket opens with `new WebSocket(url, ["switchboard.v1", token])`, and the host reads the token from the second protocol value (see Transport) and echoes `switchboard.v1`. Native clients keep the bearer header.

Why:
- A browser never attaches the bearer header or a subprotocol value on its own (facts "For the owner" 2; the tree has no ambient credential today). So no cross-site page can ride the credential. Nothing here needs an `Origin` check for that reason, and none is built.
- A cookie would make CSRF possible. It would need `Cookie` and `Set-Cookie` handling, an `Origin` check on every POST and on the upgrade, and the behaviour of `Secure` cookies through a certificate exception, which is Unresolved. The cookie route also needs `Origin` (RFC 6455 §4.1 item 8 says a browser must send it).
- The subprotocol route is the one the tree can carry with two header changes and one reply change.

Where it lives: `localStorage`, because a reload must not need a new code. Any script on the origin can read it. The CSP admits only `web/app.js`, so the page's own script is the only reader. The paired-devices dialog says what a browser holds and that revoking the device ends it on the next request (facts §4). The token never appears in a URL, a query or a cookie.

Fallback, if the owner rejects `localStorage`: `sessionStorage`, which means a new pairing per tab. The rest of the flow is the same.

### Pool and rate window (decision 4)

- **Frame or connection thread.** *Owner call* (facts "For the owner" 5). The plan answers page files on the connection thread: they are constants, need no core state, and cost no drain of the eight per pass (`src/app.rs:1587`). The adapter stays thin because the reply is one `include_bytes!` slice.
- **Pool.** A page reply sets `Connection: close`, so the connection ends when its reply is written. A page file holds an unauthenticated slot only while it is being written, not for the 30-second idle timeout (`src/adapters/remote.rs:573`). The pool stays at 4 and §4.1 is unchanged. A page load is three files plus `/pair`; the browser's parallel requests can still exceed four for an instant, and the fifth is dropped at accept (`src/adapters/remote.rs:701–704`). The spike measures that. The fallback is a larger unauthenticated pool, which needs a §4.1 amendment.
- **Own pool or shared.** *Owner call.* Shared, with the close above. The fallback is a separate pool of four inside the sixteen-connection total, which needs a §4.1 amendment.
- **Rate window.** A new `STATIC_PER_MINUTE: usize = 120` beside `PAIR_PER_MINUTE` and `INFO_PER_MINUTE` (`src/adapters/remote.rs:583–584`). `Limits::limited` (`src/adapters/remote.rs:631`) counts the page paths against it, and over the limit gets the same 429 `/pair` gets.
- **Logging.** The page handler never logs a query, and `refuse` already logs only the method and path (`src/app.rs:3754`).

### Spike first (CLAUDE.md's rule for a new risky mechanism)

`spikes/23-web-page`, or the next free number at implementation time (22 is the last spike today). Host half, measured before the adapter changes: a raw WebSocket client offering `["switchboard.v1", token]` gets exactly one echoed protocol; page connections close and the pool count returns; the `Host`-built CSP is what the browser would read. Phone half, run by the owner on an iPhone and in Chrome: the warning and its click-through; whether the exception survives a relaunch; whether `wss://` opens under the exception with the subprotocol; `history.replaceState` and the back list; `localStorage` after a relaunch; whether `data:,` stops the favicon request; the user-agent string the name is built from. The spike README is the record. The owner reads it before the phone walk.

Gate: if `wss://` fails under the exception, the page polls `/api/v1/waiting` and the open session every three seconds while visible. "Moves to the top" then holds within three seconds, and the events socket is a later change.

### Pairing URL (decision 3, *owner call*)

Plan: **one QR, and it is a URL.** The window's QR and "Copy invite" carry `https://{address}/?code={code}&key={spki_sha256}&id={server_id}`. A pure function `pairing_invite_url(address, code, spki_sha256, server_id) -> String` in `src/core/remote.rs` builds it, so the core can test it and `src/ui/remote.rs` only draws it. The text beside the QR is the same URL. The host stops producing the JSON.

Native paths, which must keep working:
- `PairingInvite.parse` (`client/Sources/SwitchboardProtocol/Info.swift:114–116`) accepts the `https://` form: `serverID` from `id`, `spkiSha256` from `key`, `code`, and `addresses` set to the URL's authority. It keeps the JSON branch, because a host built before this change still shows JSON and the headset may paste it.
- `switchboard-remote pair <text>` (`client/Sources/switchboard-remote/main.swift:110–111`) and the headset paste (`client/Sources/SwitchboardUI/BrowserView.swift:883`) both call `parse`, so neither changes. The three-argument form stays.
- The `SWITCHBOARD_INVITE` dev aid (`BrowserView.swift:106`) takes whatever `parse` takes.

Empty address: `local_address()` (`src/adapters/remote.rs:962`) can return nothing, and `src/ui/remote.rs:157–164` today still draws a JSON with an empty `addresses` list. The window now draws no QR and says the Mac has no LAN address.

Several LAN addresses. *Owner call* (facts "For the owner" 9). `local_address` returns one, so the URL carries one. The fallback is one URL per address, listed in the window.

Two payloads. *Owner call.* The fallback, if the owner wants the JSON kept for native paste, is a URL QR plus a "Copy for the headset" button that copies the JSON. Both parse, so no native change is needed for either.

Measurement: whether the URL's QR is smaller or larger than the JSON's at the dialog's width is a run question (facts). The spike or the implementer measures it and records the number.

### Revoked, rotated and identity (page behaviour, no host change)

Any 401 from the API, or a socket that closes with the device gone, makes the page fetch `/api/v1/info`. A `spki_sha256` that differs from `switchboard.pin` is "identity changed; pair again". The same key is "this device was revoked; pair again". Both clear `switchboard.token` and `switchboard.pin`. The host keeps its current 401 wording.

## UI

### The pairing window and the paired-devices dialog (`src/ui/remote.rs`)

- The QR and the text beside it carry the URL (above). A line above the QR says: "Scan with the phone's camera. Your browser will warn about this Mac's certificate: continue only on this network. The code expires in five minutes."
- The paired-devices dialog (existing) lists browser devices like any other, with their name, scope and Revoke. One added line under the list says that a browser holds its token in its own storage and that Revoke ends its access on its next request. The line states the rule plainly: a browser that is forgotten in the page still holds its token until it is revoked here.
- The scope chosen in the window applies as today. A read-scope device may open the page and sees the sessions with no reply box. *Owner call.* The fallback is control-only, and the window then says so.

### The page (`web/`)

Three files: `web/index.html` (markup and the `data:,` icon), `web/app.css` (phone width, a 16px side gutter, light and dark through `prefers-color-scheme`), and `web/app.js` (plain script, no modules, no build, no dependency). They are embedded by `src/adapters/remote_page.rs`.

Flow:

1. **Pairing from the URL.** On load, if the query has `code`, `key` and `id`, the first action is `history.replaceState(null, '', '/')`. The three values then live in memory only. The page calls `GET /api/v1/info` and requires `server_id` to equal `id` and `spki_sha256` to equal `key`. A mismatch shows "This address answers as a different Mac. Check that the code is from this Mac's window" and posts nothing. On a match it posts `POST /api/v1/pair` with `{code, device_name, spki_sha256: key}`, stores the token, `id` and `key` as above, and drops the code from memory.
2. **Signed in.** With a stored token, the page reads `/api/v1/device` (name and scope) and `/api/v1/info` (host name), then the projects and their sessions. The header shows the host name, the device name and the scope.
3. **Not signed in.** With no token and no code, the page says "Open Remote devices on the Mac and scan its code", and shows "A pairing window is open" or "No pairing window is open" from `pairing_open`. No secret is involved.
4. **Lost access.** As in Revoked, rotated and identity above. The page says which, plainly.
5. **The list.** Projects in order, each with its sessions. Waiting sessions sort first and carry a marker and `waiting_reason`. Each row shows its name, kind and card word. Sorting is client-side on `waiting`.
6. **The session.** A Claude Code session shows its last response from `last-response`, with its turn number. Other kinds show the screen. Both refresh on the matching `changed` event for the open session, and the screen is watched with the socket's `watch` frame. The excerpt is not shown (see departures).
7. **Reply.** Shown only when the scope is control. It posts `send` with `{op, text}`, where `op` is a fresh UUID: `crypto.randomUUID()` where it exists, otherwise a UUID built from `crypto.getRandomValues`. Availability of `randomUUID` in the target iOS versions is Unresolved, so the fallback is required. On a 403 "this device reads only" the page re-reads `/api/v1/device` and hides the box.
8. **Socket.** `wss://` at `location.host`, with `["switchboard.v1", token]`. On `resync_required` the page refetches the open collections. On close it reconnects with a back-off and runs the check in Revoked, rotated and identity.
9. **Forget this browser.** Clears the three `localStorage` keys. It says plainly that the device still exists on the Mac until it is revoked there.

The browser name comes from `navigator.userAgent`: `iPhone`, `iPad`, `Mac`, `Android` or `Windows` for the device, and `Safari`, `Chrome` (including `CriOS`), `Firefox` or `Edge` for the browser, joined as "{browser} on {device}". Anything else is `Browser`. `localStorage` reads and writes are wrapped so that a private window that blocks storage still gives a session that works until it is reloaded.

## Tests

`tests/remote.rs`:

- A new helper `http_raw` returns status, headers and body bytes. The existing `http` (`tests/remote.rs:245`) returns only status and body, and `call` (`tests/remote.rs:299`) panics on anything that is not JSON, so neither can check the page.
- `page_files_carry_their_headers`: `/`, `/app.css` and `/app.js`, unpaired and paired, return 200, their `Content-Type`, `Cache-Control: no-store`, `X-Content-Type-Options: nosniff`, `Referrer-Policy: no-referrer`, and the CSP with the `Host` the request sent.
- `page_refuses_other_static_paths`: `/index.html`, `/app.js/`, `/favicon.ico` and any `/api/v1/` path unpaired return what they return today; `POST /` gets 404.
- `page_request_without_host_is_400`.
- `page_connections_free_their_slots`: four page connections open at once, each read to its end; a fifth afterwards is answered, not dropped at accept. The same with eight in sequence.
- `page_rate_window`: `STATIC_PER_MINUTE` plus one page requests in a minute, the last one gets 429.
- `pairing_url_round_trip`: build the URL with `pairing_invite_url` from the live identity and an open window; parse its query; `GET /api/v1/info` matches `id` and `key`; `POST /api/v1/pair` with `device_name: "Safari on iPhone"` returns a token; `GET /api/v1/device` reports the window's scope; an upgrade to `/api/v1/events` with `["switchboard.v1", token]` answers 101 with `Sec-WebSocket-Protocol: switchboard.v1` and sends `hello`; an upgrade with no offer gets no protocol header; the token never appears in the URL.
- `revoked_and_rotated_are_told_apart_by_info`: after `RevokeDevice`, `/api/v1/device` gets 401 and `/api/v1/info` keeps its key; after a rotation, `/info` returns a different key.
- `page_references_only_served_paths`: `include_str!` of `web/app.js` and `web/index.html`. Every relative reference and every `/api/v1/` prefix the script contains must appear in a list in the test, which mirrors the arms of `serve_remote`. A paired token must get something other than 404 "not available on this port" from each listed path.
- The existing `stalled_strangers_fill_their_own_pool_and_never_a_paired_clients` (`tests/remote.rs:3226`) must still pass with `Connection: close` on page replies.
- The existing pairing, scope and revocation tests are unchanged.

`tests/ui.rs`:

- `tests/ui.rs:7487–7492` pins the JSON QR payload. It is amended to expect the URL in the window's invite text and in the Copy invite value, and to check that the address is absent when no LAN address is found.
- A dialog test: a browser device lists by its name with its scope, and the Revoke line for it is present.

`src/core/tests.rs`:

- `pairing_invite_url` builds the expected string; the code appears only in the query; an empty address yields no URL.

Swift (`client/Tests/SwitchboardProtocolTests`):

- `PairingInvite.parse` accepts the URL form and still accepts the JSON. This is not run here. The implementer runs `swift test` on a Mac and runs `switchboard-remote pair "<url>"` once by hand.

Fixtures (`client/Fixtures`): unchanged. `/api/v1/device` has no native reader in this cut, so there is nothing for Swift to decode. A fixture is added when a native client reads it.

The page's JavaScript has no automated test, and no test drives a browser. The phone walk is the browser test. It is run by the owner and its results go into the spike README and the design status.

Gates before commit: `cargo test --locked --workspace`, `cargo clippy --locked --workspace --all-targets -- -D warnings`, `cargo fmt --all`. Nothing outside a `cfg(target_os = "macos")` table is added, so the Linux clippy check is not needed for the dispatch crate.

## Spec, README, status and changelog

- `docs/remote-client.md` §4.1: page files on the connection thread, `Connection: close`, `STATIC_PER_MINUTE`. §4.3: `GET /api/v1/device` and the three page paths. Line 207: the excerpt is not yet on the wire; the follow-up is named. §5, line 306: "before anything but `/info` and `/pair`" gains the three page paths and the click-through trust step. A new paragraph covers the browser's token: the subprotocol, `localStorage`, no cookie, no token in a URL, and the URL form of the pairing invite.
- `docs/design.md`, "## Remote client (2026-10-12)" (line 3107): a dated status line for the page, what is built, and its known gaps. The gaps are no automated browser test, no excerpt, the click-through trust, and revocation shown by inference.
- `README.md`: a Layout row for `web/` (the three files, embedded at compile time) and one for `src/adapters/remote_page.rs` beside the `src/adapters/remote.rs` row (line 383). The `src/ui/remote.rs` row (line 423) says the QR is a URL. The `tests/remote.rs` row (line 437) lists the page tests. No dev aid is added.
- `CHANGELOG.md`, under Unreleased (line 9): "The remote port serves a control page for a browser on the local network. Pair it by scanning the QR, which now holds a URL."

## Order of work

One commit. The app builds and runs after each step.

1. Spike `spikes/23-…`: host half first, then the phone half by the owner. No app change.
2. Page files, headers, the connection-thread path, the rate window; the page tests in `tests/remote.rs`.
3. The URL in `pairing_invite_url` and the window; `PairingInvite.parse`; the UI and core tests.
4. `Host` and `Sec-WebSocket-Protocol` parsing, the echo, `GET /api/v1/device`, and the revocation test.
5. `web/app.js` and `web/app.css`.
6. Spec, README, design status and changelog.

Commit message:

`Serve a browser control page from the remote port, paired from a URL in the QR`

## Contract additions

- `src/adapters/remote.rs`, `RemoteRequest`: new fields `host: Option<String>` and `protocols: Vec<String>`.
- `src/adapters/remote.rs`, `respond_upgrade`: new parameter `protocol: Option<&str>` (one call site).
- `src/adapters/remote.rs`: new constant `STATIC_PER_MINUTE: usize = 120`.
- `src/adapters/remote_page.rs` (new): the page's file table and `respond`.
- `wire::Body::Device { id, name, scope }` and the route `GET /api/v1/device`.
- `src/core/remote.rs`: new pure function `pairing_invite_url(address, code, spki_sha256, server_id) -> String`.
- Wire, unauthenticated: `GET /`, `GET /app.css`, `GET /app.js`.
- Wire, events socket: the `switchboard.v1` subprotocol is echoed, and a bearer token may be offered as the second protocol value.
- Wire, pairing: the invite is a URL. `PairingInvite.parse` accepts it as an addition, and the JSON is still accepted.
- Nothing changes in `src/core/model.rs`, `src/core/action.rs` or `src/ports/`. `SCHEMA_VERSION` stays 16, `VIEWS_SCHEMA_VERSION` stays 7, `IDENTITY_SCHEMA_VERSION` stays 1, and `devices.json` keeps its shape.
