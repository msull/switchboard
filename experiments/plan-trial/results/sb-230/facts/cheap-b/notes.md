# Issue #230: a web control surface on the remote port. Investigation notes

Worktree: `/Users/sbgragent/.dispatch/worktrees/b49c34f1`, branch `dispatch/230-a-web-control-surface-the-remote-port-se`. Read: `CLAUDE.md`, `docs/design.md` (trust boundary, remote status section), `docs/lessons.md`, `docs/remote-client.md` (§4.1, §4.2, §4.3, §4.4 in part, §5, §9, §11, §13), `src/adapters/remote.rs`, `src/app.rs` (remote section), `src/core/remote.rs`, `src/ui/remote.rs`, `client/Sources/switchboard-remote/main.swift`, `client/Sources/SwitchboardProtocol/Info.swift`, `tests/remote.rs` (outline and the cited tests). No code was changed and no test was run.

## 1. Where the issue touches the tree

Line numbers are in this worktree.

- **Listener, accept and pools.** `src/adapters/remote.rs`: `bind` 667; accept loop drops a stranger past the pool at about 699–706; `MAX_UNAUTHENTICATED = 4` 567; `MAX_PAIRED = 12` 568; `connection` 770; the frame queue is fed by `requests.try_send` inside `connection`, and a full queue answers 503 at once.
- **Request parsing.** `read_request` 444. It keeps `method`, `path` (query split off), `query`, `body`, `bearer` (from `Authorization: Bearer` only, 487) and `upgrade` (the `Sec-WebSocket-Key`). It keeps no cookie, no `Origin`, no `Sec-WebSocket-Protocol`. `RemoteRequest` is at 360.
- **Responses.** `respond` 530 writes one fixed head: `Content-Type: application/json`, `Content-Length`, `Connection: keep-alive`, and nothing else (545). There is no way to send `text/html`, a CSP, cache headers or `Set-Cookie` today. `respond_upgrade` 519 sends the 101 with no subprotocol echoed.
- **Rate windows.** `PAIR_PER_MINUTE = 6` 583, `INFO_PER_MINUTE = 60` 584. `Limits::limited` 631 applies only to `/api/v1/pair` and `/api/v1/info`; every other path returns `false` there, so a static path is not rate-limited, but it does take a pool slot and a frame drain slot.
- **Routing.** `src/app.rs` `serve_remote` 3017. Line 3023 is the prefix match: `strip_prefix("/api/v1/")`, and anything else is `refuse(404, "not_found")`, a JSON body, after the request has already crossed to the frame. `info` and `pair` are matched at 3029–3031, before `authenticate`. The allowlist of `(method, segments)` pairs is 3031–3125.
- **`/info`.** `remote_info` 3568. Returns `server_id`, `server_name`, `protocol`, `capabilities`, `spki_sha256`, `version`, `limits` and `pairing_open`, inside the envelope.
- **Envelope.** `envelope` 3730 wraps every body as `{"epoch", "revision", "body"}`. The page reads `spki_sha256` from `body`.
- **`/pair`.** `remote_pair` 3606. Order of checks: window open (3610), identity present, body is JSON, then `spki_sha256` compared to the host's key (≈3623, on mismatch `PairingFailed` and `wrong_key`), then `code` compared (`wrong_code`), then `new_device`, then the reply. **The reply body carries `token` as JSON** (`{"token", "scope", "server_id", "device_id"}`). It sets no cookie. `device_name` is trimmed and capped at 64 chars by `new_device`.
- **Bearer check.** `authenticate` 3127. Requires `request.bearer`; otherwise 401 "pair this device first". An unknown token is 401 "the token is not known; pair again" (3146). Revoked and rotated devices both reach that second branch: the record is gone either way.
- **Events.** `remote_subscribe` 2723 requires `request.upgrade` and an authenticated device (through `authenticate`, so the bearer header is the only way in today). The subscriber is registered at 2748. `RevokeDevice` closes the device's subscribers at `app.rs` 486–495.
- **Commands.** `remote_command` 3159; a read-scope device is refused at 3166 (`device.scope != Scope::Control`). The same check recurs at 3232, 3384, 3482 and 2156. `send` takes an `op` in the body (the comment at 3154–3158 says it is the idempotency key).
- **Pairing core.** `src/core/remote.rs`: `OpenPairing` sets the window (`code` uppercased, `scope`, `failures = 0`); `PairingFailed` counts; `Paired` closes the window (so each pairing needs a new window); `RevokeDevice` at line 86 is a `retain`; `SetDeviceScope` at 93; `pairing_at` 152; `new_device` 166.
- **Rotation.** `src/app.rs` `load_identity` 2782. When the key is missing or does not match, a fresh identity is made and `devices.clear()` runs (2816, and again at 2839 in the other branch), so every record goes.
- **Identity and certificate.** `make_identity` in `src/adapters/remote.rs` 95–128. `rcgen::CertificateParams::new(Vec::<String>::new())` at 102: **the certificate has no subject alternative names**. The only name is the common name `Switchboard <server_id>` (104–106). No validity period is set in this code. The SPKI hash is `sha256_hex` of `key.subject_public_key_info()`.
- **Pairing code.** `pairing_code` 325: 8 Crockford characters, 40 bits.
- **Pairing window UI.** `src/ui/remote.rs`: `pairing` 141. The payload is JSON built at 160–165: `server_id`, `spki_sha256`, `code`, and `addresses` (one entry at most). The address is `format!("{ip}:{port}")` at 158, which would not be a valid URL host for an IPv6 address. The QR is drawn by `qr_texture` (≈260–290) and the text is copied by "Copy invite" (201). `devices` 230 lists scope, last seen and Revoke. The "Allow devices on the local network" checkbox is in `host` 89.
- **Native text pairing.** `client/Sources/switchboard-remote/main.swift` 107–125: `pair '<invite json>'` or `pair <host:port> <code> <spki>`. `PairingInvite.parse` (JSON) is also called by `client/Sources/SwitchboardUI/BrowserView.swift:106` and `:883`, the headset's paste path. `client/Sources/SwitchboardProtocol/Info.swift` holds `PairingInvite`, `PairRequest`, `PairReply` and `ServerInfo`. `PairReply` decodes `token` from the body.
- **Tests.** `tests/remote.rs`: `http` 245 (a raw TLS client over `pinned_client`), `get` 327, `pair` 406, `info_is_served_to_anyone_and_every_other_read_is_refused_unpaired` 425, `a_pairing_inside_the_window_gives_a_token...` 477, `the_allowlist_refuses_everything_else_with_no_side_effect` 681 (its list covers `/api/v2/projects`, `/env/resolve` and similar, not `/`), `a_wrong_pin_fails_at_the_handshake_before_any_request` 1395, `pair_is_rate_limited_per_listener` 1406, `write_fixtures` 1587 (the Swift fixtures under `client/Fixtures`), `the_client_fixtures_decode_on_both_sides` 1829, `subscribe_at` 1960.
- **Spec.** `docs/remote-client.md`: §4.1 bounds 118–139 (four unauthenticated, 128 watch keys, 8 drains per pass, 64 inbox); §4.2 caps and envelope 140–156; §4.3 endpoints 157–220 (no static route, and no route that returns the calling device's own name or scope); §4.4 events 221–268; §5 pairing 302–320, whose first bullet is "Require pairing **before** anything but `/info` and `/pair`" (the rule the issue would amend); §5 also says the token "is sent only as a bearer header over the pinned connection, never in a URL"; §9 302–396; §11 tests 471–511; §13 table 524–553.
- **Design doc.** `docs/design.md` "Remote client (2026-10-12)" at 3107 and the status text after it; the trust boundary at 219. The status section says the port "does not listen" on the LAN unless the setting is on (3130–3160).

## 2. The on-disk record and how it rehydrates, before the pane

Nothing in this feature needs a new record type for the core. The records that exist:

- **`identity.json`** (in the data directory, `RemoteFiles` at `adapters/remote.rs` 45–90). Holds `server_id`, `server_name`, `certificate` (base64 DER), `spki_sha256`, `created` and `listen` (`lan`, `port`). Rehydrated at startup by `load_identity` (app.rs 2782) into `IdentityLoaded`. The browser would read it only through `/info`.
- **The private key**: Keychain, account `remote-key:<server_id>`, read by `load_identity`. If it is missing, a new identity is made and every device record is cleared.
- **`devices.json`**: a `PairedDevice` per paired device (`id`, `name`, `scope`, `token_sha256`, `created`, `last_seen`), `ports/remote.rs`. A browser pairing would add one of these exactly as a native pairing does. The token is never stored on the host, only its hash. Scope changes and revocation save this file (`core/remote.rs` 86–100). `last_seen` is saved only when it moved by 300 s (`LAST_SEEN_SAVE_INTERVAL_SECS`, `core/remote.rs` 18).
- **The pairing window** is in memory only (`core/remote.rs` `pairing`). It is not saved, so a restart closes an open window. Its code is never written anywhere.
- **Browser-side token**: nothing exists yet. The issue's two options (an `HttpOnly` cookie, or a token in `Sec-WebSocket-Protocol`) and the `localStorage` option each put the token somewhere the host does not currently write to. The host would hold the same record either way.

Rehydration of a browser after a reload or a restart, as the tree stands:

- The page's own state is nothing but the token and whatever else the plan stores (scope, host name, the pinned key). Everything else comes back through REST: `/info`, the collections and the content endpoints.
- A host restart gives a new `epoch` (the port's own epoch, `app.rs` `self.epoch`); the spec (§4.4, §11) says the client refetches everything on a new epoch. The page would need the same rule.
- A key rotation at launch removes every record in `devices.json`. A browser still holding its old token then gets the same 401 as a revoked device (see §4 below).
- A browser is not revived by the host. A session's resume stays user-initiated (§13).

## 3. What the pane can show from what exists

- **Host name and key**: `/info`, unauthenticated, `server_name` and `spki_sha256`, plus `pairing_open`.
- **Waiting, first**: `GET /api/v1/waiting` (a collection, answered whole) and the `waiting` flag and `waiting_reason` on each session (`control/src/lib.rs` `SessionView` at 637; `waiting` at 657, `waiting_reason` at 658).
- **Projects and sessions**: `GET /api/v1/projects`, `GET /api/v1/projects/{id}/sessions`, `GET /api/v1/sessions/{id}`. Each session carries `kind`, `liveness` (640s), `name`, `card` (646, the one-line excerpt the card shows, to be confirmed in the UI code), `last_stop_at_ms`, `last_activity_ms` and `pending` (674).
- **A session's last response or screen**: `GET /api/v1/sessions/{id}/last-response` (routed at `app.rs` 3101 and `remote_content_route` 1947; `ContentView::LastResponse` at 1967) and `GET /api/v1/sessions/{id}/screen?lines=n`.
- **Live changes**: `GET /api/v1/events` with the `changed` kinds in §4.4. The spec says a session entering or leaving `waiting` emits `changed {waiting}`; I did not verify that emission in the code.
- **Reply**: `POST /api/v1/sessions/{id}/send` with an `op` (control scope required).
- **Not visible to the page**: the calling device's own name and scope. No route returns them (the `match` at `app.rs` 3031–3125 has none). The `/pair` reply has them, once, and nothing else does.

## 4. Facts in the tree that the plan has to deal with

1. **The certificate names nothing.** `make_identity` (`adapters/remote.rs` 102) sets no SAN, so a browser cannot match the certificate to the IP in the QR URL. Existing identities have that certificate; a new one would need a new certificate from the same key (the spec's §5 says a reissued certificate from the same key keeps the pin). Whether this affects a click-through is a question below.
2. **Every path crosses to the frame, and static paths would share the unauthenticated pool.** A `GET /` today is queued to the frame and answered by `refuse(404)`. The pool is four connections (567). A browser loading a page with several assets in parallel, or holding idle keep-alive connections, can fill it, and the accept loop then drops the fifth connection without a reply (the drop at about 699–706). The 30 s `IDLE_TIMEOUT` (573) holds a slot between requests once the first request is read.
3. **The query string reaches the host.** The `GET /?code=…&key=…&id=…` that the issue's QR opens is sent to the host as the request target. `read_request` splits it into `RemoteRequest.query` (`adapters/remote.rs` 496–502). Logging is by path only (`refuse` at `app.rs` 3754 logs `request.path`), so the code is not logged today, but any new static handler would have to keep it out of `query` handling and logs.
4. **The code can end up in the browser's cache.** The static response for `/?code=…` has no cache header today, because `respond` sets none (530–548). The plan's "no caching across versions" needs a header rule that also covers the query.
5. **`/pair` returns the token in the body.** `remote_pair` (3606) always writes `token` into the JSON. A cookie-based browser design would need a second mode (or a second endpoint) so the token does not also sit in a JavaScript-readable body; the native clients read the body (`Info.swift`, `PairReply`).
6. **Authentication reads only the bearer header.** `read_request` (487) and `authenticate` (3127). A cookie or a subprotocol token means changes to `read_request`, to `RemoteRequest`, and to `authenticate`, and a `Set-Cookie` path in `respond` (which cannot send one today).
7. **The WebSocket upgrade echoes no subprotocol.** `respond_upgrade` (519) writes the 101 with only `Upgrade`, `Connection` and `Sec-WebSocket-Accept`. Offering a token as a subprotocol needs the 101 to name one of the offered values.
8. **No `Origin` check anywhere.** `grep` finds no `origin` in `adapters/remote.rs` or `app.rs`. A cookie-authenticated WebSocket would be the first place this matters.
9. **Revoked and rotated look the same on the wire.** Both lead to `authenticate` 3146 ("the token is not known; pair again", 401). The issue's acceptance wants the page to say "revoked" on its next request. The tree gives it no way to tell the two apart.
10. **The page cannot tell its own scope.** See §3. A read device's `send` gets the refusal at 3166; the page can learn the scope only from a refusal or from storing it at pairing.
11. **The pairing URL needs a new parser on the native side.** The CLI (`main.swift:111`) and both headset paste sites (`BrowserView.swift:106`, `:883`) parse only JSON via `PairingInvite.parse`. A URL form needs its own parse, or the JSON stays.
12. **Only one address goes in the QR.** `local_address` (`adapters/remote.rs` 962–969) returns one IP, and the QR carries it as a one-element array. The URL form has the same single host, and an IPv6 address would need brackets, which the format at `ui/remote.rs` 158 lacks.
13. **Multiple addresses, one origin each.** A browser's certificate exception, and a cookie, are per host and port. An address that is not the one the phone used has no exception.
14. **The spec's §5 bullet and §4.3 table would both change.** §5 "Require pairing before anything but `/info` and `/pair`" and the §4.3 table are the text the issue's rule amends. §11 has no browser test.
15. **Existing tests that would change.** `info_is_served_to_anyone_and_every_other_read_is_refused_unpaired` (`tests/remote.rs` 425) and `the_allowlist_refuses_everything_else...` (681) do not assert on `/` itself, so they should survive a root route, but the plan should check their exact asserts when it amends them. `write_fixtures` (1587) is needed only if a view changes; the page uses the existing views.
16. **Dispatch tickets already ship on this port.** `serve_remote` routes `/api/v1/dispatch/tickets` and related paths (`app.rs` 3078–3098), although the issue puts tickets out of scope. The page is not expected to call them, but the paired-device surface already exposes them.

## 5. Claims in the issue that are about the world outside the tree

These are not findings. Each is a question until something outside the tree settles it.

- A phone's camera opens the URL and the browser follows it. Nothing in the tree decides this. [run] [owner]
- The browser shows a certificate warning, and the click-through trusts that certificate for that address. [source]
- The click-through trusts the certificate for the `wss://` WebSocket on the same origin too. [source]
- The `key` in the URL "lets a genuine page refuse to send the code to a host whose `/info` key differs". The logic holds only if the page is genuine; the issue says so. Not checked further. [source]
- `history.replaceState` removes the code from the address bar and the history entry for the current page. Whether the browser's history or the back-forward list keeps the URL from before the script ran is unknown. [source] [run]
- A `Secure`, `SameSite=Strict` cookie set over the self-signed origin is sent on the WebSocket upgrade, and a cross-site page cannot make the browser send it. [source]
- A WebSocket subprotocol token is accepted by a browser only if the server echoes one of the offered names. [source]
- `connect-src 'self'` in a Content-Security-Policy covers `wss:` to the same host and port. [source]
- A browser opens how many concurrent connections to one origin, and how long it keeps idle ones, on iOS Safari and Chrome. This decides whether four unauthenticated slots are enough. [source] [run]
- iOS Safari and Chrome accept a click-through for an origin given by an IP address, with a certificate that has no SAN, and with the validity period rcgen sets by default. The tree sets no validity itself; rcgen's default was not checked. [source] [run]
- The device name can be derived from the user agent (`Safari on iPhone`). A user agent string is reduced or frozen by the browser, and iPadOS can present as a Mac. [source]
- A Safari "Visit website" choice persists across launches and reloads, and whether iOS storage eviction affects cookies or `localStorage` on an IP origin. [source] [run]
- A browser's cache entry for `/?code=…` is keyed by the full URL, query included. [source]

## 6. What I did not check

- I did not run `cargo test`, the app, a browser, or a phone. Nothing here was measured.
- I did not read `client/Sources/SwiftUI` views beyond the call sites above, or the visionOS app's Keychain code.
- I did not read the rest of `tests/remote.rs` (only its outline and the cited tests) or `src/core/action.rs`.
- I did not read the rcgen, rustls, tungstenite or browser sources; every claim about them is in §5.
- I did not check whether the pairing UI's card excerpt is the `card` field; the name is the best match in `SessionView`.
- I did not verify the `changed {waiting}` emission the spec describes.
- I did not read the `/api/v1/dispatch/*` handlers beyond their routes.

## Open questions

[tree] Does the `card` field (control/src/lib.rs 646) carry the one-line excerpt the session card shows, or is that a different field?
[tree] Is a `changed {kind: session}` event emitted when `waiting` flips (the spec's §11 test), and where?
[tree] Which of the four unauthenticated-pool facts above change if a static path is answered on the connection thread rather than the frame?
[tree] Would a root route break `info_is_served_to_anyone_and_every_other_read_is_refused_unpaired` or `the_allowlist_refuses_everything_else` as written?
[tree] Does anything outside the two Swift call sites (`BrowserView.swift:106`, `:883`) and the CLI read the invite's JSON form?
[tree] Does a new certificate from the existing key (with SANs) need a new `identity.json` entry, and does the pin survive it? The spec says it does; the code path for reissue is not in the tree.
[tree] What exact code does a read-scope device's `send` return at `app.rs` 3166, and does the page need it to show "read only"?
[tree] Should `local_address` and the QR's `addresses` return more than one address, given the browser needs one URL per origin?
[source] Do iOS Safari and Chrome keep a certificate exception for the wss:// upgrade on the same origin when the certificate has no SAN?
[source] Does the default validity period of rcgen 0.14's self-signed certificate matter to iOS Safari or Chrome for an exception?
[source] Does a browser send a `Secure; SameSite=Strict` cookie on a WebSocket upgrade to the same origin, and does it send `Origin`?
[source] What must the 101 response contain for a browser to accept a token offered as `Sec-WebSocket-Protocol`?
[source] Does `connect-src 'self'` cover `wss:` to the same host and port in current browsers?
[source] Does `history.replaceState` remove the code from the back-forward list on iOS Safari, given that the navigation was recorded before the script ran?
[source] How many concurrent connections does iOS Safari and Chrome open to one HTTPS origin, and do idle keep-alive connections hold them?
[source] Is a response for `/?code=…` cached on disk under its full URL, and does `Cache-Control: no-store` stop it?
[source] Does a browser's user agent string give a stable device name (`Safari on iPhone`), including on iPadOS?
[source] Does an `Origin` check have to accompany cookie auth on the WebSocket, in the browser's own rules, beyond SameSite?
[run] Does a phone's camera open an `https://` URL with a self-signed certificate, and does the warning offer a way through on iOS?
[run] Does the click-through trust the certificate for the WebSocket as well as the page, on a real phone?
[run] How long does a page load hold unauthenticated pool slots on a real phone, and does a fifth connection get dropped?
[run] Does a session entering `waiting` reach the page within one pass, and does the page refetch the collection whole?
[run] Does the page's `send` arrive in the pane, and does the reply reach the page as a `changed` event?
[owner] Is the click-through the trust step (said in the paired-devices dialog), or should the host offer its certificate for download first?
[owner] Should the browser's `/pair` reply carry the token in the body as well, or only as a cookie, given the native clients read the body?
[owner] Should the URL replace the JSON invite everywhere, or do the headset's paste sites and the CLI keep the JSON form?
[owner] Should a revoked device and a key rotation give the page different messages, and which one should the page show for a device the host forgot?
[owner] Does the page store the key it paired with, so it can say "identity changed, pair again" after a rotation?
[owner] Which pool and rate window does an unpaired page load count against, and how many slots should static paths get?
[owner] What does the page show when opened with no code and no token?
[owner] Does the device record gain a field for the page's own scope, or does the page store it at pairing?
[owner] Should the page be shown on a LAN with the "Allow devices on the local network" setting off, or only on?
[owner] Is the scope of the browser pairing chosen in the window as today, or does the page get its own default?
