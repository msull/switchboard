# Issue #230: a web control surface on the remote port (investigation notes)

I read CLAUDE.md, docs/lessons.md, docs/remote-client.md §§0, 4.1–4.7 and 5–6, and the remote section of docs/design.md (3107–3584). Line numbers below are as they stand in this worktree (branch `dispatch/230-…`, clean, at 7fe8949). There is no `web/` directory yet.

## The on-disk record and how it is rehydrated

Persistent state on the host is the same as today. The browser adds no new kind of record.

- **`<data dir>/devices.json`**, a `Vec<PairedDevice>` (`src/ports/remote.rs:96-106`): `id`, `name`, `scope`, `token_sha256`, `created`, `last_seen`. A browser is one more entry. The name is capped at 64 chars in `AppCore::new_device` (`src/core/remote.rs:159-170`), and the scope comes from the open window. It is written through `Effect::SaveDevices` (`src/app.rs:2910`). `last_seen` is saved at most every 300 s (`src/core/remote.rs:18`, `DeviceSeen` at 72-85). Nothing in the record says what kind of client a device is, so a "browser" field would be a schema change. The name string alone needs none.
- **`<data dir>/identity.json`** (`Identity`, `src/ports/remote.rs:26-41`, `IDENTITY_SCHEMA_VERSION = 1` at :17): the server id, the name, the base64 DER certificate, `spki_sha256` and `listen {lan, port}`. The key sits in the Keychain under `remote-key:<server_id>`.
  - `make_identity` (`src/adapters/remote.rs:95-127`) builds the certificate with `CertificateParams::new(Vec::<String>::new())`. So it has **no subjectAltName**, only a CN `Switchboard <id>`. Its validity is rcgen's default, which the doc comment gives as running to year 4096. It sets no EKU.
  - If the plan wants a certificate browsers accept more readily (SAN for the IP or a `.local` name, a shorter validity, serverAuth EKU, a download path), the certificate would have to be **reissued over the same key**. The SPKI and every native pin stay valid (§5 says so), but `identity.json`'s `certificate` changes. The file layout does not change.
  - A SAN tied to an IP would go stale when the address moves.
- **The pairing window** is core state only (`PairingWindow`: `until`, `code`, `scope`, `failures`; `src/core/remote.rs:45-52`) and is never saved. The QR payload is built in the UI from it every frame (`src/ui/remote.rs:154-172`).
- **The page itself** is compiled into the binary (`include_str!`), so there is nothing to rehydrate on the host. A page left open on a phone after the host updates itself keeps the old JS until it is reloaded. `/info` already carries `version` (`src/app.rs:3568-3604`).

Rehydration on the device lives in the browser, not in Switchboard:

- **The token** lives in a cookie jar or in `localStorage`, depending on decision 2.
- **The certificate exception** lives in the browser's store.
- **The flow on reload:**
  1. `GET /` with no auth.
  2. `GET /api/v1/info`.
  3. One authenticated read: 200 shows the sessions, 401 shows "pair again / revoked".
  4. Open `/api/v1/events`, then snapshot and reconcile per §4.4. A new `epoch` means refetch everything.

What the tree implies for rehydration:

- Host restart: same certificate, same tokens. The page reconnects and sees a new epoch.
- Key rotation (`src/app.rs:2776-2852` clears `devices.json`): the browser meets a **different certificate** at the same address. Its stored exception presumably no longer applies, so it shows its own interstitial before any page script runs. The page's "identity changed, pair again" state would then not be what the owner sees on a rotation. Whether that holds depends on how the browser scopes the exception (see Open questions).
- Address change (DHCP): a new origin. Cookies and `localStorage` are keyed by host, so the token is not found and the device pairs again, leaving the old record in `devices.json` until revoked.

## What exists, file by file

### `src/adapters/remote.rs` (listener, HTTP, WebSocket)

- `RemoteRequest` (:359-371) keeps only `method`, `path`, `query` (undecoded), `body`, `bearer` and `upgrade` (the `Sec-WebSocket-Key`).
  - `read_request` (:444-516) parses every header into a local `Vec` and then drops all but `content-length`, `authorization: Bearer`, `upgrade` and `sec-websocket-key`.
  - **No `Cookie`, `User-Agent`, `Origin`, `Sec-WebSocket-Protocol`, `Accept` or `If-None-Match` reaches the frame.** Decisions 2 and 6 and any Origin check need new fields here.
- `RemoteReply` (:389-393) is `{status, body, authenticated}`. **It carries no headers.**
  - `respond` (:530-553) writes `Content-Type: application/json` on every reply, plus `Content-Length` and `Connection: keep-alive`, and nothing else. It also has a fixed reason table.
  - Serving HTML/CSS/JS with their types, a CSP, `Cache-Control`, `Set-Cookie`, `X-Content-Type-Options` or a 304 needs `RemoteReply` (or a separate static path) to carry a content type and headers.
  - `respond_upgrade` (:519-527) writes a bare 101 with no `Sec-WebSocket-Protocol`.
  - These are adapter types, not `ports/` contract types, so changing them is not bound by the additive-only rule. Tests construct the request literal at :1045-1061.
- Pools and accept:
  - `MAX_UNAUTHENTICATED = 4`, `MAX_PAIRED = 12` (:567-568).
  - The accept loop (:687-718) closes a 5th unauthenticated TCP connection **at accept, by dropping it**.
  - A connection moves to the paired pool only when a reply comes back with `authenticated: true` (:829-844).
  - So a browser fetching `/`, a CSS file, a JS file and `/api/v1/info` over parallel keep-alive connections stays wholly in the unauthenticated pool. That holds while the static replies are `authenticated: false`, and even with a cookie unless static replies authenticate.
  - Idle connections are held up to `IDLE_TIMEOUT = 30 s` (:573).
  - One browser can therefore fill the four slots and have its next connection dropped. That includes the one that would carry `/pair`.
- Rate windows: `Limits::limited` (:631-640) counts only `PAIR_PATH` (6/min) and `INFO_PATH` (60/min) (:583-587). A static path counts against nothing today. Note that **a page load that calls `/info` spends one of the 60**.
- Deadlines: 5 s for the handshake and the first request line (:570). A browser that opens a connection speculatively and sends nothing for 5 s is closed. That is harmless, but it holds a slot meanwhile.
- `connection` (:770-876) hands every request to the frame over the bounded inbox (64, 503 when full) and waits up to 60 s for the answer. Static files could be answered on the connection thread without reaching the frame, the way the 429 is. Whether they should is a design choice.
- `subscriber` (:878-927) uses `tungstenite::WebSocket::from_raw_socket` with no extensions negotiated.
- `local_address()` (:962-…) is the single IP the QR prints.

### `src/app.rs` (routing and auth)

- `serve_pending` (:1572-…) drains `port.poll(DRAIN_PER_PASS)` and calls `serve_remote` (:1595).
- `serve_remote` (:3017-3123):
  - `strip_prefix("/api/v1/")` with a 404 `not_found` for anything else (:3023-3025). This is the allowlist the issue means: `GET /` is refused here today.
  - `/info` and `/pair` are answered before `authenticate` (:3027-3031). `/events` is routed right after auth (:3036-3038).
  - The remainder is the §4.3 table, and the catch-all is 404 (:3109-3117).
- `authenticate` (:3127-3157) reads only `request.bearer`. Both 401 bodies use code `unauthorized`: "pair this device first" with no token, and "the token is not known; pair again" with an unknown token. **A revoked device and a device whose pairing was cleared by key rotation get the same answer.** Only the message differs from the no-token case.
- `remote_pair` (:3606-3674):
  - It requires the body's `spki_sha256` to equal the identity's and the code to match (upper-cased). Each mismatch dispatches `PairingFailed`.
  - `device_name` comes from the body (default "Device"). The host does not derive a name.
  - The reply is the enveloped `{token, scope, server_id, device_id}`. No header is set.
- `remote_info` (:3568-3604): `server_id`, `server_name`, `protocol`, `capabilities`, `spki_sha256`, `version`, `limits`, `pairing_open`.
- `remote_subscribe` (:2723-2759) needs `request.upgrade` and the subscriber queue, then sends `hello`. `scope=all` comes from the query.
- Revocation closes the device's sockets at :486-495 by dropping its subscriber queues. The connection thread then sends a WebSocket close with no code or reason (:893-899). The browser's next upgrade attempt is answered 401 (see Open questions on what the page can see of that).
- `refuse` (:3754-3772) logs `method` and `path` only. The query is never logged, so a `?code=` on `GET /` would not reach the log. **The code would still reach the frame inside `RemoteRequest.query`, sent by the browser in the request line before the page has run.**
- Bind (:2936-2990): loopback unless `identity.listen.lan`, which is the "Allow devices on the local network" checkbox (`src/ui/remote.rs:117-127`). §5 says "Require pairing **before** anything but `/info` and `/pair`". The issue's amendment adds the page's static files.

### `src/ui/remote.rs` (dialog and QR)

- `pairing` (:141-227) builds the QR payload as JSON `{server_id, spki_sha256, code, addresses}` (:160-166).
  - `addresses` is `local_address():port` **even when the port is bound to loopback only**, so a URL QR would point a phone at a closed port unless the dialog requires LAN first.
  - "Copy invite" copies the same JSON (:197-207). README :277 documents `switchboard-remote pair "$(pbpaste)"`.
- `qr_texture` (:279-299) encodes any string with `qrcode` 0.14, so a URL payload is a one-line change there.
- `devices` (:230-276) lists name, scope combo, paired, seen and Revoke. It shows no client kind.
- The redaction sentence is at :73-80. Decision 1's sentence on certificate trust would sit beside it.

### `src/core/remote.rs`

All pairing and device transitions are here (:21-103). Nothing in the core knows about HTTP. The scope is taken from the window (:166).

### Native text path that must keep working

- `client/Sources/SwitchboardProtocol/Info.swift:89-117`: `PairingInvite` is `Decodable` JSON. `parse(_:)` is a plain JSON decode, so it **does not accept a URL today**.
- `client/Sources/switchboard-remote/main.swift:107-127`: `pair '<json>'` or `pair <host:port> <code> <spki>`.
- The headset:
  - The paste field is `inviteText` at `client/Sources/SwitchboardUI/BrowserView.swift:792`.
  - The `SWITCHBOARD_INVITE` dev aid is at `client/Apps/SwitchboardVision/SwitchboardVisionApp.swift:56-60`, and `bootOnce` (`BrowserView.swift:103-107`) parses invites line by line.
  - The typed-code path (`ConnectionManager.pair(_:code:)`, `client/Sources/SwitchboardClientCore/ConnectionManager.swift:471-480`) needs no invite.
- The fixture test `client/Tests/SwitchboardProtocolTests/FixturesTests.swift:159-164` decodes the JSON invite. Several client-core tests build `PairingInvite` directly; they are unaffected by the parser change.

### Tests

- `tests/remote.rs`:
  - The `http` helper (:245-296) sends a bearer header and parses only the status and `Content-Length`. **It cannot see response headers**, so header assertions (CSP, type, cache, `Set-Cookie`) need it extended.
  - `call` (:299-325) pumps `serve_pending` while a client thread waits. That is the shape for an end-to-end pairing test with the host's own HTTP client.
  - The client config is pinned (`pinned_client`), so a "browser-like" unpinned client is not available as a helper.
  - Other relevant tests: `info_is_served_to_anyone_and_every_other_read_is_refused_unpaired` (:426), `the_allowlist_refuses_everything_else_with_no_side_effect` (:682), `pair_is_rate_limited_per_listener` (:1406), `revoke_and_a_scope_change_take_effect_on_the_next_request` (:658), and `two_instances_have_distinct_identities…` (:1464).
- Fixtures: `write_fixtures` (:1587) writes to `client/Fixtures/*.json` (info, pair, session, sessions, waiting, last_response, screen, …), and `the_client_fixtures_decode_on_both_sides` (:1829) checks them.

## What the page would read, from what the wire has today

- **Sessions grouped by project:** `GET projects`, then `GET projects/{id}/sessions` per project, plus `GET waiting`. Each is whole and enveloped.
- **The card fields:** `SessionView` (`control/src/lib.rs:637-688`) has `kind`, `liveness`, `card` (the card's word), `waiting`, `waiting_reason`, `trust_question` and `last_activity_ms`.
- **The one-line excerpt the card shows is not on `SessionView`.**
  - The Mac's agent card excerpts the transcript (`src/ui/cards.rs:176-178`, `agent_excerpt` at :558). A shell or service card excerpts its pane.
  - §4.3 says the excerpt is "`last_message` from the `Stop` hook … a field of the session's metadata", but no such field exists on the wire view.
  - The headset fetches `last-response` per session to fill its list (`client/Sources/SwitchboardUI/BrowserView.swift:149-163`).
  - The page would do the same, one content-worker read per Claude Code session, each entering the 128-key watch set, or the plan adds a field. This is the one place the issue's "no new read model unless the page cannot do without one" bites.
- **The session view:** `GET sessions/{id}/last-response` for Claude Code, and `GET sessions/{id}/screen?lines=n` otherwise. `changed {conversation}` / `{screen}` arrive after a `watch {kind, id}` text frame, answered with `watching`.
- **Reply:** `POST sessions/{id}/send` with `{op, text}` under `control` (`remote_command`, `src/app.rs:3159-…`). The op is a client-made UUID, and a timeout is followed by `GET ops/{op}` (§4.5).
- **Header:** `server_name` comes from `/info`. **The device's own scope** is in the `/pair` reply. I did not find an endpoint that tells an already-paired device its current scope after a change in the dialog. A read-scope device learns of a raise only from a `send` refused 403 (`src/app.rs:3171` text) or by succeeding. Whether `/info` or any authenticated reply carries the caller's scope is listed below as [tree].

## Things a plan has to account for that the tree already decides

- The first navigation `GET /?code=…&key=…&id=…` puts the code in the **request line**, so whoever terminates TLS at that address sees it before the page's script compares keys. The URL also carries `key`, so a terminator that received both could post `/pair` to the real host with the right key.
  - Under click-through trust, an active impostor serves its own script anyway, as the issue says. So the key-in-URL check defends only against an honest wrong host, for example a second Switchboard instance on another port or a stale address.
  - Whether the parameters ride in the query or the fragment is the planner's choice. The difference between the two in what the server receives is listed as a [source] question.
- Two instances on one Mac (supported; the test at :1464) share an IP. Cookies are per host and not per port, and a second instance has its own token for a different `server_id`. A cookie name not keyed by `server_id` would collide between them. `localStorage` is per origin (port included).
- `/pair` takes the device name from the body. The host would derive a name from the User-Agent only if `read_request` keeps that header.
- The pre-pairing surface (`/info` and `/pair` plus static files) and the rate windows live in `Limits` and `serve_remote`. The 4-slot unauthenticated pool is shared by everything a browser does before it authenticates.
- `SWITCHBOARD_SCRIPT` already has `pairing <code>|close`, `remote-devices` and `remote-listen lan|loopback <port>` (README :318-319), so the live acceptance can be staged without clicking.

## Open questions

- [source] Does Safari on iOS/iPadOS let the owner click through a self-signed certificate that has no subjectAltName, a validity past 825 days and no serverAuth EKU (Apple's "Requirements for trusted certificates in iOS 13 and macOS 10.15")? Or does the connection fail outright, as `make_identity` builds the certificate today?
- [source] Is Chrome's error for a certificate without a SAN (`NET::ERR_CERT_COMMON_NAME_INVALID`) bypassable from the interstitial on Android and desktop? Does Firefox accept the same certificate after "Accept the Risk"?
- [source] How do Safari, Chrome and Firefox scope a certificate exception: per host, per host and port, or per certificate? After a key rotation, does the browser show its own interstitial again before any page script runs, and how long does the exception last (session or permanent)?
- [source] After a click-through, does a WebSocket `wss://` to the same host and port succeed in Safari on iOS, or does it fail even though the page loaded?
- [source] Is a page loaded over https after a certificate-error click-through a secure context (`window.isSecureContext`, `crypto.randomUUID`) in Safari and Chrome?
- [source] To trust the certificate ahead of time on iOS, is a downloaded `.cer`/`.mobileconfig` profile installed and then "Enable full trust" turned on in Certificate Trust Settings? Does Android Chrome honour a user-installed CA or leaf for a website?
- [source] Cookies on an IP-address host: does a `Secure; HttpOnly; SameSite=Strict` cookie set by `https://<ip>:43187` get sent to other ports on the same IP (RFC 6265 cookies ignore port)? Does SameSite treat ports of one IP as same-site?
- [source] Do browsers attach cookies to the WebSocket opening handshake, and is SameSite applied to it (cross-site WebSocket hijacking)? What `Origin` header does the browser send?
- [source] If the token travels as a `Sec-WebSocket-Protocol` value, must the server echo one offered value in the 101 for the browser to accept the connection? Are base64url characters valid there (the RFC 6455 subprotocol token grammar)?
- [source] When a WebSocket upgrade is answered 401, does the browser's WebSocket API expose that status to the page, or only `onerror` and a 1006 close? The page must still tell "revoked" apart.
- [source] Is a URL fragment (`#…`) sent to the server in the request line or the `Referer`, when a query string is? Does the iOS Camera's QR banner keep the fragment when it opens the URL in Safari?
- [source] Does `history.replaceState` keep the original URL with the code out of Safari's History, iCloud-synced history and tabs, and Chrome's history? Does the iOS Camera or Google Lens keep scanned URLs in a history of its own?
- [source] Does the Android camera or Google Lens open a scanned URL in an in-app browser with its own cookie jar, so that a token set there is missing when the page is later opened in Chrome?
- [source] How many parallel connections do Safari and Chrome open to one host and port on a first page load with a stylesheet, a script and fetches? Do they preconnect speculatively, given the host's 4-slot unauthenticated pool and 5 s first-line deadline?
- [source] Does iPadOS Safari send a macOS User-Agent by default ("Request Desktop Website"), so that a name like "Safari on iPad" cannot be derived from the header?
- [source] Do Safari on iOS and Chrome on Android resolve a `<host>.local` name typed into or opened from a URL? This matters if the plan uses the Bonjour or host name instead of the IP.
- [source] Does iOS show a Local Network permission prompt for Safari when it reaches a private IP? Does any browser block https requests to private addresses (Private Network Access) when the page is itself on that address?
- [tree] Does any authenticated reply or `/info` tell a paired device its current scope after the owner changes it in the dialog? The page needs this to show or hide the reply box. I only read `remote_info` and `remote_pair`.
- [tree] Where in the visionOS app does the pasted `inviteText` (`BrowserView.swift:792`) get parsed, and does anything besides `PairingInvite.parse` read the QR's JSON (the README, a fixture, the script, Dispatch)?
- [tree] Does `local_address()` pick the right interface on a Mac with VPN or several interfaces? Does the dialog show the LAN address while the port is bound to loopback? I read only that the QR uses it unconditionally (`src/ui/remote.rs:157-159`).
- [run] How long does a page load take that reads `last-response` for every Claude Code session through the one content worker, on the owner's real data directory? How many keys does it push into the 128-key watch set?
- [run] Does a browser's first page load (HTML, CSS, JS, `/info`) against the current listener lose a connection to the 4-slot unauthenticated pool?
- [owner] On the owner's iPhone and iPad: does the camera open the pairing URL in Safari, does the certificate warning offer a click-through for this certificate, and does the events socket connect afterwards?
- [owner] Which browsers and devices does the owner actually mean to support (iPhone Safari, iPad Safari, another Mac's Safari or Chrome, Android)? The certificate and cookie answers differ per browser.
