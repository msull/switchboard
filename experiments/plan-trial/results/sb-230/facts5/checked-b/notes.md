# Notes: issue #230, a web control surface on the remote port

Read: CLAUDE.md, docs/lessons.md (a retrospective of an earlier app; nothing in it bears on this issue),
docs/remote-client.md §4.1, §4.3, §4.4, §5, §9, the "Remote client" status section of docs/design.md
(starts at line 3107), and the code below. Line numbers are from this worktree (branch
`dispatch/230-...`, HEAD 7fe8949). I ran nothing and opened no browser.

## On-disk record and rehydration (before the pane)

Nothing new has to be stored for the first cut. The page is data compiled into the binary; the state is already durable:

- `identity.json` (`Identity`: server_id, name, certificate DER base64, `spki_sha256`, listen). The key is in the
  Keychain under `remote-key:<server_id>` (`src/adapters/remote.rs:95` `make_identity`, `src/app.rs:2783` `load_identity`).
- `devices.json` (`PairedDevice`, `src/ports/remote.rs:96`): id, name, scope, **token SHA-256 only**, created, last_seen.
  A browser is a row of the same shape. Its token is stored by the host only as a hash, so the browser's copy is its only
  copy; this is the rehydration question. Today the token reaches the client in the `/pair` body (`src/app.rs:3606`
  `remote_pair`, reply keys `token, scope, server_id, device_id`). The device keeps it. For the page, "rehydrate" means:
  reload the page, present the same credential (cookie, or whatever storage Decision 2 picks), read `/api/v1/*` again.
  The pairing window itself is never saved (`core/remote.rs`, `pairing_at`); a restart closes it.
- Revocation = delete the row (`AppAction::RevokeDevice`), and the next request 401s (`authenticate`, `src/app.rs:3127`)
  and the device's socket closes (test `revoke_closes_the_devices_socket`, tests/remote.rs:2892). The page's "revoked"
  state is therefore a 401 `unauthorized` "the token is not known; pair again" from any `/api/v1` call. The host cannot tell
  "revoked" from "never paired" or "key rotated" on a 401 (same body); "identity changed" is only detectable client-side,
  by `/info.spki_sha256` or `server_id` differing from what the page remembered (or `server_id` equal and key different).
  A rotated key deletes every device (docs §5, `identity_rotated`), so the page's stored credential just 401s.
- Nothing in the page's state is read by the core. Per CLAUDE.md the core stays deterministic, so serving static files
  is an adapter/app concern, not a core one.

## Where the issue touches the code

Serving the page
- `src/app.rs:3017` `serve_remote`: line 3023 `strip_prefix("/api/v1/")` else `refuse(404 not_found)`. This is the one place
  a non-API path is refused; `GET /` and static files must be answered before it. The match at 3028 is the API allowlist.
- `src/adapters/remote.rs:360` `RemoteRequest` and `RemoteReply` (389): `RemoteReply` is `{status, body, authenticated}`
  only. **There are no response headers and no content type.** `respond` (530) hard-codes `Content-Type: application/json`,
  `Connection: keep-alive`, and a fixed reason table (no 302/304/etc.). Static files, `Set-Cookie`, CSP, `Cache-Control`
  and `Content-Type: text/html` all need a header channel on `RemoteReply` (additive; 30 construction sites of
  `RemoteReply {` across src and tests, so a `Default`/constructor helps) or a path decided in the connection thread
  before the frame is asked.
- `read_request` (444) keeps only: method, path, query (undecoded), body, bearer, upgrade key. It **drops every other
  header**: no `Cookie`, no `User-Agent`, no `Origin`, no `Host`, no `Sec-WebSocket-Protocol`, no `Accept-Encoding`. Each
  one the plan wants must be added to `RemoteRequest` (CLAUDE.md: the bearer is taken out of the headers so it is never
  logged with them, so a cookie must get the same treatment). `MAX_HEAD` is 8 KiB (437).
- Query values are "undecoded: ids and numbers only" (line 377 doc). The URL form carries `code` (Crockford, safe), `key`
  (hex), `id` (uuid), all URL-safe, so decoding is not needed for these, but the doc comment's promise would be amended.
- `connection` (770): per-connection loop; `Limits::limited` (631) only rate-limits the exact paths `/api/v1/pair` (6/min) and
  `/api/v1/info` (60/min). Any other path, including a new `/` or `/app.js`, is **not rate limited at all** today.
  The unauthenticated pool is 4 connections (`MAX_UNAUTHENTICATED`, 567), counted per TCP connection until the first
  reply with `authenticated: true` (810 on). A browser opens several parallel connections to load `/`, `.css`, `.js`, `/info`
  (typically up to 6 per host) with keep-alive; that can fill the 4-slot unauthenticated pool and the accept loop silently
  drops the rest (`bind`, 690-700). Which pool a page load counts against follows from this code: all of it is
  "unauthenticated" until a request answers `authenticated: true`; static-file replies are `false` unless the plan changes that.
  Idle timeout 30 s (573) holds a slot from a browser that keeps a connection open.
- The `ANSWER_TIMEOUT`/frame hop: every request, including a static one, goes over the 64-deep channel to the frame, drained
  8 per pass (`serve_pending`, src/app.rs:1572). Serving a page this way costs frame passes; answering static paths on the
  connection thread would not, but then they bypass the frame's refuse/log path. Plan's call; both are in the tree.
- `local_address()` (adapters/remote.rs:962) picks one outbound IPv4/IPv6 source address; it is not bracketed for a URL.

Pairing and the QR
- `src/ui/remote.rs` `pairing` (≈ line 150-220): builds the JSON `{server_id, spki_sha256, code, addresses}` (the payload
  variable), makes the QR from the same string (`qr_texture`, 254 on), and "Copy invite" copies the same text. Address is
  `local_address():port` where port is `cx.remote_port` or the configured one. Note: if the port is bound to loopback
  (`listen.lan` false, `bind_remote` at src/app.rs:2938) the QR still shows the LAN address, which nothing answers; a phone
  scan would fail. The dialog's checkbox text says "only paired devices get past /info" (ui/remote.rs ≈ line 108) and will be
  wrong once the page itself is served before pairing.
- `core/remote.rs`: pairing window state, `OpenPairing{code, scope}`, `PairingFailed` (three wrong closes it), `new_device`.
  The code is 8 chars, 40 bits (`pairing_code`, adapters/remote.rs:325), single use; `Paired` consumes it. The device name
  comes from the body's `device_name` (src/app.rs ≈ 3640, defaults to "Device"); **a user agent is not available to the host**
  (not retained, see above), so "Safari on iPhone" must come from the page (`navigator.userAgent`/`userAgentData`) or from a
  new `RemoteRequest` header field. The page choosing its own name is the same trust as any device.
- `remote_pair` (3606) requires `spki_sha256` in the body to equal the host's. A browser page cannot compute the key of the
  certificate it is talking to (no JS API); it can only compare the URL's `key` with `/info.spki_sha256`, which the **host
  itself** returns, so that comparison proves only that the URL's key matches whatever answered. This is what the issue
  already says; recorded here because the check cannot be stronger from the tree.
- Native text path: `client/Sources/SwitchboardProtocol/Info.swift:94` `PairingInvite` is `Decodable` with `parse(_:)`
  = `Wire.decode(JSON)`; `client/Sources/switchboard-remote/main.swift:107-125` takes either `<invite json>` (one argument) or
  `<host:port> <code> <spki_sha256>`. It is also used by `ConnectionManager.pair(invite:)` and about ten Swift tests
  (`ReconcileTests`, `EventTests`, `DraftTests`, `NewSessionTests`, `FixturesTests.swift:161` with a fixture `client/Fixtures/pin.json`/`pair.json`).
  A URL accepted there means a `parse` that tries URL first, then JSON; the `addresses` array becomes one entry from the URL authority.
  The headset's paste (apps under `client/Apps`) was not read.
- README.md ≈ line 271-280 documents "Copy invite" + `switchboard-remote pair "$(pbpaste)"` and `curl -sk`; `SWITCHBOARD_SCRIPT`
  has `pairing <code>` / `remote-listen lan <port>` (src/script.rs:729-730) which is the way to put an instance in the state the
  acceptance test needs.

Token transport (Decision 2) in this code
- `authenticate` (src/app.rs:3127) reads `request.bearer` only. The events route is `authenticate` then `remote_subscribe`
  (3051, 2723); the WebSocket handshake is read by the same `read_request` and upgraded in `connection` (843). A cookie or
  `Sec-WebSocket-Protocol` means adding a field to `RemoteRequest` and, for the subprotocol, echoing the chosen protocol in
  `respond_upgrade` (519), which today writes no `Sec-WebSocket-Protocol` line (a browser closes the socket if it offered
  protocols and the reply selects none).
- There is **no `Origin` check** on the upgrade or on any request. With a cookie credential this matters (cross-site
  WebSocket hijacking, and CSRF on `POST .../send`). With a bearer header it does not, because a header cannot be forged
  cross-site. The tree has no CSRF defence to reuse. Content-Type is not checked on POST bodies either (`serde_json::from_slice`
  of the body, src/app.rs ≈ 3180), so a cross-site form POST of JSON text would parse.
- `/pair` reply is built by `envelope` (src/app.rs ≈ 3727, wraps `epoch`/`revision`?) — I did not read `envelope`; whether the
  pair reply carries them was not checked.

Data the page needs, and what the wire gives
- `GET /api/v1/projects`, `/projects/{id}/sessions`, `/waiting`, `/sessions/{id}`, `/sessions/{id}/last-response`,
  `/sessions/{id}/screen?lines=n`, `POST /sessions/{id}/send`, `GET /events` — all exist (src/app.rs:3033-3100).
  The send body shape (`text`, `op`) is at `remote_command` (3157); not read to the end.
- `SessionView` (control/src/lib.rs:637) has `kind`, `card` (the card's word), `waiting`, `waiting_reason`, `liveness`,
  `quiet_secs`, `last_activity_ms`, etc. **It has no excerpt field.** The card's one-line excerpt is computed in the UI only:
  `src/ui/cards.rs:558` `agent_excerpt` (first line of the last turn's `final_text`, else last activity, else "You: …", capped
  at 160 chars) from `cx.conversations`, and for non-agents from pane captions refreshed every two seconds (`UiState::captions`).
  docs §4.3 even says the excerpt "is not part of" `last_response`. So "each with … the one-line excerpt the card shows" is a
  **new read model** (the issue says "No new read model unless the page cannot do without one"), or N extra
  `last-response` fetches per list, or the page shows `waiting_reason`/`card` only. Settling that is the plan's choice;
  that the gap exists is a finding.
- Grouping by project and "waiting first": `waiting` is a boolean on `SessionView`, and `/waiting` is a collection.
  `?scope=all` includes Dispatch worker sessions; default hides them (`hide_workers`). The page should not pass `scope=all`.
- Reconcile rule (§4.4): `hello` carries epoch+revision; snapshots carry revision; events past the snapshot revision invalidate.
  The Swift client implements it in `SwitchboardClientCore` (`ServerConnection.swift`, `ConnectionManager.swift`, tests `ReconcileTests`,
  `EventTests`, `WatchTests`); the JS port can be written from those and from `client/Fixtures/*.json` plus the Rust fixture writer
  `write_fixtures` (tests/remote.rs:1587, `SWITCHBOARD_WRITE_FIXTURES=1`).
- Content `changed` events only flow for watched keys: the page must send `watch {kind:"conversation"|"screen", id}` text
  frames (§4.4 "Watch frames", handled by `subscriber_frame`, src/app.rs ≈ 1700-1740) and the 30-second refetch applies.

TLS/certificate (Decision 1) in this code
- `make_identity` (adapters/remote.rs:95): `CertificateParams::new(vec![])` so **no subjectAltName at all**, CN
  `Switchboard <server_id>`, `not_after` year 4096 (comment at 90). `tls_config` (144) serves it. `Pin` verifier (183) is the
  native-client side only. A browser will treat the missing SAN, and the IP-literal host, as name-mismatch in addition to
  self-signed (see Open questions). The certificate is already in `identity.json` (`certificate_der`, 130), so a
  "download the certificate" route can be served with no new storage; it would be a new unauthenticated path (static,
  public data: the certificate is sent in every handshake anyway).
- Bonjour (`adapters/bonjour.rs`, `_switchboard._tcp`, TXT v/name/id) advertises a name; a `<name>.local` host would be a
  name to put in a certificate SAN but nothing does that today.
- Listener: loopback by default, `0.0.0.0` when `listen.lan` (src/app.rs:2938-2947). §5 "no non-loopback bind serves
  anything but /info and /pair" is the rule to amend; `docs/remote-client.md` §4.1/§5 and design.md "Remote client" status
  (3107 on) are the places the plan edits. Note design.md has "Milestone N status" sections; CLAUDE.md says update the relevant one.

## Files that change or get added (all by CLAUDE.md's rules)

- `src/adapters/remote.rs` (RemoteRequest/RemoteReply/respond/read_request/respond_upgrade/Limits; the new `include_str!` page files could live in
  a new `src/adapters/web.rs` or `web/` at repo root as the issue says; `Cargo.toml` `include` list should be checked if the crate is packaged — not checked).
- `src/app.rs` `serve_remote` and `remote_pair` (cookie), `remote_info`.
- `src/ui/remote.rs` payload → URL; dialog copy; "Copy invite".
- `client/Sources/SwitchboardProtocol/Info.swift` `PairingInvite.parse`; `client/Sources/switchboard-remote/main.swift` usage text;
  `client/Tests/.../FixturesTests.swift`; client README; the headset's paste field (not located).
- `tests/remote.rs`: `http` helper (245) returns only `(status, body)` and reads only `Content-Length`; a header-asserting test needs a
  variant that returns headers. `tests/ui.rs` covers the dialog (QR text assertion if any: not checked).
- `docs/remote-client.md` §4.1, §4.3, §5; `docs/design.md` remote status; README Layout table (one line per new file) and the
  `curl`/`Copy invite` paragraph; `CHANGELOG.md` Unreleased (currently empty, line 9); `src/lib.rs` layering only if a new module.
- Hard rules touched: nothing writes into project dirs; no secrets in logs (a cookie value must not be logged: `refuse` logs method+path only,
  `serve` logs ops, so path-only logging holds as long as the query is not logged. **A `?code=` URL path is query, and `request.path` is logged
  without the query (`read_request` splits it)**, so the code does not reach the log from the host side; the browser's own history/`Referer` are the leak, and
  the page's `replaceState` addresses them. `Referrer-Policy: no-referrer` on the page is cheap and appropriate; the app is not in the tree yet.)
- Naming: no client or customer names anywhere; nothing in the investigation touches them.

## What I did not check

- `envelope` and the rest of `remote_command` (send body fields); `client/Apps` (headset paste field); `tests/ui.rs` coverage of the dialog's QR text;
  whether `Cargo.toml` has an `include`/`exclude` list that would drop `web/` from a published or bundled crate; `scripts/bundle.sh`
  (embedding at compile time makes the bundle unaffected, I assume); `docs/design.md` beyond the remote status heading.

## Open questions

- [tree] Does the `/pair` reply (`envelope`, src/app.rs ≈ 3727) carry epoch/revision, and what are the exact `send` body fields and error codes (`remote_command`, src/app.rs:3157-3260) the page must speak?
- [tree] Where does the headset app's paste/typed-code path parse an invite (client/Apps), and does anything besides `PairingInvite.parse` and `switchboard-remote` consume the JSON form (so Decision 3 can be settled)?
- [tree] Does any UI test in tests/ui.rs assert the QR payload or the "only paired devices get past /info" label, and does `Cargo.toml`/`scripts/bundle.sh` limit which files a `web/` directory needs to be listed in?
- [tree] Is the card excerpt (`agent_excerpt`, pane `captions`) cheap to expose as an additive `SessionView` field without breaking the Swift fixtures and `revision.rs` fingerprints, versus fetching `last-response` per session?
- [source] Do Safari (iOS/macOS), Chrome and Firefox let the user click through a TLS interstitial for a self-signed certificate that has no subjectAltName and is addressed by an IP literal (the failure: ERR_CERT_COMMON_NAME_INVALID / "not private" with or without a proceed link), and does HSTS or "HTTPS-First" mode change it?
- [source] After a click-through, do those browsers accept a `Secure` cookie, `fetch` and `new WebSocket("wss://…")` to that origin for the session, and for how long is the exception remembered (iOS Safari in particular, which asks again after a short period or on restart)?
- [source] Does a certificate with `notAfter` in year 4096 (rcgen default), a P-256 key, and no SAN hit any browser or OS validity-limit rule (the 398-day limit, iOS "trust this certificate" profile requirements) if the owner installs it from a host download?
- [source] Cookie scoping across ports: cookies are not port-isolated (RFC 6265), and `SameSite=Strict` treats different ports on one host as same-site; what does that mean for an `HttpOnly; Secure` cookie set by this port being sent to, and read by, other services on that host?
- [source] The WebSocket handshake in browsers: which headers are sent (`Cookie`, `Origin`, `Sec-WebSocket-Protocol`), whether a browser closes the socket if the server does not echo an offered subprotocol, and whether `Sec-WebSocket-Protocol` values may carry a 43-char base64url token (token characters allowed by RFC 6455 §4.1 / RFC 7230).
- [source] Does `history.replaceState` remove the query from what a phone camera's in-app browser or Safari later offers in history, tab previews, or `Referer`, and does a Safari/iOS camera link-open land in Safari itself or in a QR preview webview with different certificate-warning behaviour?
- [source] Does the camera app on iOS and Android offer to open an `https://<ip>:<port>/?…` URL from a QR (IP literal, non-default port, ~120 characters), and does the QR at the existing payload size stay readable?
- [run] Whether a browser page load (HTML, CSS, JS, then `/info`, then `/pair`) stays inside the 4-connection unauthenticated pool and `Limits` with keep-alive and up to six parallel connections, given `connection` holds a slot until the first authenticated reply and the accept loop drops over-pool connections silently.
- [owner] Whether the owner's actual phone/iPad/other Mac browsers accept the click-through on the host's current certificate and keep it across a day (needs the owner's devices and a signed bundle's identity).
- [owner] Which network the owner will use the phone on (a `.local` name vs an IP, IPv6 link-local, a guest network that isolates clients), since `local_address()` picks one outbound interface.
