# Issue #230 — a web control surface on the remote port: investigation notes

Worktree: `/Users/sbgragent/.dispatch/worktrees/299903e5`, HEAD `7fe8949` (Release 0.4.1), branch `dispatch/230-…`.
Read: CLAUDE.md (system-supplied copy), `docs/design.md` (remote status §3107, trust boundary §219), `docs/lessons.md`, `docs/remote-client.md` §§3–5, 10–13 (§4.1–4.4 read in full), `src/adapters/remote.rs` (1–560, 586–660, 765–880, 960–976), `src/app.rs` (remote regions), `src/core/remote.rs`, `src/ports/remote.rs` (1–150), `src/ui/remote.rs`, `control/src/lib.rs` (SessionView), `src/ui/cards.rs` (caption and excerpt), `client/Sources/switchboard-remote/main.swift`, `client/Sources/SwitchboardProtocol/Info.swift`, `tests/remote.rs` (names and helpers), `Cargo.toml`. Nothing was run and nothing was edited.

## 1. The surface the page would sit on

**Routing and responses.** `serve_remote` (`src/app.rs:3017`) refuses any path without the `/api/v1/` prefix with a 404 JSON `not_found` (`src/app.rs:3023`). That refusal runs on the frame, so every stray `GET /` costs a frame pass and an unauthenticated slot. Only `GET /api/v1/info` and `POST /api/v1/pair` are answered before authentication (`src/app.rs:3028-3029`). `respond` (`src/adapters/remote.rs:530-553`) writes `Content-Type: application/json` unconditionally (line 545) and has no way to add headers. `RemoteReply` (`src/adapters/remote.rs:389-393`) is `{status, body, authenticated}` with no headers field, so neither `Set-Cookie` nor a CSP can leave the port today.

**Request parsing.** `read_request` (`src/adapters/remote.rs:444`) keeps the query, the `Authorization: Bearer` value (line 487), and `Upgrade` with `Sec-WebSocket-Key` (line 491). It discards `Cookie`, `Origin`, `Sec-Fetch-*`, `User-Agent` and `Sec-WebSocket-Protocol`. Nothing checks `Host`.

**WebSocket.** `respond_upgrade` (`src/adapters/remote.rs:519-527`) writes a 101 with no `Sec-WebSocket-Protocol`. The upgrade's bearer check runs in `serve_remote` before `remote_subscribe` (`src/app.rs:3036-3037`, `authenticate` at `src/app.rs:3127`). A refused upgrade gets the JSON error as a plain HTTP response, which a browser's WebSocket object cannot read.

**TLS.** `tls_config` (`src/adapters/remote.rs:144-155`) sets no ALPN, so a client falls back to HTTP/1.1. The certificate is made with `rcgen::CertificateParams::new(vec![])` (`src/adapters/remote.rs:102`): no subject alternative names, a common name of `Switchboard <server_id>`, and rcgen's default far-future `not_after` (per the doc comment at line 91).

**Pools and limits.** `MAX_UNAUTHENTICATED = 4`, `MAX_PAIRED = 12` (`src/adapters/remote.rs:567-568`). The accept loop drops a stranger past the pool at once (`src/adapters/remote.rs:701-706`). A connection starts in the unauthenticated pool and moves to the paired pool only on its first authenticated reply (`src/adapters/remote.rs:835-845`). After the first request the read timeout is `IDLE_TIMEOUT = 30 s` (`src/adapters/remote.rs:573`), so a keep-alive connection holds its slot for up to 30 s after its last request. Rate windows cover only the exact paths `/api/v1/pair` (6/min) and `/api/v1/info` (60/min) (`src/adapters/remote.rs:583-586`, `631-639`).

**Pairing.** `remote_pair` (`src/app.rs:3606`) checks the body's `spki_sha256` against the host's (`src/app.rs:3636`) before the code (`src/app.rs:3646`). A wrong key or a wrong code each count against the global window (`PairingFailed`). The success reply is a JSON body carrying the token in clear (`src/app.rs:3660-3665`), with `scope`, `server_id` and `device_id`. The token is stored only as its SHA-256 (`src/core/remote.rs:166` `new_device`). The window is single use (`src/core/remote.rs`, the `Paired` arm closes `self.pairing`). The device name is the body's `device_name`, trimmed to 64 characters, default `"Device"` (`src/app.rs:3650-3655`, `src/core/remote.rs:166-174`).

**Refusals and logs.** `refuse` (`src/app.rs:3754-3772`) logs method and path, not the query, so the `code` in `?code=` is not logged on a refusal. The `authenticate` refusals for a missing bearer and for an unknown token share the code `unauthorized` (`src/app.rs:3127-3152`). The page therefore cannot tell "revoked" from "never paired" from the code.

**Scope.** `GET /api/v1/info` (`src/app.rs:3568-3599`) returns identity, `capabilities` (line 3580), limits and `pairing_open`. It does not return the caller's scope, and no endpoint does. A read device sending a command gets 403 `scope` (`src/app.rs:3166-3173`). The only place a page could learn its scope is the pair reply body.

**Send.** `POST /api/v1/sessions/{id}/send` takes `{op, text}` (`src/app.rs:3179`, `3192`), with `text` capped at `SEND_BYTES_CAP` (16 KiB) and `op` at 64 characters. Command routing is `remote_command` (`src/app.rs:3159`).

**Pairing window UI.** `pairing` (`src/ui/remote.rs:141`) builds the QR payload at lines 160-165 as JSON `{server_id, spki_sha256, code, addresses}`. `addresses` holds the single result of `remote::local_address()` (`src/ui/remote.rs:157`, `src/adapters/remote.rs:962-969`, an IPv4 UDP-connect trick). The text form is the same JSON, copied by "Copy invite" (`src/ui/remote.rs:201-205`). The QR is a `qrcode` texture (`src/ui/remote.rs:280`).

**A tree fact the QR does not reflect.** `bind_remote` listens on loopback unless `identity.listen.lan` is set (`src/app.rs:2962-2966`). The pairing window does not check that: it always prints `local_address()` (`src/ui/remote.rs:157`), so with LAN off the QR names an address the listener does not answer. The plan has to decide whether opening the window requires LAN.

## 2. What is stored, and how the port rehydrates

**On disk.**
- `identity.json`: `Identity` (`src/ports/remote.rs:26`) with `server_id`, `server_name`, the DER certificate, `spki_sha256`, `created`, `listen`. It has a `schema_version` field but the device file does not.
- The private key is a Keychain item under `remote-key:<server_id>` (`src/ports/remote.rs` `key_account`).
- `devices.json`: a bare `Vec<PairedDevice>` (`src/ports/remote.rs:96`; `load_devices` returns the vec with `unwrap_or_default`, `src/adapters/remote.rs:80-82`). Fields: `id`, `name`, `scope`, `token_sha256`, `created`, `last_seen` (`#[serde(default)]`). Adding a field needs `#[serde(default)]` on it; there is no version to bump.
- Session records, projects, sets and so on: the app's existing store. The port reads them through `AppCore` and never writes them.

**What a browser device adds to disk.** One more `PairedDevice` row: a name, a scope, the token's hash, times. The token itself is never stored. The cookie or the header the browser keeps lives only in that browser. The name is the only new data; a user-agent-derived name would be a string the device supplies, trimmed and capped at 64 characters, as today.

**Rehydration on the host.** On launch the core reads `identity.json` and `devices.json` (`IdentityLoaded`, `src/core/remote.rs`), and the port binds when the key is present (`bind_remote`). Session state comes back from the store in the usual `StoreLoaded` then `HostListed` reconcile. The port's epoch is per run, so a host restart changes `epoch` and every client refetches.

**Rehydration in the page.** The page holds nothing that survives a visit except what it stores itself. The tree gives it no store:
- the token, if it is kept at all, lives where the decision in §3 puts it;
- the host's key, for an "identity changed, pair again" comparison on a later visit. Nothing in the tree stores a key for the page, and `/info` only gives the current one. A page reopened without a code therefore has no earlier key to compare against unless the plan stores it somewhere (localStorage, IndexedDB, or a cookie). The issue does not say where.

So a reopened page rehydrates by `GET /api/v1/info`, then the reads it needs, then the events socket with subscribe-then-snapshot (`docs/remote-client.md` §4.4). The page's own state is UI state, not a read model (§12.6).

## 3. What the pane shows, and what the tree lacks for it

**The list.** `GET /api/v1/projects`, `GET /api/v1/projects/{id}/sessions`, `GET /api/v1/waiting` exist (`src/app.rs:3040-3060`). `SessionView` (`control/src/lib.rs:637-678`) carries `kind`, `liveness`, `card` (a state word), `waiting`, `waiting_reason`, `trust_question`, `name`, `project`, `last_stop_at_ms`, `quiet_secs` and `last_activity_ms`. Grouping by project, waiting first and marked, is possible with what is there.

**The excerpt is not on the wire.** The issue asks for "the one-line excerpt the card shows". In the tree the card's excerpt is computed in the UI: for Claude Code sessions it is `agent_excerpt(conversation)` (`src/ui/cards.rs:558-574`), a function of the parsed transcript; for other kinds it is the caption or the host title (`src/ui/cards.rs:176-180`). `SessionView` has no excerpt field. `docs/remote-client.md` §4.4 and §4.3 name a `last_message` from the `Stop` hook as a metadata field, but `grep` finds no such field in `control/src/lib.rs` or the port. The excerpt would need either an additive wire field computed in the core (CLAUDE.md: logic under `ui/` moves to the core; `docs/lessons.md`, "Keep `ui.rs` dumb"), or one `last-response` fetch per Claude session.

**The session view.** `GET /api/v1/sessions/{id}/last-response` is served (`src/app.rs:1967`, routed at `src/app.rs:3101`), and `GET .../screen?lines=` is served by the same route family (`src/app.rs:1947`). A Claude session's response is `last-response`; anything else uses `screen`. Both are masked before they are cut (the redaction policy in `docs/remote-client.md` §4.3 and `remote_content_route`, `src/app.rs:1947-2024`).

**The reply.** `POST /api/v1/sessions/{id}/send` with `{op, text}` (`src/app.rs:3179-3192`). The page can make `op` a UUID and send it again only on a timeout, which follows §4.5.

**Live updates.** `changed {kind,id}` for `session`, `sessions`, `waiting`, `conversation` and `screen` are emitted by the read-model diff (`src/core/revision.rs`, which the file list in design.md names, and the port's events). A session that starts waiting therefore produces `changed {waiting}` and the page refetches the collection, as the issue asks.

**The "device's scope" in the header.** The page can show the host name from `/info`. It cannot show the device's scope from any endpoint (see §1, Scope). If the plan keeps only a cookie, it needs an endpoint or a field for the caller's scope, or it has to store the scope from the pair reply and accept that a dialog change does not reach the page until a refusal.

**"Revoked".** A revoked device gets `401 unauthorized` with "the token is not known; pair again" (`src/app.rs:3142-3148`), the same code as a device that never paired (`src/app.rs:3129-3135`). The page would need a distinct code. A revoked device's WebSocket is closed (`docs/remote-client.md` §4.5, and the `revoke_closes_the_devices_socket` test in `tests/remote.rs:2891`), and a browser WebSocket cannot read a refused upgrade's body (see Open questions).

## 4. The pairing URL and the native text path

- `PairingInvite` (`client/Sources/SwitchboardProtocol/Info.swift`, the `parse` function at the end of the file) decodes JSON only (`Wire.decode`). A URL form needs a second parser on the Swift side, and the CLI's one-argument `pair` (`client/Sources/switchboard-remote/main.swift`, the `pair` branch, `rest.count == 1`) calls it directly.
- `client/Tests/SwitchboardProtocolTests/FixturesTests.swift:158-164` tests the JSON invite from an inline string, not from `client/Fixtures/`. No invite fixture exists in `client/Fixtures/` (its 28 files are reply bodies such as `pair.json` and `info.json`).
- `tests/remote.rs` drives the port with a raw TLS client (`http()`, `tests/remote.rs:245-297`, `Host: localhost`, pinned through `pinned_client`), and `pair()` (`tests/remote.rs:406`) posts `/pair` with the JSON body. The `crate::adapters` module is public (`src/lib.rs:19`), so a test can reach the page constants.
- The URL's `key` parameter matches `spki_sha256` in `/info` and the `/pair` body. Both are checked by the host already (`src/app.rs:3636`), so the page's own comparison adds a refusal before the code is sent, not a second trust check.
- The refusal logs carry method and path only (`src/app.rs:3754-3772`), so `?code=` is not written to the log by the port. Nothing else in the port logs a request line.

## 5. Dependencies and rules the plan has to respect

- `Cargo.toml` has `tungstenite 0.30`, `rustls` (ring), `rcgen`, `base64`, `qrcode` (no default features), `rustix`, `getrandom`, `sha2`, `uuid`, `serde_json`. No cookie or user-agent crate. A cookie parser or UA name mapping would be hand-written, or added as a dependency with a CI check.
- `web/` does not exist in the tree. The `include_str!` files and a route table would be new.
- CLAUDE.md: the `unsafe` ban and the Linux CI rule apply to any new code. The embedded page is not egui, so the UI-test rule in CLAUDE.md does not cover it; `tests/remote.rs` does.
- CLAUDE.md "Adding a feature" wants an action variant and core test, an effect for the outside world, and README and design.md updates. A static route is adapter-only, and a `CHANGELOG.md` line under Unreleased is required for a user-visible change.
- The trust-boundary paragraph in `docs/design.md` (around line 219-262) would need the page's sentence, as §5 of `docs/remote-client.md` already says.
- CLAUDE.md asks for a spike directory before building on a new risky mechanism. A browser's handling of a self-signed certificate, cookies and WebSockets on it is one, which is why it is listed below as a run-or-source question.
- Redaction is unaffected: the page reads through the same content routes. Static files carry no session data and do not need the redaction path.

## 6. Which decisions the tree already settles, and which it leaves to the plan

Settled by the tree (the plan should cite, not re-decide):
- Auth for every non-static read is the bearer, checked in `authenticate` (`src/app.rs:3127`), and the token is stored only as its hash (`src/core/remote.rs:166`).
- `/pair` already refuses a key that is not the host's (`src/app.rs:3636`) before the code is checked.
- A WebSocket subscription needs `Upgrade` and a bearer (`src/app.rs:3036-3037`, `src/app.rs:2723`).
- Pools: 4 unauthenticated, 12 paired, rate windows only on `/pair` and `/info`.

Left to the plan (design choices, not open questions here): the cookie vs. subprotocol choice for the token; the click-through trust step; one QR or two; the static path table and headers; which pool a page load counts against; the device name's source; the excerpt field; the scope endpoint; the distinct revoked code; the LAN requirement for opening the pairing window; where the page keeps a pinned key.

## 7. What I did not check

- I did not run the host, the tests, or any browser. Nothing here is measured.
- I did not read `src/adapters/remote.rs` 560–765 and 880–1088 beyond their names (the subscriber loop, `serve`'s pools in detail, the tests), nor `src/app.rs` beyond the remote regions I listed, nor `src/core/revision.rs`, the watch set, the content cache, or the worker.
- I did not read the Swift transport (`SwitchboardTransport`), the pinning delegate, `client/Fixtures/*.json` contents, or the visionOS app.
- I did not read `spikes/22-remote-port` beyond its file list and the design's summary of its results.
- I did not confirm the event kinds from the diff code or the tests; `docs/design.md` and `tests/remote.rs` names say they exist.
- I did not check `scripts/bundle.sh`, the README's dev-aid list, or the CHANGELOG format.

## Open questions

- [source] Whether Chrome, Safari on macOS, and Safari on iOS/iPadOS let a user click through the warning for a self-signed certificate with no subject alternative name, a common name only, and a far-future expiry, when the host is an IP address.
- [source] Whether the click-through exception covers `wss://` and `fetch` to the same host and port, and whether it persists after the browser is closed and reopened, per browser.
- [source] Whether any browser lets a page pin a certificate's SPKI hash for `fetch` or `WebSocket` on an origin (the issue says none does).
- [source] Whether the WHATWG WebSocket constructor can send custom headers (the issue says it cannot; the constructor takes only a URL and protocols).
- [source] Whether any published specification (RFC 6455, WHATWG Fetch or Cookies) forbids a token in a URL, or whether the prohibition is this repo's own rule in `docs/remote-client.md` §5.
- [source] Whether `Set-Cookie` with `HttpOnly; Secure; SameSite=Strict` set from an https origin on an IP-address host is stored after a certificate click-through, in each browser.
- [source] Whether `SameSite=Strict` cookies are sent with a WebSocket handshake and with `fetch` from the same origin, and whether a page on another origin on the same host (a different port, or a rebinding name) makes a credentialed request that carries them.
- [source] Whether browsers send `Origin` and `Sec-Fetch-Site` on WebSocket upgrades, and which values a cross-port same-site request carries, since the port would need an `Origin` check if a cookie carries the token.
- [source] Whether a 101 without `Sec-WebSocket-Protocol` is accepted by a browser that offered a protocol, which decides whether a subprotocol-carried token works.
- [source] What a WebSocket `error` event exposes to script on a refused upgrade (status, body), since the revoked state needs the body.
- [source] Whether `history.replaceState` removes the code from the back-forward cache, the session restore data and the Referer of later requests, in Safari and Chrome.
- [source] Whether `connect-src 'self'` permits the same origin's `wss:` URL in Safari and Chrome, and whether `script-src 'self'` with no inline script is enough for the page's load.
- [source] How many connections a browser opens for one page load and its WebSocket, and whether Safari on iOS/iPadOS and Chrome send `/favicon.ico` or `apple-touch-icon.png` unprompted, since four unauthenticated slots are shared by them.
- [source] Whether a browser retries or reports a failed page load when the host closes an accepted connection past the pool (the accept-time drop at `src/adapters/remote.rs:701`).
- [source] Whether iOS and iPadOS Safari's user agent says `iPhone` or `iPad` for the device, or reports a desktop `Macintosh` form on iPad, which decides how a `Safari on iPhone` name can be derived.
- [source] Whether Chrome or Safari's Local Network Access permission prompts when a page served from a LAN address fetches from that same address.
- [run] The browser's real connection count and request order for one page load, against the port with a stub page, to see whether the 4 unauthenticated slots are reached before pairing.
- [run] Whether `history.replaceState` before the first `fetch`, a cookie set through a click-through, and a WebSocket upgrade on a self-signed certificate all work end to end in one browser on this Mac.
- [run] Whether `switchboard-remote pair` with a URL argument, once the parser exists, pairs against a host with `lan` on, and with `lan` off (the loopback case in §1).
- [owner] On a phone, whether the iOS Camera opens a QR's https URL in Safari directly, and what the warning looks like there.
- [owner] On the owner's network, whether the address `local_address()` picks is the one the phone can reach (several interfaces, a VPN).
- [owner] Whether a click-through the owner makes once is acceptable as the only trust step on a home network for a device with `control` scope.
