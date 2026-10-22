# Issue #230 investigation notes: a web control surface on the remote port

Worktree: `/Users/sbgragent/.dispatch/worktrees/f53f3e3e` (branch `dispatch/230-a-web-control-surface-the-remote-port-se`, HEAD `7fe8949`). Tree is clean; nothing was edited, built or tested. No `web/` directory exists.

Read: `CLAUDE.md` (given), `docs/remote-client.md` (all 553 lines), `docs/lessons.md`, `docs/design.md` (trust boundary at 219–260, remote status at 3107–3270 and 3480–3495), and the code listed below by line.

---

## 1. Where the issue touches the code

**Listener, connection, pools, HTTP parsing, responses: `src/adapters/remote.rs`**

- `RemoteReply` (`{status, body, authenticated}`) at 389. It has no header field, so a `Set-Cookie` cannot ride on a reply today.
- `RemoteRequest` at 360: `method, path, query, body, bearer, upgrade`.
- `read_request` at 444–516 keeps only `Authorization` (`bearer`, 487) and `Upgrade` plus `Sec-WebSocket-Key` (`upgrade`, 491). It drops `Cookie`, `Origin`, `Sec-WebSocket-Protocol`, `Sec-Fetch-*` and `Content-Type`.
- `respond_upgrade` at 519–527 writes a 101 with `Sec-WebSocket-Accept` only. It echoes no subprotocol.
- `respond` at 530–553 hard-codes `Content-Type: application/json` (545). Every response, so every static file, would carry the wrong type unless this changes. It writes no cache or security headers.
- `MAX_UNAUTHENTICATED = 4` (567), `MAX_PAIRED = 12` (568), `HANDSHAKE_TIMEOUT` 5 s (570), `IDLE_TIMEOUT` 30 s (573), set after the first request at 785. `PAIR_PER_MINUTE = 6` (583); `INFO_PER_MINUTE = 60` in the same block.
- Pool accounting: at accept, a TCP connection is counted as unauthenticated (`fetch_add` at 707) and dropped at once when the pool is full (check at 701, `drop(stream)` at 704). The count moves to the paired pool only when a reply comes back `authenticated: true` (846 area). A connection that serves only static files never sets that flag, so it keeps its unauthenticated slot until the idle timeout or the client closes.
- `Limits::limited` at 631 rate-limits only `/api/v1/pair` and `/api/v1/info`. Nothing else is rate-limited.
- `Pin::verify_server_cert` at 188–205 compares only the SPKI hash and ignores the server name. The native pin does not depend on the certificate's names.
- `make_identity` at 95–127. `CertificateParams::new(Vec::<String>::new())` at 102 gives **no subject alternative names**, only a CN of `Switchboard <server_id>`.
- `local_address()` at 962–970 returns one LAN IPv4 address (UDP connect trick).
- Store: `load_devices`/`save_devices` at 80/83; identity at 74/77.

**Frame routing, auth, pairing handlers: `src/app.rs`**

- `serve_pending` at 1572 drains `DRAIN_PER_PASS` (8) requests per frame pass (1587).
- `serve_remote` at 3017. Any path not starting with `/api/v1/` gets a 404 through the frame (3023–3024). `GET /info` and `POST /pair` are matched first (3027–3029). The allowlist is the match from about 3030 to 3110, with the fallthrough 404 "not available on this port" at 3113.
- `GET /api/v1/sessions/{id}/screen | conversation | last-response` and `/runs/{n}/output` and `/projects/{id}/files/...` route through `remote_content_route` (3101, 1947).
- `GET /api/v1/events` goes to `remote_subscribe` at 2723; it needs an upgrade and a bearer device.
- `authenticate` at 3127–3150. No bearer gives 401 "pair this device first". An unknown token gives 401 "the token is not known; pair again" (3146). **A revoked token and a token from before a key rotation get the same answer.** A successful call dispatches `DeviceSeen` (3150).
- `remote_info` at 3568–3603. It returns `server_id`, `server_name`, `protocol`, `version`, `spki_sha256`, `limits`, `pairing_open` (3596) and a `capabilities` list (3580).
- `remote_pair` at 3606–3670. Checks in order: window open (403 `pairing_closed`), body is JSON, `spki_sha256` equals the host's own (`wrong_key`, 3641), `code` equals the window's (`wrong_code`, 3648). `device_name` is read from the body verbatim (3650; default "Device"). Reply: `{token, scope, server_id, device_id}`, wrapped in the envelope, with `scope` at 3662.
- `remote_command` at 3159. The scope check (`device.scope != Scope::Control`, 3166) runs before the body is parsed. The body is `{op, text}` for `send`.
- `refuse` at 3754 logs method and path only, never the query string. A `code` in a query string does not reach that log through `refuse`.

**Pairing window and QR: `src/ui/remote.rs`, `src/core/remote.rs`**

- `pairing()` at `src/ui/remote.rs:141`. The QR payload is the JSON `{server_id, spki_sha256, code, addresses}` at 160–166, with the address from `local_address` at 157. "Copy invite" copies the same JSON (201–205). The QR is drawn by `qr_texture` at 280.
- `src/core/remote.rs`: `PairingFailed` at 56, `Paired` at 65 (closes the window, pushes the device, saves), `RevokeDevice` at 86, `save_devices` at 119, `pairing_at` at 152, `new_device` at 166–176 (scope from the window, else `Read`; name trimmed to 64 characters).

**Device record: `src/ports/remote.rs:96`** `PairedDevice {id, name, scope, token_sha256, created, last_seen}`.

**Text pairing paths that must keep working**

- `client/Sources/SwitchboardProtocol/Info.swift:94–116`: `PairingInvite` decodes the QR JSON only. `parse` (114–116) is `Wire.decode` of the JSON.
- `client/Sources/switchboard-remote/main.swift:109–121`: one argument goes to `PairingInvite.parse` (110–111); three arguments (address, code, key) take the other branch.
- `client/Sources/SwitchboardUI/BrowserView.swift:883`: the headset's paste field calls `PairingInvite.parse`. `BrowserView.swift:106` is the `SWITCHBOARD_INVITE` dev aid.

**Read models the page would use: `control/src/lib.rs`**

- `SessionView` at 637. Fields the page needs exist: `kind` (641), `notes` (643), `card` (646, "the card's word"), `waiting` (657), `waiting_reason` (658), `trust_question`, `resume_id`, `liveness`, `last_activity_ms`.
- **There is no excerpt field.** The card excerpt is computed in the UI: `src/ui/cards.rs:176–180`, where agent cards use `agent_excerpt(conversation)` from the UI's conversation cache and other kinds use `cx.state.captions` (UI state) or the host status title.
- `last_message` exists only on hook events (`src/ports/events.rs`, `src/adapters/hooks.rs`), not on `SessionRecord` (`src/core/model.rs` has no match). The spec (`docs/remote-client.md` §4.3, §4.4) says the excerpt is "`last_message` … a field of the session's metadata". **That does not hold in this tree.**

**Tests: `tests/remote.rs`**

- `http()` at 245–297 returns `(status, body String)`. It reads headers only to find `Content-Length`, and returns no headers.
- `call()` at 299–325 parses the body as JSON and panics otherwise (323). Any static page or non-JSON response fails through this helper.
- `pair()` at 406–424 drives `/api/v1/pair` and reads `body.token` from the envelope.
- Fixture writer `write_fixtures` at 1587; `the_client_fixtures_decode_on_both_sides` at 1829 decodes them on the Rust side. The Swift side reads them in `client/Tests/SwitchboardProtocolTests/FixturesTests.swift`. A JavaScript page would be a fourth speaker and is not covered by either.

**Other touch points**

- `CHANGELOG.md:9` has an empty `## [Unreleased]`. CLAUDE.md requires a line for any user-visible change.
- `spikes/22-remote-port` exists (README, `measure.sh`). CLAUDE.md requires a new spike directory for a new risky mechanism before building on it. The cookie, the subprotocol and the certificate trust are candidates.
- `docs/remote-client.md` §4.1, §4.3 and §5 and the remote status in `docs/design.md` are what the plan must amend.

---

## 2. On-disk record and rehydration, then what the pane shows

### Records that exist today

- **`identity.json`** (`src/adapters/remote.rs` store, `save_identity` at 77): `server_id`, `server_name`, `certificate` (DER, base64), `spki_sha256`, `created`, `listen` (LAN flag, port). The private key is in the Keychain under `remote-key:<server_id>`.
- **`devices.json`** (`save_devices` at 83, core save at `src/core/remote.rs:119`): one `PairedDevice` per paired device. The record holds `id`, `name`, `scope`, `token_sha256` (never the token), `created`, `last_seen`. `last_seen` is saved only every `LAST_SEEN_SAVE_INTERVAL_SECS` (the `DeviceSeen` arm, `src/core/remote.rs` around 75–85).
- **Pairing window**: in memory only, in core state (`PairingWindow {until, code, scope, failures}`). Neither the code nor the window is ever persisted.
- **Sessions, projects, sets**: unchanged, and the source of everything the page shows.

### What a browser device adds

- **The device record is the same `PairedDevice`.** The name comes from the body's `device_name` (`src/app.rs:3650`), trimmed to 64 characters (`src/core/remote.rs:170`), so a page can send "Safari on iPhone" built from the user agent. Nothing in `src/core/model.rs` changes, so this record appears to need no `SCHEMA_VERSION` bump. A closer read of the store's load and save (`src/adapters/remote.rs:51–90`) would confirm it (see open questions).
- **Browser-side state**: the bearer token (cookie, or a script-readable store, depending on decision 2); the SPKI the page pinned (needed to tell "identity changed" apart from "revoked", which the token cannot do); and per-key held revisions in memory.
- **Rehydration.** `devices.json` loads at startup (`src/app.rs:2791`), so an existing token keeps working across a Mac restart. A key rotation clears `devices.json` (`src/app.rs:2809–2817`), so an old token gets 401 "the token is not known; pair again". Revoking removes the record the same way, so the host cannot say "revoked" as distinct from "never paired" or "rotated" (`src/app.rs:3146`).
- **Static files**: no record. They would be compiled in with `include_str!` / `include_bytes!`. The repo already does this for fonts and the icon (`src/ui/theme.rs:203–211`, `src/main.rs:167`).

### What the pane shows, and where each datum comes from

- **Header**: host name from `GET /api/v1/info` (`server_name`). **The device's own scope and name have no read route.** Scope appears only in the `/pair` reply (`src/app.rs:3662`). Until a route exists or the page stores scope, a reload cannot show it. This is a gap the issue assumes away.
- **Sessions grouped by project**: `GET /api/v1/projects`, then `GET /api/v1/projects/{id}/sessions`. Each row is a `SessionView`. Waiting first, from the `waiting` bool (657). Sorting is client-side. `GET /api/v1/waiting` exists too.
- **Card line (one-line excerpt)**: not on the wire for any session (see §1). Options exist in the tree: a Claude Code session can use `GET /sessions/{id}/last-response` (routed at `src/app.rs:1967`, allowlisted at 3101), and any session can use `screen`. Adding a field to `SessionView` is also possible. The plan must pick one.
- **Session view**: last response (Claude Code) or screen (others), from the content routes. `changed {session | conversation | screen}` events arrive on `GET /api/v1/events` (`remote_subscribe`, `src/app.rs:2723`).
- **Reply box**: `POST /api/v1/sessions/{id}/send` with `{op, text}`. The scope gate (`src/app.rs:3166`) returns 403 "this device reads only" for a read device, so the box can be hidden from `scope`, but the device needs to know its scope (see above). The page must mint `op` (a UUID, unrelated to the server).
- **Trust button**: `session.trust_question` is on the view, and the `trust` route exists. Not in the issue's first cut.
- **Pairing**: `?code=&key=&id=` → `history.replaceState` → `GET /api/v1/info` → compare `spki_sha256` → `POST /api/v1/pair {code, device_name, spki_sha256}` → read `body.token` and `body.scope` from the envelope. The host checks `spki_sha256` and `code` itself (`src/app.rs:3641`, `3648`), so the page's own key comparison is a second check, not the only one.
- **Opened without code or token**: `GET /` with no query and no cookie. `/info` answers unauthenticated, and `pairing_open` (`src/app.rs:3596`) says whether a window is open. A page can say "pair from the Mac's QR" with no secret involved.
- **Protocol drift**: `/info` returns `protocol` and `version` (`src/app.rs:3568–3603`). A cached page from another build can compare against them. This is a usable hook for the "no caching across versions" requirement.

---

## 3. Facts in the tree that bear on the six decisions

**Decision 1: browser trust in the self-signed certificate.** The certificate has no SAN (`src/adapters/remote.rs:102`). A browser checks the name it connects to, and the address in the QR is an IP, so the warning will be a name mismatch as well as an untrusted issuer. The native pin (`188–205`) ignores names, so nothing native breaks. Reissuing the certificate from the same key with an IP SAN would keep the SPKI pin the spec relies on. Nothing in the tree does that reissue today; `identity.json` stores the DER as made (`make_identity` 95–127; `tls_config` at 144).

On the URL's `key`: `remote_pair` already refuses a mismatched `spki_sha256` (`src/app.rs:3641`). The page's comparison of `/info` against the URL's key can only guard a page the genuine host served. Whichever host answers the URL decides which page runs, so the check is advisory, as the issue itself concedes. I did not settle whether it earns its URL bytes.

**Decision 2: token transport.** The cookie route needs three tree changes, none of which exists: a header slot on `RemoteReply` (389), `respond` taking extra headers (530), and `read_request` keeping `Cookie` (444–516). The subprotocol route needs `respond_upgrade` (519–527) to echo one offered protocol and `read_request` to keep `Sec-WebSocket-Protocol`. **Today no request can be forged cross-site with a credential, because the only credential is a bearer header that browsers never attach.** A cookie makes CSRF possible: nothing reads `Origin`, the only parsed headers are those listed in §1, and `POST` bodies are read regardless of `Content-Type` (`read_request` ignores it). `SameSite=Strict` is one layer; an `Origin` check on mutating requests and on the upgrade would be a second, and needs the header to be kept.

**Decision 3: one QR.** The parse sites are in §1. The URL form would need `PairingInvite.parse` to read a URL as well as JSON (`Info.swift:114–116`), the CLI's one-argument branch (`main.swift:110–111`) and the headset's paste (`BrowserView.swift:883`). The URL carries no `addresses` list or `server_id` in the same shape; `id` and the authority would have to map onto `PairingInvite`. The JSON stays in the Rust code only as long as something uses it: `Copy invite` (`src/ui/remote.rs:201–205`).

**Decision 4: static paths and pools.** Static responses go through the frame today as a 404 (`src/app.rs:3023–3024`). Serving them there costs one of the eight drains per pass (`src/app.rs:1587`). Serving them on the connection thread bypasses the frame, but the tree has no such path, and the adapter's rule is thin handlers. The pools are the live risk: a browser page load opens several parallel connections. Each counts as unauthenticated at accept (`src/adapters/remote.rs:701–707`), and a page that loads only static files keeps those slots for up to 30 s (`573`, `785`). The fifth connection, including the `/api/v1/pair` that the same page sends next, is dropped at once. The spec's §4.1 allows four unauthenticated connections, so one page load may already exhaust the pool. Static paths also are not rate-limited (`631`). Plus: `respond` sends no CSP and no cache headers, and there is no place to add them (`530–553`).

**Decision 5: tests.** `http()` and `call()` cannot assert on headers or non-JSON bodies (`tests/remote.rs:245–297`, `299–325`). A page test needs a helper that returns raw bytes and headers. No test checks `Set-Cookie` or `Content-Type`. The fixture test (`1829`) covers Rust and Swift only. No browser runs in any test harness here; `egui_kittest` drives egui, not a DOM. The issue asks for the pairing flow to run through the host's HTTP client "as the CLI does"; the CLI is Swift, and the Rust test client is `http()`.

**Decision 6: device record.** Covered in §2. Revocation today closes the WebSocket (spec §5) and deletes the record. Scope changes take effect on the next request via `SetDeviceScope` (`src/core/remote.rs`, around 96–100).

**Acceptance criteria against the tree**

- "A session that starts waiting moves to the top without a reload": `changed {session}` fires from the diff (spec §4.4), but no test in `tests/remote.rs` covers a browser subscriber.
- "Revoke the phone and the page says so on its next request": the next request gets the same 401 as a never-paired token (`src/app.rs:3146`). The page must use `/info` to tell revocation from rotation, or the host must add a distinct answer, which needs a tombstone. Neither exists.

---

## 4. Other facts a plan needs

- The repo's trust boundary (`docs/design.md:251`) says secrets leave the port only as redacted text. The cookie makes a stolen browser equal to a stolen device token. The paired-devices dialog must say so, as the spec already requires for redaction.
- The `send` scope check runs before the body parse (`src/app.rs:3166`), so a read device's reply box fails safely without the page.
- The existing content routes already redact (spec §4.3 and `design.md`); the page adds no new text exposure.
- `docs/remote-client.md` §5 says "Require pairing before anything but `/info` and `/pair`". The issue's rule that the page itself may also be served before pairing is a spec amendment, and the issue says so.
- `README.md` needs Layout rows for `web/` and its files, and a dev-aid line if any variable is added (the spec's `SWITCHBOARD_REMOTE_PORT` is already documented).

---

## Did not check

- Did not build, did not run `cargo test`, `swift test` or the app. Did not start a server or a browser.
- Did not read the WebSocket subscriber body (`src/adapters/remote.rs` from about 878), `src/ports/remote.rs` beyond the struct, the store's load and save bodies (`src/adapters/remote.rs:51–92`), the Swift transport (`client/Sources/SwitchboardTransport`), or the visionOS pinning delegate.
- Did not read the rcgen, rustls, tungstenite or base64 sources, so every claim about their defaults is unverified.
- Did not check `tests/ui.rs` for any test that pins the QR JSON.
- Did not check whether `last_message` is written to the record by any path other than the hook event; I grepped `src/core/model.rs` only.
- Did not read `docs/design.md` beyond the sections named above.
- Did not check any browser's behaviour; every browser claim in the issue is an open question below.

---

## Open questions

- [source] Does iOS Safari accept a self-signed certificate with no SAN for a bare IP, with a click-through, and does the exception persist for that host and port?
- [source] Is the click-through remembered per certificate, per host, or per session, and does a changed certificate for the same address warn again?
- [source] Does a browser store and send a `Secure` cookie on a page reached through a certificate exception, including on the WebSocket handshake?
- [source] For `SameSite=Strict`, what is "same-site" for an IP host, and does a different port, or an `http:` page on the same IP, count as same-site?
- [source] Which `Origin` and `Sec-Fetch-Site` values do target browsers send on a `fetch` POST and on a WebSocket upgrade, and is `Origin` always present on the upgrade?
- [source] Does a browser require the server to echo one of the offered `Sec-WebSocket-Protocol` values, and does it keep the socket if the echoed name is the token?
- [source] Are `crypto.randomUUID` and `window.isSecureContext` available on the target iOS versions for a page loaded through a certificate exception?
- [source] Does `connect-src 'self'` allow `wss:` to the same host in target browsers, under a CSP with no inline script?
- [source] Does `history.replaceState` remove the code from the address bar and from the back/forward list on iOS Safari, including the entry made by the first request?
- [source] Does the iOS camera open a URL with an IP host and a port in Safari, and does it offer to open an `https:` URL whose certificate is self-signed?
- [source] Does the default validity period rcgen 0.14 gives its certificate fall inside any target browser's maximum for an accepted self-signed certificate?
- [source] Does rustls 0.23 with the `ring` provider negotiate with iOS Safari and Chrome without a cipher refusal?
- [source] Is the issue's statement that a browser cannot set a header on `new WebSocket()` and that a token in a URL is forbidden true for every target browser? The repo's own rule (`docs/remote-client.md` §5) forbids a token in a URL; the browser claim is unverified.
- [source] What user-agent strings do iPhone and iPad Safari and Chrome send (iPad in desktop mode reports macOS), so that "Safari on iPhone" can be built from them?
- [run] How many parallel connections does one page load from iPhone Safari and from Chrome open to the port, and how long do they stay open after their last response?
- [run] Does a page load plus its favicon request fill the unauthenticated pool (`src/adapters/remote.rs:701–707`), and what does a second device see at accept?
- [run] Latency and frame cost of static responses through `serve_pending` at 8 per pass (`src/app.rs:1587`), against answering them on the connection thread.
- [run] Does a static-only unauthenticated connection hold its slot for the full 30 s idle timeout (`src/adapters/remote.rs:573`, `785`)?
- [run] Is the URL form of the QR shorter or longer than the current JSON form, and does it scan at the current module size and the dialog's width?
- [run] Does the certificate and identity persist across a restart, so the warning is shown once per device, not per launch?
- [run] Does `swift test` and the one-argument `switchboard-remote pair` path still pass after `PairingInvite.parse` accepts a URL?
- [tree] Does `tls_config` (`src/adapters/remote.rs:144`) take the stored DER as is, so a reissued certificate from the same key can be swapped in without new key material? Also does anything else read `identity.certificate`?
- [tree] Does `devices.json` carry a version or go through a migrate path (`src/adapters/remote.rs:51–92`)? If not, a browser record needs no store change.
- [tree] Does any route return the device's own scope or name? I found none in `serve_remote` (`src/app.rs:3017–3113`); a closer read of the match arms would confirm it.
- [tree] How many call sites use `respond` and `respond_upgrade` (`src/adapters/remote.rs:519`, `530`)? Changing their signatures touches all of them.
- [tree] Is the Stop hook's `last_message` persisted to the record through any path (the spec says it is on the record)? A closer read of `src/adapters/hooks.rs` and the reconcile path settles it.
- [tree] Can `agent_excerpt` (`src/ui/cards.rs:558`) be computed on the frame from the record and transcript, or does it need the UI's conversation cache, which the wire cannot see?
- [tree] Does any test in `tests/ui.rs` or `tests/remote.rs` pin the QR JSON text (`src/ui/remote.rs:160–166`)?
- [owner] Trust model: the click-through is the trust step, stated in the paired-devices dialog, or the host offers its certificate for download to trust on the device first?
- [owner] Keep the URL's `key` as an advisory check, or drop it and rely on the code and the host's own `spki_sha256` check?
- [owner] Accept a cookie-carried bearer token readable by no script but sent with any same-site request, given the CSRF surface above, or require `Origin` checks and a `Content-Type` rule first?
- [owner] "Revoked" as a distinct answer needs a tombstone in `devices.json` or a page-side inference from `/info`. Which?
- [owner] The device's own scope and name on reload: a new read route, or stored in the browser and refreshed from the `/pair` reply only?
- [owner] Where the "identity changed" check keeps its pinned SPKI: the browser's per-viewer store (fine for a convenience, not for trust) or a cookie?
- [owner] Should unauthenticated static requests count against the unauthenticated pool, or get their own pool and rate window?
- [owner] Static assets answered on the frame (a new arm in `serve_remote`) or on the connection thread (no frame)?
- [owner] Which address the QR carries when the Mac has several LAN interfaces (`local_address` returns one)?
- [owner] Does a read-scope device see the page at all, or only a control-scope device?
- [owner] Is a spike directory required before the cookie, subprotocol and certificate work (CLAUDE.md's rule for new risky mechanisms), and who signs off the spike's results?
- [owner] Acceptance on a real phone: scan, accept the warning, see sessions, reply, revoke. Needs the owner's device and network.
- [owner] Headset paste of the URL form needs a visionOS device or simulator run; the simulator is not checked here.
