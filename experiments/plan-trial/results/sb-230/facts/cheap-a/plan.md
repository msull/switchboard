# Plan: #230, a web control surface on the remote port

Written against HEAD `7fe8949` on `dispatch/230-a-web-control-surface-the-remote-port-se`, tree clean. Every file and line below was checked in this worktree. Facts are cited from `facts.md` by their section; "Settled" lines are relied on as given, "Unresolved" and "For the owner" lines are decided here and say so.

## Where this departs from the issue

- **The card's one-line excerpt is not on the wire.** The issue's list rows show "the one-line excerpt the card shows". In the tree the excerpt is computed in the UI: an agent card takes it from the UI's conversation cache (`src/ui/cards.rs:176–180`, `agent_excerpt` at `cards.rs:558`), other kinds take `cx.state.captions`. `last_message` exists only on the hook event (`src/ports/events.rs:24`), not on `SessionRecord`. `docs/remote-client.md:207` says it is "a field of the session's metadata"; the tree contradicts that line. This cut shows kind, the card word (`SessionView.card`), `waiting` and `waiting_reason` on each row, and no excerpt. The excerpt follows in its own issue, which persists `last_message` on the record and bumps `SCHEMA_VERSION`. *Owner call.* If the excerpt is wanted now, that is the change to make; the alternative, a `last-response` read per row, parses a transcript per session through the content worker, and I do not recommend it.
- **The device's own scope has no read route.** The issue's header shows the scope. Scope appears only in the `/pair` reply (`src/app.rs:3662`); no arm of `serve_remote` (`src/app.rs:3028–3110`) returns it. This cut adds `GET /api/v1/device`. *Owner call.* The fallback is to store the scope from the `/pair` reply in the browser and label it as possibly stale.
- **"Revoked" and "rotated" get the same answer.** Both get 401 "the token is not known; pair again" (`src/app.rs:3146`). The page tells them apart by comparing `/info`'s `spki_sha256` with the key it pinned. A different key is "identity changed". The same key with a 401 means the device's record is gone, and on this host a record goes only by revoke or by key rotation (rotation clears `devices.json`, `src/app.rs:2809–2817`). No tombstone. *Owner call* (facts "For the owner" 6). The fallback is a tombstone; `devices.json` is a bare array with no version (`src/adapters/remote.rs:52–90`, Settled), so a tombstone also needs a versioned file.
- **The headset's pairing parses a pasted invite.** The issue says the headset pairs "from the Bonjour record and a typed code". In the tree the headset takes the whole invite as pasted text and parses it (`client/Sources/SwitchboardUI/BrowserView.swift:883`). The parse is the part of the headset path this plan changes.

## The record

No record changes in this cut.

- A browser is a `PairedDevice` (`src/ports/remote.rs:96`), saved to `devices.json` like any native device. The page names it from the user agent: `Safari on iPhone` or `Chrome on iPad`, else `Browser`. The core trims the name to 64 characters (`src/core/remote.rs:169`). The user-agent strings themselves are Unresolved, so the `Browser` fallback is what an unrecognised agent gets.
- The browser's own storage is a per-viewer convenience, not a record and not trust. Three `localStorage` keys: `switchboard.server_id`, `switchboard.pin` (the SPKI the page pinned at pairing, used only to tell "identity changed" from "revoked"), and `switchboard.token`. Token transport below says why the token sits there.

No schema is bumped. `SCHEMA_VERSION` is 16 (`src/core/model.rs:12`), `VIEWS_SCHEMA_VERSION` is 7 (`src/core/model.rs:612`), `IDENTITY_SCHEMA_VERSION` is 1 (`src/ports/remote.rs:17`), and `devices.json` is unversioned (Settled), so a browser record needs no change.

## Signals

No new signal. The page uses the events `docs/remote-client.md` §4.4 already defines.

## Actions and effects

No new `AppAction`, `Effect` or port method. `GET /api/v1/device` is answered on the frame from core state, and its reply is built with `serde_json::json!`, as `remote_info` builds its own (`src/app.rs:3568`).

### Routes

- `GET /`, `GET /app.css`, `GET /app.js`: the page. No authentication. Answered on the connection thread, not the frame (see Pool). Routing ignores the query; the query is never logged.
- `GET /api/v1/device` (new; any scope): `{"id", "name", "scope"}` from the authenticated `PairedDevice`, in the envelope like every other remote reply. Added to the match in `serve_remote` (`src/app.rs:3028`), after the existing `ops` arm. The control port's `Body` enum (`src/ports/dispatch.rs`, imported at `src/app.rs:42`) is not touched; the remote replies are all `json!`.
- Existing routes unchanged. The page reads `/api/v1/projects`, `/projects/{id}/sessions`, `/waiting`, `/sessions/{id}`, `/sessions/{id}/last-response` (Claude Code sessions), `/sessions/{id}/screen` (other kinds), and posts `/sessions/{id}/send` (control scope only). The open session's content is watched with the socket's `watch` frame, as §4.4 describes.

### Transport (`src/adapters/remote.rs`, new `src/adapters/remote_page.rs`)

- `read_request` (`src/adapters/remote.rs:444`) keeps two more headers. `Host` goes into a new `RemoteRequest.host: Option<String>`. `Sec-WebSocket-Protocol` goes into `RemoteRequest.protocols: Vec<String>`. When there is no `Authorization` header, the first offered protocol that is not `switchboard.v1` becomes `bearer`. The token is base64url without padding (`src/adapters/remote.rs:334–340`), so every character is a valid subprotocol token and a token offered that way parses as one. Like `bearer`, it is never logged; the field doc says so. `RemoteRequest` is built in one place (`src/adapters/remote.rs:506`).
- `respond_upgrade` (`src/adapters/remote.rs:519`) takes `protocol: Option<&str>` and echoes `switchboard.v1` only when the client offered it. With no protocol offered it sends no protocol header (RFC 6455 §4.2.2, Settled). One call site (`src/adapters/remote.rs:853`).
- Page files go through a new `remote_page::respond`, not `respond`, which hard-codes JSON (`src/adapters/remote.rs:530`). The file table maps `/` to `web/index.html`, `/app.css` to `web/app.css` and `/app.js` to `web/app.js`, each built with `include_str!`.
- Headers on every page reply:
  - `Content-Type`: `text/html; charset=utf-8`, `text/css; charset=utf-8`, `text/javascript; charset=utf-8`.
  - `Cache-Control: no-store`.
  - `Content-Security-Policy: default-src 'none'; script-src 'self'; style-src 'self'; connect-src 'self' wss://{host}; base-uri 'none'; form-action 'none'; frame-ancestors 'none'`, where `{host}` is the request's `Host` value. `connect-src 'self'` does not cover websockets in all browsers (Settled), so the socket is named.
  - `X-Content-Type-Options: nosniff`, `Referrer-Policy: no-referrer` (a referrer would carry the query), and `Connection: close`.
- Only `GET` on the three paths is answered. Any other request still goes to the frame and gets today's 404 (`src/app.rs:3023`). A page request whose `Host` is missing, or holds anything but `[A-Za-z0-9.:\[\]-]`, gets 400. The check runs before the CSP is built, so a `;` or a space in `Host` never reaches a header; the CSP's `{host}` is therefore a bare authority.
- The HTML has no inline script and no `style` attribute.

### Certificate trust (decision 1, *owner call*)

Plan: **the click-through is the trust step.** The browser warns on the first visit; the owner continues; the pairing window and the paired-devices dialog say so.

Why click-through and not a downloaded certificate:
- The native pin is the SPKI and ignores names (`src/adapters/remote.rs:189–205`). A browser cannot check an SPKI, so the browser has no equivalent of the native pin either way.
- The certificate has no SAN, only a CN (Settled, `src/adapters/remote.rs:102`), and Chrome requires a SAN (Settled). The warning therefore covers the name and the issuer. Neither route removes that warning.

What it buys: the code and token go only to a host the owner accepted after reading the warning, on the network they paired on. The URL's `key` makes the page refuse to pair with a host whose `/info` key differs. What it does not buy: a host that answers the address is the host that receives the code in the first request, and a page served by an impostor is not a genuine page. The key check is advisory. `/info` is self-reported, and anyone who has seen the QR can copy the key into an impostor's `/info`. The check catches a different installation at a reused address. The code expires in five minutes, and three wrong codes close the window (existing behaviour).

Fallback, if the spike shows the exception is not kept per address on iOS: the owner chooses between re-accepting the warning on every visit (the native app stays the path) and a downloaded certificate, which needs a second pre-pairing path (a §5 amendment) and an iOS profile install that is Unresolved.

The §5 rule holds: before pairing, a non-loopback bind serves only `/info`, `/pair` and the three page paths.

### Token transport (decision 2, *owner call*)

Plan: **no cookie.** The page sends `Authorization: Bearer` on `fetch`. The socket opens with `new WebSocket(url, ["switchboard.v1", token])`, and the host reads the token from the second protocol value (see Transport) and echoes `switchboard.v1`. Native clients keep the bearer header.

A browser never attaches the bearer header or a subprotocol value on its own (facts "For the owner" 2; the tree has no ambient credential today), so no cross-site page can ride the credential. Nothing here needs an `Origin` check for that reason, and none is built.

Where it lives: `localStorage`, because a reload must not need a new code. Any script on the origin can read it. The CSP admits only `web/app.js`, so the page's own script is the only reader. The paired-devices dialog says what a browser holds and that revoking the device ends it on the next request (facts §4). The token never appears in a URL, a query or a cookie.

### Pool and rate window (decision 4)

- **Frame or connection thread.** *Owner call* (facts "For the owner" 5). The plan answers page files on the connection thread: they are constants, need no core state, and cost no drain of the eight per pass (`src/app.rs:1587`).
- **Pool.** `Connection: close` alone ends nothing: the loop in `connection` (`src/adapters/remote.rs:783`, the `while let`) reads the next request after every reply. So the page arm writes its reply and returns `Ok(())` from `connection`, as the subscriber path already does by returning. The accept thread releases the unauthenticated slot when `connection` returns (`src/adapters/remote.rs:719`). A page request over the rate limit takes the same return: the `continue` after the 429 (`src/adapters/remote.rs:788–794`) stays for `/pair` and `/info`, and the page branch returns instead. A page file therefore holds its slot only while it is written, not for the 30-second idle timeout (`src/adapters/remote.rs:573`). Page files share the unauthenticated pool with other unauthenticated connections, and the pool stays at 4 and §4.1 is unchanged. A page load is three files plus `/pair`; the browser's parallel requests can still exceed four for an instant, and the fifth is dropped at accept (`src/adapters/remote.rs:701–704`). The spike measures that. The fallback is a larger unauthenticated pool, which needs a §4.1 amendment.
- **Rate window.** A new `STATIC_PER_MINUTE: usize = 120` beside `PAIR_PER_MINUTE` and `INFO_PER_MINUTE` (`src/adapters/remote.rs:583–584`). `Limits::limited` (`src/adapters/remote.rs:631`) counts the page paths against it, and over the limit gets the same 429 `/pair` gets.

### Spike first (CLAUDE.md's rule for a new risky mechanism)

`spikes/23-web-page`, or the next free number at implementation time (22 is the last spike today). Host half, measured before the adapter changes: a raw WebSocket client offering `["switchboard.v1", token]` gets exactly one echoed protocol; page connections close and the pool count returns; the `Host`-built CSP is what the browser would read. Phone half, run by the owner on an iPhone and in Chrome: the warning and its click-through; whether the exception survives a relaunch; whether `wss://` opens under the exception with the subprotocol; `history.replaceState` and the back list, including whether the first request's history entry is dropped; `localStorage` after a relaunch; the user-agent string the name is built from. The spike README is the record. The owner reads it before the phone walk.

Gate: if `wss://` fails under the exception, the page polls `/api/v1/waiting` and the open session every three seconds while visible. "Moves to the top" then holds within three seconds, and the events socket is a later change.

### Pairing URL (decision 3, *owner call*)

Plan: **one QR, and it is a URL.** The window's QR and "Copy invite" carry `https://{address}/?code={code}&key={spki_sha256}&id={server_id}`. A pure function `pairing_invite_url(address, code, spki_sha256, server_id) -> String` in `src/core/remote.rs` builds it, so the core can test it and `src/ui/remote.rs` only draws it. The text beside the QR is the same URL. The host stops producing the JSON.

Native paths, which must keep working: all of them call `PairingInvite.parse`, so they take the URL with no change: `switchboard-remote pair <text>` (`client/Sources/switchboard-remote/main.swift:110–111`), the headset paste (`client/Sources/SwitchboardUI/BrowserView.swift:883`) and the `SWITCHBOARD_INVITE` dev aid (`BrowserView.swift:106`). `PairingInvite.parse` (`client/Sources/SwitchboardProtocol/Info.swift:114–116`) accepts the `https://` form: `serverID` from `id`, `spkiSha256` from `key`, `code`, and `addresses` set to the URL's authority. It keeps the JSON branch, because a host built before this change still shows JSON. The three-argument form stays.

Empty address: `local_address()` (`src/adapters/remote.rs:962`) can return nothing, and `src/ui/remote.rs:157–164` today still draws a JSON with an empty `addresses` list. The window now draws no QR and says the Mac has no LAN address.

Several LAN addresses. *Owner call* (facts "For the owner" 9). `local_address` returns one, so the URL carries one. The fallback is one URL per address, listed in the window.

Camera fallback. *Owner call* (facts, Unresolved: whether the iOS camera opens an `https://<ip>:<port>/` link). If the spike shows the camera does not offer it, the window keeps the JSON QR as today, which the native paste and the CLI already parse, and shows the URL as text beside it. The phone reaches the page by typing that URL into Safari. The window says so. Nothing else changes.

### Revoked, rotated and identity (page behaviour, no host change)

Any 401 from the API makes the page fetch `/api/v1/info`. A socket that closes after it was open, or fails to open at all, does the same. A failed open gives the page no status: an unauthenticated upgrade is refused with a normal HTTP 401 that has no `Sec-WebSocket-Protocol` header, and a browser reports only a failed handshake, whose `error` and `close` events carry no HTTP status or useful close code. So every failed open runs this check. A `spki_sha256` that differs from `switchboard.pin` is "identity changed; pair again". The same key is "this device was revoked; pair again". Both clear `switchboard.token` and `switchboard.pin`. The host keeps its current 401 wording.

The "same key" answer rests on the host's ways of removing a record. Revoke removes one (`RevokeDevice`, `src/core/remote.rs:86`). A key-missing regeneration makes a new identity with the same `server_id` through `make_identity` and clears `devices` (`src/app.rs:2810–2816`), so its SPKI differs from the pinned one. The no-identity path (`src/app.rs:2836–2839`) makes a new `server_id` as well. The read-only branch keeps both the identity and the devices (`src/app.rs:2807`), so a read-only host does not rotate. Nothing else clears `devices.json`.

## UI

### The pairing window and the paired-devices dialog (`src/ui/remote.rs`)

- The QR and the text beside it carry the URL (above). A line above the QR says: "Scan with the phone's camera. Your browser will warn about this Mac's certificate: continue only on this network. The code expires in five minutes."
- The paired-devices dialog (existing) lists browser devices like any other, with their name, scope and Revoke. One added line under the list says that a browser holds its token in its own storage and that Revoke ends its access on its next request. The line states the rule plainly: a browser that is forgotten in the page still holds its token until it is revoked here.
- The scope chosen in the window applies as today. A read-scope device may open the page and sees the sessions with no reply box. *Owner call* (facts "For the owner" 8). The fallback is control-only, and the window then says so.

### The page (`web/`)

Three files: `web/index.html` (markup), `web/app.css` (phone width, a 16px side gutter, light and dark through `prefers-color-scheme`), and `web/app.js` (plain script, no modules, no build, no dependency). They are embedded by `src/adapters/remote_page.rs`.

Flow:

1. **Pairing from the URL.** On load, if the query has `code`, `key` and `id`, the first action is `history.replaceState(null, '', '/')`. The three values then live in memory only. The page calls `GET /api/v1/info` and requires `server_id` to equal `id` and `spki_sha256` to equal `key`. A mismatch shows "This address answers as a different Mac. Check that the code is from this Mac's window" and posts nothing. On a match it posts `POST /api/v1/pair` with `{code, device_name, spki_sha256: key}`, stores the token, `id` and `key` as above, and drops the code from memory.
2. **Signed in.** With a stored token, the page reads `/api/v1/device` (name and scope) and `/api/v1/info` (host name), then the projects and their sessions. The header shows the host name, the device name and the scope.
3. **Not signed in.** With no token and no code, the page says to open Remote devices on the Mac and scan its code, and shows `pairing_open`.
4. **Lost access.** As in Revoked, rotated and identity above. The page says which, plainly.
5. **The list.** Projects in order, each with its sessions. Waiting sessions sort first and carry a marker and `waiting_reason`. Each row shows its name, kind and card word. Sorting is client-side on `waiting`.
6. **The session.** A Claude Code session shows its last response from `last-response`, with its turn number. Other kinds show the screen. Both refresh on the matching `changed` event for the open session, and the screen is watched with the socket's `watch` frame. The excerpt is not shown (see departures).
7. **Reply.** Shown only when the scope is control. It posts `send` with `{op, text}`, where `op` is a fresh UUID: `crypto.randomUUID()` where it exists, otherwise a UUID built from `crypto.getRandomValues`. Availability of `randomUUID` in the target iOS versions is Unresolved, so the fallback is required. On a 403 "this device reads only" the page re-reads `/api/v1/device` and hides the box.
8. **Socket.** `wss://` at `location.host`, with `["switchboard.v1", token]`. On `resync_required` the page refetches the open collections. A failed open or a close runs the check in Revoked, rotated and identity first, then reconnects with a back-off if the token still stands.
9. **Forget this browser.** Clears the three `localStorage` keys. It says plainly that the device still exists on the Mac until it is revoked there.

The name comes from `navigator.userAgent`: `iPhone` or `iPad` for the device and `Safari` or `Chrome` for the browser, joined as "{browser} on {device}"; anything else is `Browser`. `localStorage` reads and writes are wrapped so that a private window that blocks storage still gives a session that works until it is reloaded.

## Tests

`tests/remote.rs`:

- A new helper `http_raw` returns status, headers and body bytes. The existing `http` (`tests/remote.rs:245`) returns only status and body, and `call` (`tests/remote.rs:299`) panics on anything that is not JSON, so neither can check the page.
- `page_files_carry_their_headers`: `/`, `/app.css` and `/app.js`, unpaired and paired, return 200, their `Content-Type`, `Cache-Control: no-store`, `X-Content-Type-Options: nosniff`, `Referrer-Policy: no-referrer`, and the CSP with the `Host` the request sent.
- `page_refuses_other_static_paths`: `/index.html`, `/app.js/`, `/favicon.ico` and any `/api/v1/` path unpaired return what they return today; `POST /` gets 404.
- `page_host_is_validated`: a request with no `Host`, and one holding `;`, a space or `'`, each get 400 and no `Content-Security-Policy` header.
- `page_connections_free_their_slots`: four page connections open at once. Each client reads its reply and then to end of stream, so the host has closed the socket; without the return from `connection`, the read would wait out the idle timeout and fail. A fifth connection afterwards is answered, not dropped at accept.
- `page_rate_window`: `STATIC_PER_MINUTE` plus one page requests in a minute, the last one gets 429.
- `pairing_url_round_trip`: build the URL with `pairing_invite_url` from the live identity and an open window; parse its query; `GET /api/v1/info` matches `id` and `key`; `POST /api/v1/pair` with `device_name: "Safari on iPhone"` returns a token; `GET /api/v1/device` reports the window's scope; an upgrade to `/api/v1/events` with `["switchboard.v1", token]` answers 101 with `Sec-WebSocket-Protocol: switchboard.v1` and sends `hello`; an upgrade with no offer gets no protocol header; the token never appears in the URL.
- `revoked_and_rotated_are_told_apart_by_info`: after `RevokeDevice`, `/api/v1/device` gets 401 and `/api/v1/info` keeps its key; after a rotation, `/info` returns a different key.

`tests/ui.rs`:

- `tests/ui.rs:7487–7492` pins the JSON QR payload. It is amended to expect the URL in the window's invite text and in the Copy invite value, and to check that the address is absent when no LAN address is found.

`src/core/tests.rs`:

- `pairing_invite_url` builds the expected string; the code appears only in the query; an empty address yields no URL.

Swift (`client/Tests/SwitchboardProtocolTests`):

- `PairingInvite.parse` accepts the URL form and still accepts the JSON. This is not run here. The implementer runs `swift test` on a Mac and runs `switchboard-remote pair "<url>"` once by hand.

## Spec, README, status and changelog

- `docs/remote-client.md`: §4.1 (page files on the connection thread, `STATIC_PER_MINUTE`), §4.3 (`GET /api/v1/device`, the three page paths), §5 (the click-through step, the subprotocol token, the URL invite), and the excerpt line at 207.
- `docs/design.md`: the Remote client status (line 3107).
- `README.md`: Layout rows for `web/` and `src/adapters/remote_page.rs`, and the `src/ui/remote.rs` and `tests/remote.rs` rows.
- `CHANGELOG.md`: Unreleased (line 9).

## Order of work

One commit.

1. Spike `spikes/23-…`: host half first, then the phone half by the owner. No app change.
2. The three page files (`web/index.html`, `web/app.css`, `web/app.js`) and their `include_str!` table in `src/adapters/remote_page.rs`, with the page headers, the `Host` check, the connection-thread path and the rate window, and the page tests in `tests/remote.rs`. The files land here because `include_str!` fails the build without them, and the README rows land with them.
3. The URL in `pairing_invite_url` and the window; `PairingInvite.parse`; the UI and core tests.
4. `Sec-WebSocket-Protocol` parsing, the echo, `GET /api/v1/device`, and the revocation test.
5. Spec, README, design status and changelog.

Commit message:

`Serve a browser control page from the remote port, paired from a URL in the QR`

## Contract additions

- `src/adapters/remote.rs`, `RemoteRequest`: new fields `host: Option<String>` and `protocols: Vec<String>`.
- `src/adapters/remote.rs`, `respond_upgrade`: new parameter `protocol: Option<&str>` (one call site).
- `src/adapters/remote.rs`: new constant `STATIC_PER_MINUTE: usize = 120`.
- `src/adapters/remote_page.rs` (new): the page's file table and `respond`.
- `src/core/remote.rs`: new pure function `pairing_invite_url(address, code, spki_sha256, server_id) -> String`.
- Wire: the route `GET /api/v1/device`, reply `{id, name, scope}` in the envelope; no Rust type is added.
- Wire, unauthenticated: `GET /`, `GET /app.css`, `GET /app.js`.
- Wire, events socket: the `switchboard.v1` subprotocol is echoed, and a bearer token may be offered as the second protocol value.
- Wire, pairing: the invite is a URL. `PairingInvite.parse` accepts it as an addition, and the JSON is still accepted.
