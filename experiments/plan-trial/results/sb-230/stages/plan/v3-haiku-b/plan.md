# Plan: #230, a browser control page on the remote port

Checked against worktree `sb-230-stage-plan-v3-haiku-b` at `7fe8949`. The facts file is cited as "facts", by its heading.

## Where this departs from the issue

- **The card excerpt is not a record field.** The issue, and `docs/remote-client.md` §4.3 (line 207), say the excerpt is `last_message` from the `Stop` hook on the session record. The tree does not keep it. The hook's `last_message` rides on `EventKind::Stopped` (`src/ports/events.rs:24`), and the core discards it (`src/core/events.rs:132`, `Stopped { pending, .. }`). `SessionView` (`control/src/lib.rs`) has no excerpt field. The headset gets its card line from `last-response` per session (`client/Sources/SwitchboardUI/BrowserView.swift:149-163`). The page does the same and re-fetches only when a session's `last_stop_at_ms` moves. This commit corrects spec line 207. The facts file's "last_message exists on the session record" is wrong for the same reason.
- **The device's scope is on no endpoint after pairing.** It appears only in the `/pair` reply (`src/app.rs:3606`, `remote_pair`). The plan adds an authenticated `GET /api/v1/device`.
- **"Revoked" cannot be told apart on the wire.** `authenticate` (`src/app.rs:3127`) answers the same `unauthorized` for a revoked token and a token never issued. The page says "no longer paired with this Mac" for either, once it has paired. No new refusal code.
- **The code may survive in history.** `history.replaceState` changes the current entry only (facts, Unresolved 6). The entry the browser made when the camera opened the URL may keep the code. The plan does not promise otherwise. The code is single-use, the window closes on the first pairing, and the dialog says the link is spent once paired.
- **The key check proves a claim, not a genuine page.** The page compares the URL's key with `/info`, which shows only that the host claims that key. Spec §5 says a relayed pairing "fails even if the code reached it". That holds for the native client, which pins the key. It does not hold for a browser page served by an impostor. §5 is amended.
- **Live updates depend on Safari.** The issue says the page refreshes "on `changed`". Whether a WebSocket opened through a click-through certificate works on iOS is unresolved (facts, Unresolved 1). The default is polling, and the socket is an upgrade the header shows as "live". Content `changed` events also need `watch` frames (spec §4.4), which the default does not use.

## Decisions

### D1. Trust in the self-signed certificate

**Pick:** the click-through is the trust step. The host does not offer a profile to install.

**Why:** a profile installs a manually trusted root, and what iOS requires of one is unresolved (facts, Open question on manual trust). The click-through needs nothing on the device.

**What it buys:** the native pin is unchanged. The page's key compare stops a genuine page from sending the code to a second genuine host (a rotated key, or another installation on that port). It does not stop an impostor page. The paired-devices dialog and the pairing window both say the warning is the trust step, and that an address change or a renewal can bring it back.

**What it rests on:** whether a click-through covers `fetch` and `wss` in current Safari is unresolved (facts, Unresolved 1). The default needs `fetch` to work after the click-through, and the plan treats that as the first thing the phone run checks (owner item A). If `fetch` is refused, the fallback is the profile install, which is the owner's call. The socket is not needed for the default.

**Certificate contents (the cheap, settled part):**
- Reissued over the same key, through `rcgen::KeyPair::from_pem` (`src/adapters/remote.rs:146`). The SPKI hash is of the key (`:247`, `sha256_hex` `:292`), so the pin stays. Facts' "can the certificate be reissued" question is answered by the tree.
- Subject alternative names: loopback (`127.0.0.1`, `::1`) and the current LAN address as an IP SAN. The default uses an IP SAN and no `.local` name. Whether Safari accepts an IP SAN is unresolved (facts, Unresolved 2). A `.local` DNS SAN is an enhancement behind owner item B.
- Extended key usage `serverAuth`.
- Validity 800 days. Apple's rule is 825 days or fewer for certificates issued after 1 July 2019 (facts, Settled: Apple's TLS server certificate rules). The page does not say whether a manually trusted leaf is exempt (Unresolved 3), so the default meets the rule either way.
- Renewed at bind time when the SAN set differs from the wanted set, or when the certificate is 730 days old or has no `issued` time.

### D2. Where the token travels

**Pick:** the page keeps the token in `localStorage` and sends it as `Authorization: Bearer` on every `fetch`, which the host already accepts. The WebSocket carries it in `Sec-WebSocket-Protocol: switchboard.v1, bearer.<token>`, and the host echoes `switchboard.v1`. No cookie.

**Why a cookie is not the default:** whether a `Secure` cookie is stored for an IP host, and whether `SameSite=Strict` cookies go on a same-origin WebSocket handshake in Safari, are both unresolved (facts, Unresolved 5). A default that needs a cookie would fail on the phone if the answer is no. Cookies are also scoped by host, not port (facts, Settled: RFC 6265 quoted), so a cookie would reach other HTTPS services on the same Mac. A cookie name per `server_id` helps two Switchboard instances but not those services.

**Why no Origin check:** a bearer header and a subprotocol value are not ambient. A cross-site page cannot set either, so it has no credential to forge a request or open a WebSocket with. Cross-site request forgery and cross-site WebSocket hijacking need an ambient credential, and this design has none. So the host reads no `Origin` header. If a cookie is ever added, this decision is reopened.

**The socket:** RFC 6455 §4.2.2 says the server replies with one subprotocol it accepts, and §4.1 says the client fails on one it did not offer (facts, Settled: WebSocket subprotocol). The host echoes `switchboard.v1` and no other value. A base64url token is a legal token (facts, Settled). Whether browsers accept the echo in Safari is not settled, so if the socket fails the page falls back to polling.

**Trade:** any script on the origin can read the token. The page's Content Security Policy (no inline script, no external script) is the control. The owner accepts this, and can ask for the cookie path later.

### D3. One QR, carrying a URL

**Pick:** the QR and "Copy invite" carry the URL only. `PairingInvite.parse` (Swift) and `switchboard-remote pair` accept the URL, and still accept the old JSON, because a copied JSON invite in someone's clipboard should still pair. No code writes JSON any more.

**Address:** one address, the one `local_address()` returns (`src/adapters/remote.rs:962`). A second `.local` QR waits on owner item B.

### D4. Static paths on a port that serves `/api/v1/*`

**Pick:** exactly two `GET` paths are served before pairing, and nothing else.
- `GET /` answers `text/html; charset=utf-8`, with CSS inline in a `<style>` block and `<link rel="icon" href="data:,">` so the browser does not ask for `/favicon.ico`.
- `GET /app.js` answers `text/javascript; charset=utf-8`.
- Both carry: `Content-Security-Policy: default-src 'none'; script-src 'self'; style-src 'unsafe-inline'; img-src data:; connect-src 'self' wss:; base-uri 'none'; form-action 'none'; frame-ancestors 'none'`, `Cache-Control: no-store`, `X-Content-Type-Options: nosniff`, `Referrer-Policy: no-referrer`, `Connection: close`, and `Content-Length`.
- No other path is static. `/index.html`, `/app.css` and `/favicon.ico` stay on the existing allowlist's 404.

**Where they are answered:** on the connection thread, not the frame. The bytes need no state, and a stalled frame must not blank the page. The reply closes the connection, so the unauthenticated-pool slot frees at once. A request for a static path is not logged, which keeps `?code=` out of the logs as `refuse` already does (`src/app.rs:3754`).

**Pool and rate:** a page load is two requests plus `/info` and `/pair` (facts, Unresolved 8). The unauthenticated pool stays at four (`MAX_UNAUTHENTICATED`, `src/adapters/remote.rs:567`). Static paths get no rate window, since the pool bounds them. If the phone run shows that a page load drops connections at accept, the enhancement is a separate pool for static paths, built behind this default.

**No code and no token:** the page shows the instruction "open the pairing window on the Mac and scan its code". No code-entry form is built. A typed form would need the key typed too, and the headset's typed-code path already covers a device without a camera.

**Spec bounds (§4.1) are unchanged** except for one sentence about static pages closing their connection.

### D5. Where the page lives, and how it is tested

**Pick:** `web/index.html` and `web/app.js`, embedded with `include_str!` in a new adapter module `src/adapters/remote_web.rs`, which `src/adapters/mod.rs` declares. No build step, no bundler, no dependency. The page's protocol logic has no CI test, because the issue rules out a JS harness dependency. The tests cover the endpoints the page calls, and the phone run covers the page itself. The design status says so.

### D6. The device record

**Pick:** the page names the device from its user agent (`Safari on iPhone`) and sends it as `device_name`. The host already cuts it to 64 characters (`new_device` in `src/core/remote.rs`). No `kind` field.

**Why:** the name is enough for the acceptance steps. A `kind` would be an additive `#[serde(default)]` field in `devices.json`, which has no version (`load_devices` in `src/adapters/remote.rs`), so it costs nothing to add later if the owner wants it.

## The record

- **No new record kind. No `model.rs` change.** `SCHEMA_VERSION` stays `16` (`src/core/model.rs:12`). `VIEWS_SCHEMA_VERSION` stays `7` (`src/core/model.rs:612`). Neither is bumped.
- **`identity.json`** (`Identity`, `src/ports/remote.rs:26-42`) gains two fields, both `#[serde(default)]`:
  - `pub addresses: Vec<IpAddr>`, the IP SANs the certificate carries. Empty for an identity made before this field.
  - `pub issued: Option<SystemTime>`, when the certificate was made. `None` marks a certificate that is due for renewal.
  `IDENTITY_SCHEMA_VERSION` stays `1` (`src/ports/remote.rs:17`). `Identity` has no `deny_unknown_fields`, and an older build reads the file with the new fields ignored. If the implementer finds a reason to version the file, the new value is the constant at implementation time plus one, with a step in the identity loader.
- **`devices.json`:** unchanged. A browser is one `PairedDevice`.
- **Browser state (not a record):** `localStorage` under `switchboard.pairing.<server_id>` holds `{token, device_id, server_name, spki_sha256, scope}`. The code is never stored.

## Signals (the wire)

- **`GET /` and `GET /app.js`:** the static answers in D4. Served before the bearer check, on the connection thread.
- **`GET /api/v1/device`:** new, authenticated. Body `{device_id, name, scope, server_name}` in the envelope, `authenticated: true`. `authenticate` is unchanged. Added to `serve_remote` (`src/app.rs:3017`) after the `authenticate` call, beside the `events` arm.
- **`GET /api/v1/events` upgrade:** the token may come from `Sec-WebSocket-Protocol`, but only on an upgrade request and only when there is no `Authorization` header. The 101 echoes `switchboard.v1` when offered.
- **`POST /api/v1/pair`:** unchanged. The page sends `{code, spki_sha256, device_name}`.
- **`/info` reads:** the page reads `/info` on load, after pairing, and after an `unauthorized` reply. Never on a timer, because `/info` is limited to 60 a minute per listener (`INFO_PER_MINUTE`, `src/adapters/remote.rs`).
- **Events:** no new kinds. `changed` and `removed` keep their meaning.

**Adapter changes (`src/adapters/remote.rs`):**
- `read_request` (`:444`) reads `Sec-WebSocket-Protocol` into a new `subprotocols: Vec<String>` field on `RemoteRequest` (`:360`). It takes the bearer from a `bearer.` entry only on an upgrade, when `Authorization` is absent. The grep confirms `RemoteRequest` is constructed only at `:506`, and no test builds one.
- `respond_upgrade` (`:519`) takes the echo as a parameter. The caller computes it before the request moves into `RemoteIncoming`.
- A new `respond_page` writes the static headers. `respond` (`:530`) is unchanged.
- `RemoteReply` (`:389`) is **unchanged**, so the ~15 literal sites in `src/app.rs` do not move.

## Actions and effects

- **`AppAction::IdentityRenewed(Identity)`** (`src/core/action.rs`, additive). The core accepts it only when `spki_sha256` matches the identity it holds. It then sets the identity and pushes `Effect::SaveIdentity`, unless read-only, the same as `update_identity` in `src/core/remote.rs`. Devices are untouched. A key that differs is a rotation, which is not this action.
- **Renewal, in the adapter:** `certificate_wants_renewal(identity, lan: Option<IpAddr>, now: SystemTime) -> bool` (pure) and `reissue_certificate(identity, key_pem, addresses, now) -> Result<Identity, String>`. Both live in `src/adapters/remote.rs`. `make_identity` (`:95`) and the reissue share one `certificate_for` helper, so they cannot diverge.
- **`bind_remote` (`src/app.rs:2936`):** before `remote::tls_config` (`:2957`), when renewal is wanted, it reissues over the Keychain key, dispatches `IdentityRenewed`, and uses the new identity for TLS. A failed reissue logs and binds the old certificate, as today.
- **Certificate parameters:** `CertificateParams::new` with the IP addresses as strings, or `SanType::IpAddress` directly (check at implementation). `ExtendedKeyUsagePurpose::ServerAuth`. `not_before` one hour back, `not_after` 800 days out. The `time` crate may need to be a direct dependency at the version `Cargo.lock` already holds (check the rcgen 0.14 source, `Cargo.toml:62`).
- **`pairing_url`** (new, `src/adapters/remote.rs`, pure): `pairing_url(addr: SocketAddr, server_id: Uuid, spki_sha256: &str, code: &str) -> String` gives `https://{addr}/?code={code}&key={spki}&id={server_id}`. `SocketAddr` display brackets an IPv6 address.
- **`remote_web::page`** (new, `src/adapters/remote_web.rs`, pure): `page(method, path) -> Option<Page>`, where `Page` holds the content type and body. Called in `connection()` before the rate window.
- No new `Effect`, no new port, no `Clock` use outside the adapter.

## The UI

**The pairing window (`src/ui/remote.rs`):**
- The QR payload (around `:160`) becomes `remote::pairing_url(SocketAddr::new(ip, port), identity.server_id, &identity.spki_sha256, &window.code)`. The address line uses the same `SocketAddr`.
- The URL is shown in monospace under the QR, so it can be selected.
- "Copy invite" (`:201`) copies the URL.
- The meta line says: "Scan with the phone's camera. Your browser will warn about the certificate; accept it for this address. The link pairs once, and is spent after." The key short form stays, so the owner can compare it on the device.

**The paired-devices dialog:** a sentence under the list: "A browser warns about the certificate on first use, and again if the address changes or the certificate is renewed."

**The page (`web/index.html`, `web/app.js`):**
- **Start:** the first statement reads the query, then calls `history.replaceState(null, "", "/")`. Nothing else runs before it.
- **Unpaired** (no query, no stored marker): the instruction text from D4.
- **Pairing** (query present): fetch `/info`, compare `spki_sha256` and `server_id` with the query's `key` and `id`. Only on a match, post `/pair` with `{code, spki_sha256: key, device_name}`. Store the result in `localStorage`. Errors map as follows: `wrong_code` says the code did not match; `pairing_closed` says the window is closed on the Mac; `wrong_key` says the page refused to send the code.
- **Ready:** header shows device name, scope word, server name, and "live" or "updating every 3 s". Sessions grouped by project, waiting ones first with a "Waiting on you" mark. Each row shows kind, the card's word (`card`), and for Claude sessions the first line of `last-response`.
- **Session:** Claude sessions show `last-response`; others show `screen?lines=40`. The reply box appears only under `control` scope. Send posts `sessions/{id}/send` with `{op, text}`. `op` is `crypto.randomUUID()` when `window.isSecureContext`, else an id from `crypto.getRandomValues` (facts, Unresolved 4). On a timeout the page follows §4.5 and reads `ops/{op}`.
- **Identity changed:** if `/info` now shows a different `spki_sha256` or `server_id` from the marker, the page says "identity changed, pair again", drops the token, and sends nothing.
- **Revoked:** an `unauthorized` reply after pairing re-reads `/info` once. If the key still matches, the page says "this device is no longer paired with this Mac" and drops the token.
- **Revisions:** keep the `revision` per collection, apply a reply only when it is newer, and refetch in full on an `epoch` change.
- **Polling:** list every 3 s, open session every 2 s, `/device` every 30 s.
- **Socket:** `new WebSocket("wss://" + host + "/api/v1/events", ["switchboard.v1", "bearer." + token])`. On `hello` take the revision. On `changed`, refetch that collection. On `resync_required` or close, refetch and keep polling. The header shows "live" only while the socket is open.

## Tests

**Adapter (`src/adapters/remote.rs`, `src/adapters/remote_web.rs`):**
- `read_request` takes the bearer from the subprotocol only on an upgrade, and `Authorization` wins when both are present.
- `respond_upgrade` echoes `switchboard.v1` only when it was offered.
- `pairing_url` produces the shape above, with IPv6 bracketed; the test parses the query back.
- `reissue_certificate` keeps `spki_sha256`, carries the IP SAN (the four octets follow tag `0x87` in the DER), and gives `not_after` at most 825 days after `not_before`.
- `certificate_wants_renewal` over a small truth table: SAN missing, SAN present and young, SAN present and 730 days old, `issued` absent.
- `page("/", "GET")` and `page("/app.js", "GET")` return the content types and the CSP; `page("/index.html", "GET")` returns `None`.

**Core (`src/core/tests.rs`):**
- `IdentityRenewed` with the same SPKI sets the identity and emits `SaveIdentity`.
- `IdentityRenewed` with another SPKI is ignored.
- Read-only: no effect.

**Integration (`tests/remote.rs`):**
- `the_page_is_served_without_pairing_with_its_headers`: `GET /` and `GET /app.js` answer 200 with exactly the headers in D4. A new helper, `fetch_with_headers`, returns the response headers, because the existing `http` helper (`:245`) keeps none.
- `the_allowlist_still_refuses_other_static_paths`: `/index.html`, `/app.css` and `/favicon.ico` answer 404 `not_found`. Added to the neighbourhood of `the_allowlist_refuses_everything_else_with_no_side_effect` (`:682`).
- `a_page_load_is_answered_off_the_frame`: six sequential `GET /` and `GET /app.js` requests are answered while the app is not pumped, so the answer cannot depend on the frame. Each closes its connection.
- `the_script_calls_only_served_paths`: scans `web/app.js` (via `include_str!`) for `/api/v1/` literals, substitutes `${…}` placeholders with seeded ids, and calls each with a paired token. None may be refused with "not available on this port" or "no such endpoint".
- `a_browser_pairs_from_the_url_as_the_page_does`: opens the window, builds the URL with `pairing_url`, parses its query, reads `/info`, compares the key and id, posts `/pair` with a body shaped as the page sends it, reads `/device` (the name `Safari on iPhone`, the granted scope), revokes, and sees `/device` answer 401 `unauthorized`. Uses the helpers beside `pair` (`:406`) and the revoke test (`:658`).
- `a_socket_takes_its_token_from_the_subprotocol`: an upgrade with `Sec-WebSocket-Protocol: switchboard.v1, bearer.<token>` gets 101 with the echo. A wrong token gets 401 before any upgrade. No token gets 401. A new helper beside `subscribe_at` (`:1960`) sends the header.
- `the_client_fixtures_decode_on_both_sides` (`:1829`) also covers `client/Fixtures/pairing_url.txt`, which `write_fixtures` (`:1587`) writes from `pairing_url`.

**UI (`tests/ui.rs`):** with the pairing window open, the copied text from "Copy invite" equals the `pairing_url` the adapter builds. Check how kittest exposes copied text at implementation.

**Swift (`client/Tests/SwitchboardProtocolTests`):** `PairingInvite.parse` accepts the URL fixture, the legacy JSON, and an IPv6 URL, which becomes `[addr]:port`. Run `swift test` in `client/` on macOS. If the Swift toolchain is not present in the implementer's environment, say so in the final report.

**Gate:** `cargo test --locked --workspace`, `cargo clippy --locked --workspace --all-targets -- -D warnings`, `cargo fmt --all`. The new code is portable, so CI's Linux clippy needs nothing extra.

**Live check (the app runs at every step):** with `SWITCHBOARD_DATA_DIR` and `SWITCHBOARD_TMUX_SOCKET` pointing at test locations, run `remote-listen lan <port>` and `pairing <code>` through `SWITCHBOARD_SCRIPT`. Then `curl -k -sD- https://<lan-ip>:<port>/` for the headers, `curl -k https://<lan-ip>:<port>/api/v1/info` for the key, and `switchboard-remote pair '<url>'` against a test `SWITCHBOARD_REMOTE_DIR`.

**Not testable here, and owned by the owner:** the page in a real browser, and the phone run (see Acceptance).

## Docs

- **`docs/remote-client.md`:**
  - §4.1: static pages are answered on the connection thread and close their connection. The pool bounds are unchanged.
  - §4.3: a `GET /api/v1/device` row. A "static pages" note listing the two paths. Line 207 corrected: the excerpt is the last response, not a record field.
  - §5: the QR is a URL. The sentence that a relayed pairing fails is limited to the native client. Browser token carriers are the bearer header and the WebSocket subprotocol. "Never in a URL" stays true. "Require pairing before anything but `/info`, `/pair`" gains the two page files.
- **`docs/design.md`:** the "Remote client" section's "Built so far" gets a bullet for the browser page. "Known gaps" gains three lines: no phone run recorded; the socket depends on Safari; the page's logic has no CI test.
- **`README.md`:**
  - Layout: a row for `web/` and one for `src/adapters/remote_web.rs`.
  - The `identity.json` description (around `:78-83`) gains `addresses` and `issued`.
  - Development (around `:265-278`): "Copy invite" copies the URL, the `switchboard-remote pair` example takes it, and the page is at `https://<address>:<port>/`.
- **`CHANGELOG.md`:** one line under `## [Unreleased]`: the remote port serves a browser page; the pairing QR is a link; the page lists sessions and sends replies.

## Build order

Each step leaves the app running.

1. Identity fields, `certificate_for`, renewal, `IdentityRenewed`, core and adapter tests.
2. `read_request` subprotocols and bearer fallback, `respond_upgrade` echo, tests.
3. `remote_web.rs` with minimal placeholder pages, the connection-thread arm, `respond_page`, the static tests.
4. `GET /api/v1/device`, its arm in `serve_remote`, tests.
5. `pairing_url`, the pairing window, the CLI and Swift parse, the fixture, tests.
6. The real `web/index.html` and `web/app.js`, and the script test.
7. Docs, README, CHANGELOG.
8. Gate, then the live check.

## Acceptance

The issue's steps, each of which the polling path satisfies. The socket only shortens the wait.

1. Open the pairing window; the QR is a URL. (UI test.)
2. Scan with the phone, accept the warning, see the sessions. (Owner, phone. Needs `fetch` after the click-through, owner item A.)
3. A session that starts waiting moves to the top within 3 seconds with no reload. (Owner, phone.)
4. A reply from the phone arrives. (Owner, phone. The `send` path is covered by the existing control tests.)
5. Revoke the phone in the dialog; the page says so on its next request. (Test: revoke, then 401. Owner, phone.)

## Left to the owner

- **Item A (phone run):** whether `fetch` and `wss` work after the click-through, with the current and the renewed certificate.
- **Item B:** a `.local` DNS SAN. An enhancement; the default does not need it.
- **Item C and the history question:** if the phone run shows the code persisting, the plan still holds (single-use, expires). Nothing to change.
- **Pool:** if the phone run shows drops at accept, a separate static pool, built later.
- **Cookie:** if the owner wants the token out of script reach, the cookie carrier is a later change, and needs items 5 and D favourable.
- **Excerpt:** if fetching `last-response` for many sessions is slow, an `excerpt` field on `SessionView`, additive.
- **`kind` on the device record, and a code-entry form:** only if the owner asks.
- **Browser tab limit:** a tab can hold several keep-alive connections, each a paired-pool slot (`MAX_PAIRED`, 12). The phone run should check that the headset is not refused `full` while the page is open. Not mitigated in this cut.

## Contract additions

- `src/core/action.rs`: `AppAction::IdentityRenewed(Identity)`.
- `src/ports/remote.rs`: `Identity.addresses: Vec<IpAddr>` and `Identity.issued: Option<SystemTime>`, both `#[serde(default)]`.
- `src/adapters/remote.rs`: `RemoteRequest.subprotocols: Vec<String>`; `pairing_url`, `reissue_certificate`, `certificate_wants_renewal`; the echo parameter on the private `respond_upgrade`.
- `src/adapters/remote_web.rs` (new): `page`, `Page`.
- Wire: `GET /api/v1/device` (new, authenticated); `GET /` and `GET /app.js` (new, before pairing, not under `/api/v1/`).
- Swift: `PairingInvite.parse` accepts the URL form; `client/Fixtures/pairing_url.txt`.
- Spec: §4.3 row for `/device`; line 207 corrected.

## Commit message

`Serve a browser control page on the remote port and pair it from a URL`
