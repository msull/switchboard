# Issue #230 — a web control surface on the remote port: investigation notes

Read: `CLAUDE.md`, `docs/design.md` ("Durable store", "Trust boundary", "Remote client (2026-10-12)" status), `docs/lessons.md`, `docs/remote-client.md` §0–§6. Line numbers are as they stand in this worktree (branch `dispatch/230-…`, at `7fe8949`).

## The record and rehydration first

**On the host, nothing new has to be durable.** A browser is a `PairedDevice` like any other:

- `devices.json` already holds `{id, name, scope, token_sha256, created, last_seen}` (`src/ports/remote.rs:95-107`). The hash is the only copy of the token the host keeps. The name is whatever the device sent as `device_name`, cut to 64 chars (`src/core/remote.rs:166-175`). "Safari on iPhone" fits that field without changes. If the plan wants to mark a device as a browser (to word the dialog or the revoke), that is an additive field with `#[serde(default)]`. These types live in `ports/remote.rs`, not `core/model.rs`, so the `SCHEMA_VERSION`/`migrate` rule doesn't apply. `identity.json` has its own `IDENTITY_SCHEMA_VERSION = 1` (`src/ports/remote.rs:17`), but `devices.json` has no version field at all.
- `identity.json` holds the certificate DER (base64) and the SPKI hash (`src/ports/remote.rs:26-42`). The certificate is made by `make_identity` (`src/adapters/remote.rs:95-127`) with **no subjectAltName** (`CertificateParams::new(Vec::<String>::new())`), CN `Switchboard <server_id>`, and rcgen's default `not_after` (the comment says the year 4096). Native clients pin the SPKI, so a certificate reissued over the **same key** with SANs or a shorter validity keeps every pin. That means a rewrite of `identity.json` through the atomic path, with no key rotation and no `devices.json` wipe. Whether a browser needs that reissue is an open question below.
- The pairing window is in memory only: `PairingWindow {until, code, scope, failures}` (`src/core/action.rs:972-978`, `src/core/remote.rs:46-64`). It is never saved. A success consumes it (`Paired`, `src/core/remote.rs:65-71`).
- Loading at startup is unchanged: `load_identity` (`src/app.rs:2782-2850`) reads both files and dispatches `IdentityLoaded`. A browser's token is checked the same way as any other: `device_by_token(sha256)` (`src/core/remote.rs:145-147`) in `authenticate` (`src/app.rs:3127-3156`).

**The page is a cache too, and its whole state rebuilds from the host.** Per load, its only state is a token (in a cookie or in script storage, whichever decision 2 picks) and the key hash it paired against, which it keeps for its own "identity changed" check. Each load rebuilds the rest: `GET /api/v1/info` → compare `spki_sha256` with the stored key → open `/api/v1/events` and wait for `hello` → fetch collections and the open session at their revisions → apply the buffered events by revision (§4.4 "Subscribe first, then snapshot"). Spec §1 and A2 say "the client keeps content in memory only" (F5), and the page would follow the same rule. The page itself is compiled into the binary, so a new app version is a new page (see the caching question).

## What exists, and where it stops for a browser

### Transport: `src/adapters/remote.rs`

- `RemoteRequest` (`:359-372`) carries only `method, path, query, body, bearer, upgrade`. `read_request` (`:444-516`) lowercases every header but keeps only `content-length`, `authorization: Bearer`, `upgrade` + `sec-websocket-key`. **No `Cookie`, `Origin`, `Host`, `User-Agent`, `Sec-WebSocket-Protocol` or `Accept` reaches the frame.** A cookie- or subprotocol-borne token, and any `Origin` check on the WebSocket, need new fields here.
- The query is split but **not percent-decoded** (`:496-504`, doc "ids and numbers only"). A browser that loads `/?code=…&key=…&id=…` sends that query to the host on the first `GET /`. The host has no use for it. As long as nothing logs `request.query` it stays out of the log, and `refuse` logs only method and path (`src/app.rs:3762-3766`).
- `RemoteReply` (`:389-393`) is `{status, body, authenticated}`. `respond` (`:530-552`) always writes `Content-Type: application/json`, `Content-Length` and `Connection: keep-alive`, with **no other header** and a fixed reason table (`:531-543`, no 304). Serving HTML/CSS/JS, a CSP, `Cache-Control`, `X-Content-Type-Options` or `Set-Cookie` needs reply headers, which is an additive field on `RemoteReply` or a second reply type.
- `respond_upgrade` (`:519-527`) writes the 101 with no `Sec-WebSocket-Protocol`. Per RFC 6455, a browser that offered a subprotocol and gets none back fails the connection (open question). `subscriber` (`:878-927`) is unchanged by this.
- Pools: `MAX_UNAUTHENTICATED = 4`, `MAX_PAIRED = 12` (`:567-568`). A new connection always starts in the unauthenticated pool. The accept loop **drops** the 5th stranger at once (`:701-707`). A connection moves to the paired pool on its first reply with `authenticated: true` (`:839-853`). Deadlines: handshake and first request line within 5 s, idle 30 s (`:570-574`). A browser that has not paired opens several parallel connections for `/` plus its subresources, plus a speculative preconnect and `favicon.ico`. Each of these takes one of the four slots and holds it until the 30 s idle timeout.
- Rate windows: `Limits::limited` matches only `PAIR_PATH` and `INFO_PATH` exactly (`:586-587`, `:623-640`), so static paths are under no window today.
- `local_address` (`:962-971`) picks the one IPv4 address the system routes out on. `bind_remote` binds `0.0.0.0` or `127.0.0.1` only (`src/app.rs:2962-2969`), so IPv6 is not reached today.

### Routing: `src/app.rs`

- `serve_pending` (`:1572-1606`) drains 8 remote requests per pass into `serve_remote`.
- `serve_remote` (`:3017-3122`): anything without the `/api/v1/` prefix gets `404 not_found` (`:3023-3025`). `info` and `pair` come before `authenticate` (`:3027-3031`). Everything after needs a bearer (`:3032-3035`), including `GET events` (`:3036-3038`). Unknown paths are refused by name (`:3108-3116`). §5 says "Require pairing before anything but `/info` and `/pair`" (`docs/remote-client.md:306`). The page's static paths would be the third exception, and the spec line changes with them.
- `authenticate` (`:3127-3156`) answers **the same `401 unauthorized`** for no token, an unknown token and a revoked token. Only the message differs ("pair this device first" vs "the token is not known; pair again"). The page's "revoked" state can only key on that message or on a new code. A wiped `devices.json` after key rotation looks the same, but there the page also sees a different key at `/info` (and the browser a new certificate).
- `remote_info` (`:3568-3602`) is public and includes `spki_sha256`, `server_name`, `capabilities` and `pairing_open`. It does not include the device's scope, because scope is known only after authentication. The page header "the device's scope and host name" therefore needs scope from the `/pair` reply (`scope` is in it, `:3659-3664`), which goes stale after a scope change in the dialog (`SetDeviceScope`, `src/core/remote.rs:93-100`). No authenticated endpoint today answers "who am I", so the scope has to come from somewhere else. The plan has to say where.
- `remote_pair` (`:3606-3673`) wants a JSON body `{spki_sha256, code, device_name}`, upper-cases the code, and counts a wrong key or a wrong code against the window (`PairingFailed`). It answers `{token, scope, server_id, device_id}` in the envelope with `authenticated: false`. That last part means the pairing connection itself stays in the unauthenticated pool until its next request.
- Revocation drops the device's subscribers in `dispatch_inner` (`:486-497`), and dropping the queue closes the socket (`subscriber`'s `Disconnected` arm). The page sees a WebSocket close, and its next REST call gets 401.
- What the page needs is all served today: `waiting` (`:3054`), `projects/{id}/sessions` (`:3048-3050`), `sessions/{id}` (`:3051-3053`), `sessions/{id}/last-response` and `screen` through `remote_content_route` (`:1947-`, `:3099-3107`, last-response mapped at `:1967`), and `POST sessions/{id}/send` (`:3060-3062`, `remote_command`, which needs `control` scope and an `op`). `SessionView.card` is the card's word (`control/src/lib.rs:645-646`). The one-line excerpt is `last_message` per spec §4.3 (`docs/remote-client.md:207`); I did not open the struct past line 690 to confirm that field's name on the wire. Screen and conversation updates need a `watch {kind,id}` frame on the socket (§4.4 watch frames), which the page would send the same way the Swift client does.

### Pairing window: `src/ui/remote.rs`

- The QR payload is built **in the UI** (`:155-166`): JSON `{server_id, spki_sha256, code, addresses:["ip:port"]}`. The address is `local_address()` with the bound port, and is empty with no network. A URL in the QR, built from the same parts, is a pure function. By CLAUDE.md's rule ("If logic appears under `ui/`, move it to the core") it belongs in the core or `adapters::remote`, where `tests/remote.rs` and the script can reach it.
- "Copy invite" copies the same text (`:198-206`). `qr_texture` (`:280-299`) uses the `qrcode` crate at 4 px per module. A URL of about 130 characters (64 hex digits of key, a 36-character UUID, 8 code, address) is a denser QR than today's JSON only by the `https://…/?` framing.
- The window text says "Scan, or type the key and the code" (`:191-197`). The dialog has no sentence about a browser, a certificate warning, or what the click-through does and does not prove. Decision 1 adds that sentence.
- The script's `pairing <code>` always opens with `Scope::Read` (`src/script.rs:730-733`). Driving a control-scope browser pairing in the live app needs a scope argument.

### Native text path: `client/`

- `PairingInvite.parse` decodes JSON only (`client/Sources/SwitchboardProtocol/Info.swift:94-116`).
- Callers: `switchboard-remote pair '<invite json>'` (`client/Sources/switchboard-remote/main.swift:12`, `:107-125`), the headset's paste field (`client/Sources/SwitchboardUI/BrowserView.swift:849`), and `SWITCHBOARD_INVITE` at boot, one per line (`BrowserView.swift:103-108`; `client/README.md:180-190`).
- Test: `testInviteParsesTheQRText` (`client/Tests/SwitchboardProtocolTests/FixturesTests.swift:157-165`).
- Accepting the URL means adding a URL branch to `parse` (query `code`, `key`, `id`; host:port → `addresses`), plus a fixture case. Swift tests are not in the pre-commit hook or CI (design.md A2), so this branch is checked only by a hand run.

### Tests: `tests/remote.rs`

- `http` (`:245-296`) is a raw rustls client pinned to the key. It writes only `Authorization` and `Content-Length`, reads only `Content-Length`, and `call` parses the body as JSON (`:299-325`). Tests for static paths, headers, `Set-Cookie`/`Cookie` or a subprotocol need a variant that sends extra request headers and returns response headers and raw bytes.
- `the_allowlist_refuses_everything_else_with_no_side_effect` (`:682-724`) covers refused paths. Today none of them is outside `/api/v1/`, so the new static set needs its own negative cases (`/index.html` vs `/`, `/..`, unknown files).
- The fixtures `client/Fixtures` are written by `write_fixtures` (`:1587-`) and checked on both sides (`:1829`). They change only if a view changes.
- No `web/` directory exists. The tree already embeds assets: `include_bytes!` for fonts (`src/ui/theme.rs:203-211`) and the icon (`src/main.rs:167`). Nothing embeds text with `include_str!` yet.

### Docs to amend

- `docs/remote-client.md` §4.1 (bounds, pools, rate windows), §4.3 (the endpoint table, now with non-API paths), §5 (`:306` "before anything but `/info` and `/pair`"; `:312` token "sent only as a bearer header … never in a URL"; `:309` QR payload).
- `docs/design.md` "Trust boundary" (`:219-266`, which says how a secret's surroundings leave the app) and the Remote client status section.
- README Layout (`:369-437`) for `web/` and any new file, and the dev-aid list if the script gains a scope.
- `CHANGELOG.md` Unreleased.

## Things the tree shows the plan must not miss

- The issue's acceptance step "revoke the phone … the page says so on its next request" depends on telling revoked from never-paired. Today both are the same 401 (`src/app.rs:3127-3150`).
- The issue's acceptance step "a session that starts waiting moves to the top without a reload" is served by `changed {waiting}` / `changed {session}` from the diff pass (design.md A3). Nothing new is needed on the host.
- If the token moves to a cookie, a cross-site page on the LAN can make the browser open `wss://host:port/api/v1/events` and send the cookie. `SameSite=Strict` governs that, and so would an `Origin` check, but `Origin` is not read today (`read_request`). CSRF on the `POST` commands is the same exposure: `send` takes JSON, and today's handler does not look at `Content-Type`.
- A pairing code in a URL can reach places before the page's `replaceState` runs: the camera app's link handoff, the browser's history for the interstitial page, and the first `GET /` itself (sent over TLS, so only to the host).

## Open questions

- [source] Can Safari on iOS and iPadOS (and Chrome on Android) click through a self-signed certificate with **no subjectAltName** on an IP-address URL, or does `NET::ERR_CERT_COMMON_NAME_INVALID` / Safari's "not private" page refuse it with no "visit this website" option?
- [source] Does Safari or WebKit accept a certificate whose `notAfter` is the year 4096 (validity far over 825 or 398 days), either on click-through or as a manually trusted certificate? iOS 13+ "Requirements for trusted certificates" and Chrome's max validity rules.
- [source] After a click-through exception in Safari on iOS, does `new WebSocket("wss://…")` to the same host:port succeed, or does WebKit refuse a wss connection to a host whose certificate was only excepted (known WebKit self-signed websocket failures)? Same for Chrome and Firefox.
- [source] On iOS, can a self-signed leaf certificate (not a CA, `basicConstraints` absent) be installed as a profile and enabled under Settings › General › About › Certificate Trust Settings, or does full trust apply only to root CA certificates? What `extendedKeyUsage` and SAN does iOS require for a manually trusted server cert?
- [source] How long does Safari on iOS keep a certificate exception: per tab session, until the app is quit, or persisted? Does a phone that slept and returned require the warning again, and does the exception survive the page's own reload?
- [source] When a browser offers `Sec-WebSocket-Protocol` and the server's 101 echoes no protocol, the browser fails the connection (RFC 6455 §4.1). Is the subprotocol value logged or exposed anywhere a header would not be (Web Inspector, proxies)?
- [source] Do Safari on iOS and Chrome send cookies with `SameSite=Strict; Secure; HttpOnly` set over a click-through (untrusted) TLS origin, and on the WebSocket upgrade to that same origin? Is a cookie set on an IP-address host (no `Domain`) kept across restarts?
- [source] Does iOS's Camera app open a scanned `https://<ip>:<port>/?…` URL directly in Safari, or show a banner first? Does it add the full URL with the code to Safari history or to the camera's own record before the page's `history.replaceState` runs? Is the interstitial (warning) page recorded in history with the query?
- [source] How many parallel TLS connections do Safari on iOS and Chrome open for a first page load of one HTML, one CSS and one JS file plus a `favicon.ico` probe? Do they preconnect speculatively? That decides whether four unauthenticated slots (`src/adapters/remote.rs:567`) suffice.
- [source] Does `Content-Security-Policy: default-src 'self'` allow `wss:` to the same origin in Safari, or does Safari need `connect-src 'self' wss://<host>:<port>` named explicitly (the WebKit `'self'`-excludes-ws bug)?
- [tree] What exactly is the excerpt field on the wire `SessionView` (`control/src/lib.rs:637-690`), and is it present on the `waiting` and `sessions` collections the page lists?
- [tree] Does `remote_content_route` (`src/app.rs:1947-2046`) answer `last-response` for a Codex, shell or command session with a "no conversation" reason the page can switch on, and under which code?
- [tree] Does any other code path log `request.query` or the full target (the `log::debug!("remote connection: {e}")` at `src/adapters/remote.rs:724` gets an error, not the request; I did not read every `log::` in the remote routes)?
- [run] With the current listener, does a real browser's first visit (a warning, then a click-through, then the page and its files) stay within the 4-connection unauthenticated pool and the 5 s handshake deadline, or does the 5th connection get dropped and the page load half-way?
- [run] Does the rcgen-made certificate as it stands (no SAN, CN only, P-256) complete a TLS handshake with Safari, Chrome and Firefox at all, whatever the warning says (rustls default cipher suites and ALPN with no `h2` offered)?
- [owner] On the owner's own iPhone and iPad: scan the QR, accept the warning, and see whether the page loads, the events socket stays open and a reply arrives. That is the issue's acceptance run, and only the owner's devices and network can show it.
- [owner] Does the owner's local network (the router or a mesh) isolate clients, or block port 43187 between the phone and the Mac? This needs the owner's network.
