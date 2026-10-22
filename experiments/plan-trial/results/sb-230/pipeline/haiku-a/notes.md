# Issue #230 — a web control surface on the remote port: investigation notes

Worktree: `/Users/sbgragent/.dispatch/worktrees/1ea22e2f` (branch `dispatch/230-a-web-control-surface-the-remote-port-se`, HEAD `7fe8949`, clean).
Read: `CLAUDE.md` (in the prompt), `docs/remote-client.md` §0–§5 and §10 heading, `docs/design.md` (trust boundary, remote status, known gaps, open questions), `docs/lessons.md`. No code was changed, no build or test was run, and no device was tested. Everything below is from reading the source; items marked **verify** are behaviour of browsers I have not checked.

---

## 1. Where the code the issue touches is

| Concern | Location (this worktree) |
|---|---|
| Listener, accept loop, unauthenticated pool drop | `src/adapters/remote.rs` 676–735 (accept loop; `drop(stream)` at 704 when the 4-slot pool is full; unauthenticated++ before spawn) |
| Pool and deadline constants | `src/adapters/remote.rs` 567 (`MAX_UNAUTHENTICATED = 4`), 568 (`MAX_PAIRED = 12`), 570–577 (5 s handshake, 30 s idle, 10 s write, 60 s answer) |
| Rate windows (`/pair` 6/min, `/info` 60/min) | `src/adapters/remote.rs` 583–584; `Limits::limited` 631–640 returns **false for every other path** |
| Request parsing (only `Authorization`, `Upgrade`, `Sec-WebSocket-Key`, `Content-Length` are kept) | `src/adapters/remote.rs` 444–516; `RemoteRequest` 360–372 |
| Response writer (**hard-codes `Content-Type: application/json`**, `Connection: keep-alive`, no extra headers) | `src/adapters/remote.rs` 530–553 |
| `RemoteReply` (status, body, `authenticated`; no headers field) | `src/adapters/remote.rs` 389–395; built as a literal at 2 sites in the adapter and 28 in `app.rs` |
| WebSocket 101 (echoes only `Accept`; no `Sec-WebSocket-Protocol`) | `src/adapters/remote.rs` 519–526 |
| Per-connection flow: rate check → hand to frame (`try_send`, 503 if full) → wait for reply → move to paired pool on first authenticated reply | `src/adapters/remote.rs` 770–876 (`limited` at 788, `try_send` at 812) |
| Subscriber (WebSocket) loop, non-blocking | `src/adapters/remote.rs` 878–927 |
| TLS config: no ALPN set, so browsers stay on HTTP/1.1 | `src/adapters/remote.rs` 144–158 |
| Identity certificate: **no SANs** (`CertificateParams::new(Vec::new())`), validity left at rcgen default (the code comment says year 4096) | `src/adapters/remote.rs` 95–128, esp. 102 and 91–94 |
| Pinned client (native clients and the Rust tests) | `src/adapters/remote.rs` 168–246 |
| `local_address()` (UDP route guess, used for the QR address) | `src/adapters/remote.rs` 962 |
| Frame drain: 8 per pass, then `serve_remote` | `src/app.rs` 1572 (`serve_pending`), 1584–1596 |
| `serve_remote`: the `/api/v1/` prefix check (non-api paths get 404 `not_found` **on the frame**), then the route table | `src/app.rs` 3017–3025 (prefix at 3023); route table 3026–3110 |
| `authenticate`: bearer only; `DeviceSeen` on each call; 401 `unauthorized` "the token is not known; pair again" | `src/app.rs` 3127–3157 |
| Revocation closes the device's WebSocket (drops its queue) | `src/app.rs` 486–497 |
| `remote_subscribe` (upgrade, `hello`, subscriber push) | `src/app.rs` 2723–2764 |
| `remote_info`: capabilities list, `pairing_open`, `limits`; no page-related field | `src/app.rs` 3568–3603 |
| `remote_pair`: `wrong_key` and `wrong_code` both dispatch `PairingFailed` (shared counter); success returns `token` in the JSON body with `authenticated: false` | `src/app.rs` 3606–3670 (token at 3661; wrong-key branch counts at ~3625) |
| `envelope` / `envelope_at` (`epoch`, `revision`, `body`) | `src/app.rs` 3717–3741 |
| `refuse` (logs method, path, code only: not the query, not the body) | `src/app.rs` 3754–3768 |
| Pairing window state: `OpenPairing` has **no `lan` check**; single-use code; 3 failures close | `src/core/remote.rs` 46–52 (open), 53–60 (fail), 61–67 (Paired closes window); `pairing_at` 152; `new_device` 166–175 |
| `devices.json` record (`PairedDevice`: id, name, scope, token_sha256, created, last_seen) | `src/ports/remote.rs` 96–110; `Identity` 26–47; `RemoteStore` 111 |
| `last_seen` save throttle (300 s) | `src/core/remote.rs` 18 |
| Pairing window UI: QR payload JSON (`server_id`, `spki_sha256`, `code`, `addresses`), "Copy invite" copies that same JSON; the address comes from `local_address()` **whether or not `lan` is on** | `src/ui/remote.rs` 141–200 (address 157, payload 160–166, Copy invite 201); `qr_texture` near 280–300; scope radios 215–217; `SetDeviceScope` 251 |
| Port bind: loopback unless `identity.listen.lan`; `SWITCHBOARD_REMOTE_PORT` override | `src/app.rs` 2936–2990 (`let ip = if identity.listen.lan` at 2962) |
| Spec/UI test for `remote-devices` dialog | `tests/ui.rs` 7420–7505 (asserts code shape, not payload) |
| Rust test client helpers (`http` discards response headers; `call`, `get`, `pair`) | `tests/remote.rs` 245–297 (`http`), 299–335, 406–423 (`pair`) |
| Allowlist and info tests (`/other` → 404 `not_found`; `/info` ok; `info.revision` 0) | `tests/remote.rs` 425–475 (info test), 681–724 (allowlist) |
| Pin-mismatch test (no request reaches the app) | `tests/remote.rs` 1394–1403 |
| Pair rate-limit test (`[403×6, 429]`) | `tests/remote.rs` 1405–1419 |
| Client fixtures written and read on both sides (`client/Fixtures/*.json`, `info.json`, `pair.json`, …) | `tests/remote.rs` 1587 (`write_fixtures`), 1828–1840 (decode check); files in `client/Fixtures/` |
| Swift invite parse (JSON only) and its test | `client/Sources/SwitchboardProtocol/Info.swift` 88–117 (`PairingInvite`, `parse` at 113–116); `client/Tests/SwitchboardProtocolTests/FixturesTests.swift` 157–166 |
| Swift `PairRequest` / `PairReply` (reply expects `token`) | `client/Sources/SwitchboardProtocol/Info.swift` 48–78 |
| CLI pairing: one arg = invite text (JSON), three args = `<host:port> <code> <spki>` | `client/Sources/switchboard-remote/main.swift` 107–126 (usage line 12–13) |
| Headset pairing via Bonjour + typed code (`ConnectionManager.pair(_:code:)`) | `client/Sources/SwitchboardClientCore/ConnectionManager.swift` 471–487 |
| Existing `src/` embed precedent (`include_bytes!` for icon and fonts) | `src/main.rs:167`, `src/ui/theme.rs:203–211`. No `include_str!` in the tree yet. |
| Dependencies: `rustls 0.23` (ring), `rcgen 0.14` (source at `~/.cargo/registry/.../rcgen-0.14.10`, `SanType::IpAddress` at `src/lib.rs:313`), `tungstenite 0.30` | `Cargo.toml` 61, 62, 66 |
| Spec sections the plan must amend | `docs/remote-client.md` §4.1 (bounds, pools), §4.3 (endpoint table, 157–219), §5 (pairing, 302–319); `docs/design.md` remote status (3107–3585) and the trust-boundary paragraph (219–265) |
| README: Layout rows for `adapters/remote.rs`, `ui/remote.rs`, `client/`; dev aids for `SWITCHBOARD_REMOTE_PORT` and `remote-listen` (lines 267–278, 318–319, 383, 423, 437) | `README.md` |

Note on the three-id rule and the layering: nothing in the page needs a new core action. The page only consumes existing REST and events. The only frame-side change the plan needs is an additive read (see §3.3 below). The static bytes can be answered entirely in the adapter, which keeps the frame out of it (§4.1 rule: "the frame does core state only").

---

## 2. The on-disk record, then rehydration, then the pane

**On disk, unchanged by this issue.** A browser is a `PairedDevice` in `<data dir>/devices.json` (`src/ports/remote.rs` 96–110): `id`, `name` (≤64 chars, trimmed in `new_device`, `src/core/remote.rs` 166–175), `scope` (taken from the open pairing window), `token_sha256`, `created`, `last_seen` (saved at most every 300 s, `src/core/remote.rs` 18). No `SCHEMA_VERSION` bump is needed, because the existing fields already say what a browser needs. The token itself is never stored, only its SHA-256. The identity (`identity.json`) gains nothing; the key stays in the Keychain under `remote-key:<server_id>`.

**Rehydration on host start.** `devices.json` loads into the core through `AppAction::IdentityLoaded` (`src/core/remote.rs` 23–28; `src/core/action.rs` 278). Because the token hash is on disk, a browser's cookie or bearer still authenticates after a host restart without a new pairing. The only thing lost on restart is a pairing window that was open, which is in memory only. A key rotation (missing key at listen) clears `devices.json` (`src/app.rs` 2816, per `docs/remote-client.md` §3.1), so every browser's token is then unknown and the next request gets 401.

**What the page has when it loads.** Nothing from disk. It has its own state, which lives in the browser:
- In memory: the token, if pairing just happened; the epoch and revision from `hello`; the held revision per resource key.
- Per-viewer convenience, safe in `localStorage` (not secret, and per the issue's rule): the server's `spki_sha256` as last seen, the device's display name and scope from the last `/pair`. Without a stored key the page cannot say "identity changed" as opposed to "revoked" (see §3.4).
- Never in `localStorage`: the token. With a cookie (§3.2) the script never has it.

**What the pane shows, in order (the sequence the issue's acceptance needs):**
1. Page loads; a `GET /` answers from the adapter with the embedded HTML (no frame pass, see §3.1).
2. Page reads `location.hash` (fragment form, §3.3) or the query, removes it with `history.replaceState`, compares the key with `GET /api/v1/info` (`src/app.rs` 3568), and POSTs `/api/v1/pair` only on a match (`src/app.rs` 3606).
3. On 200, the page holds the device (cookie set; token not in the body). It opens `wss://…/api/v1/events` first (subscribe first, §4.4), waits for `hello`, then snapshots `projects`, `waiting`, `sessions`, `sessions/{id}` (each a `{epoch, revision, body}` envelope, §4.2), and enables the reply box when it has `control` scope.
4. A session that starts waiting: the diff pass (`src/app.rs` `diff_pass`, the `revision.rs` fingerprint, §4.4) emits `changed {session}` and `changed {waiting}`; the page refetches `waiting` and the session and re-sorts. No reload. This is the acceptance case and it needs no new host code.
5. Session view: last response is `session.last_response` (`/api/v1/sessions/{id}/last-response`), which is a projection of the `conversation` key. **The page must send `watch {kind:"conversation", id}`** on the WebSocket, or the `changed` event for that turn never comes (the 30 s refetch in §4.4 covers it, late). For non-Claude sessions the page watches `screen` instead.
6. A reply: `POST /api/v1/sessions/{id}/send` with an `op` (UUID made by the page) and the text; the body is read in `remote_command` (`src/app.rs` 3159). The reply is `accepted`; progress arrives as `changed {session}` when `Stop` fires.
7. Revocation: the dialog's Revoke drops the device's subscriber (`src/app.rs` 486–497, the socket closes at once) and any later request gets 401 `unauthorized` from `authenticate` (`src/app.rs` 3127–3157). The page's "This browser is no longer paired" message comes from that 401.

So the pane is a pure function of the frame's state at the moment of the snapshot. Nothing about the page's view is persisted by the host.

---

## 3. The six decisions

### 3.1 Browser trust in the self-signed certificate (decision 1)

**What the code does now.** The certificate has no SANs (`src/adapters/remote.rs` 102: `CertificateParams::new(Vec::<String>::new())`) and no set validity beyond rcgen's default (the code's own comment says year 4096, lines 91–94). Browsers will show a warning for every address because the name does not match, and Apple's TLS trust policy rejects long-lived server certificates (I recall a 398-day limit for certificates issued after September 2020; **verify**). So the warning is certain and its exception may be less sticky than the plan assumes.

**What the click-through does and does not buy (my reading, which the plan should state).**
- It buys: the browser accepts that the connection's key is the one the owner accepted, for that host and port, for as long as the exception lasts. It does not buy identity. The owner has no information at the click that tells a genuine host from an impostor on the same address.
- The page's `key` check does not change that. It is only as strong as the page, and a page served by the party the click let through can say whatever it likes.
- The stronger point: **the code is in the URL, and the browser sends the URL to whichever server answers it, before any script runs.** If the URL is `/?code=…` (as the issue specifies), the code goes in the GET request line to the host at that address, and the JS key check runs after that. So the key check cannot protect the code from a host that is not the one on the QR. A stale QR whose address is now another machine (DHCP moved, or another Switchboard on the same address) gets the code in the GET even though the POST is refused. A LAN attacker with its own certificate gets the code in the GET and can then pair itself by POSTing to the real host.
- The fragment form fixes the disclosure to the server: `https://<address>:<port>/#code=…&key=…&id=…`. The fragment is never sent in the HTTP request, so the server sees only `GET /`. The page reads `location.hash`, clears it with `replaceState`, checks the key, then POSTs. This matters for the stale-address case the key check exists for. It does not defeat an attacker who serves its own page (the fragment is visible to that page's script), so it is a narrowing, not a fix. I recommend the fragment and calling this out as a deviation from the issue's text.
- The page's pair POST also has a cost: a POST to a host with the wrong key counts against the global pairing failure counter (`src/app.rs` 3606–3670, the `wrong_key` branch dispatches `PairingFailed`). So the page must do the key check before it POSTs, or a stale QR closes the window after three tries.

**Options and my pick.**
- (a) Click-through as the trust step, said plainly in the paired-devices dialog and the page. *Recommended.* Say in the dialog: "A browser trusts this Mac only by the certificate warning you accept. Accept it only on the local network, and only for the address on this screen."
- (b) Offer the certificate for download and have the owner trust it on the device. Reject for this cut. It needs a path outside `/api/v1/`, which §5 forbids before pairing (except `/info`, `/pair`, and the page). On iOS, trusting a root needs a profile install and a separate trust-settings toggle (**verify** whether a self-signed leaf with no CA bit is accepted). The owner would also have to do a multi-step setup on the phone.
- Reduce the warning's noise: add SANs to the certificate (rcgen 0.14 has `SanType::IpAddress`, `~/.cargo/registry/.../rcgen-0.14.10/src/lib.rs:313`) for the current LAN address and a stable `.local` name for the Mac. Give the certificate a finite validity (≤398 days) and re-issue it **over the same Keychain key** when the address set changes. The SPKI hash stays the same, so the native pin (`docs/remote-client.md` §5) survives. The browser exception does not survive a re-issue, so the owner accepts once more. The `.local` name avoids that for DHCP moves, if the Mac's name is stable. **Verify** Safari's behaviour with a SAN-bearing self-signed leaf.
- Optional, owner's choice: show the certificate's SHA-256 in the dialog next to the key, so the owner can compare it with the certificate details on the warning page if the browser shows them (**verify** iOS Safari shows a SHA-256 fingerprint there).

**Rule kept.** §5's "no non-loopback bind serves anything but `/info`, `/pair` and now the page" holds. The static set in §3.4 is the only addition.

**Other gaps the plan must name:**
- Issue text says the pairing window "opens" with a LAN address even when `lan` is off. `bind_remote` binds loopback when `lan` is off (`src/app.rs` 2962), and `OpenPairing` has no `lan` check (`src/core/remote.rs` 46–52). The QR and URL point to an address the phone cannot reach. This is an existing gap that the URL form makes visible. Fix: refuse or explain in the dialog ("turn on Allow devices on the local network first") when `identity.listen.lan` is false.
- The Mac's address comes from `local_address()` (`src/adapters/remote.rs` 962), a UDP route guess. With more than one interface it can be the wrong one. Pre-existing; the URL makes it more visible.

### 3.2 How the token reaches the WebSocket (decision 2)

**What the code does now.** Only `Authorization: Bearer` is read (`src/adapters/remote.rs` 487–490). There is no `Cookie` parsing, no `Origin` or `Host` check anywhere (grep: no matches for origin, cookie, subprotocol, user-agent in `src/`). `respond_upgrade` (519–526) sends no `Sec-WebSocket-Protocol`, so a subprotocol-token design also needs that echo. `RemoteReply` has no headers field (389–395), so `Set-Cookie` cannot be sent without a struct change (30 literal sites to touch, or a constructor).

**Recommendation: an HttpOnly cookie for browsers, the bearer header kept for native clients.**
- The `/pair` reply for a browser sets the cookie and leaves `token` out of the body. Today the body always carries `token` (`src/app.rs` 3661). A browser that reads the body would hand the token to the script, which defeats HttpOnly. So the page should say it is a browser (a body field, or a header) and the host should answer with the cookie only. The host cannot verify the claim, but the token is the device's own, so a mislabelled reply only costs the HttpOnly protection, not access.
- Cookie attributes: `HttpOnly; Secure; SameSite=Lax; Path=/`, with a persistent expiry (a session cookie can be lost when Safari closes; the token itself does not expire, revocation is what ends it). Name it per installation, for example `sb-<server_id>`. **Cookies are keyed by host, not port.** Two instances on one address with different ports (the dev case `SWITCHBOARD_REMOTE_PORT` exists for) would overwrite one another's cookie under one name. **Verify** whether `__Host-` works for an IP-literal origin.
- `SameSite=Lax` rather than `Strict`: with Strict, a link opened from Messages or Mail is a cross-site navigation and the cookie is not sent, so the page shows "not paired" for an already-paired phone. Lax still blocks cross-site POST, fetch and WebSocket, which is all the state-changing traffic. This is a deviation from the issue's `Strict`.
- Parse `Cookie` in `read_request` (add it to the kept headers, `src/adapters/remote.rs` 444–516) and accept it in `authenticate` (`src/app.rs` 3127), with the same 401 path.
- Because cookie-authenticated requests are the ones a cross-site page could try to ride, add an `Origin` check on them: reject when `Origin` is present and is not this host's own `https://<host>:<port>`. Also consider a `Host` check on the page routes, since DNS rebinding can aim a page at the LAN address. Optional; the reads need a token either way.
- The WebSocket upgrade carries the cookie automatically on a same-origin `wss://`. No protocol echo is needed.
- The page never sees the token. `localStorage` holds only non-secret convenience values (§2).

**Honest limit.** A script on this origin still makes authenticated calls, so XSS in the page is still a way in. The protections are: the page has no inline script and no external resource (CSP, §3.4), the cookie cannot be read or exported by script, and revoke works on the next request.

### 3.3 One QR or two (decision 3)

**Recommendation: one URL, in the fragment form (§3.1), for the QR, "Copy invite", and the CLI and headset paste.** The JSON form is produced only by `Copy invite` today (`src/ui/remote.rs` 160–166, copied at 201). The CLI and the headset parse it (`client/Sources/switchboard-remote/main.swift` 109–112; `client/Sources/SwitchboardProtocol/Info.swift` 113–116). Keep JSON parsing in `PairingInvite.parse` for one cut, so text copied from an earlier build still pairs, then drop the host's JSON output. The URL carries one address (`addresses` becomes `[host:port]`), which is all the code uses. Add a URL case to `testInviteParsesTheQRText` (`client/Tests/SwitchboardProtocolTests/FixturesTests.swift` 157–166) and to `write_fixtures`.

The CLI's three-argument form (`host:port code spki`) stays as it is.

### 3.4 Static paths, headers, pools, and the no-code page (decision 4)

**Routing.** Today `GET /` is refused on the frame (`src/app.rs` 3023–3025: 404 `not_found`), and the frame drains only 8 requests per pass (`src/app.rs` 1584–1596). Serving the page through the frame would make every page load wait for a frame pass and count against that drain. **Serve the static set in the adapter**, in `connection()` before `requests.try_send` (`src/adapters/remote.rs` ~800–812). The static table is then a second allowlist next to the `/api/v1/` prefix in `app.rs`, so both must be tested (§3.5).

**Exact set** (my proposal):
- `GET /`: `text/html; charset=utf-8`, the embedded `web/index.html`.
- `GET /app.css`: `text/css; charset=utf-8`.
- `GET /app.js`: `text/javascript; charset=utf-8`.
- `GET /favicon.ico`: 404 with no body, answered in the adapter, so a browser stops asking and the frame never sees it.
- Anything else outside `/api/v1/`: unchanged (frame, 404 `not_found`).

**Headers on the static set:**
- `Content-Security-Policy: default-src 'none'; script-src 'self'; style-src 'self'; connect-src 'self'; img-src 'self' data:; base-uri 'none'; form-action 'none'; frame-ancestors 'none'`. No inline script or style, no `eval`, no external resource. `connect-src 'self'` must cover `wss://` on the same host: **verify** in Safari.
- `Cache-Control: no-store` (the bytes change with the binary, and there is no version in the path; a version-keyed ETag is the alternative).
- `X-Content-Type-Options: nosniff`, `Referrer-Policy: no-referrer`.

**Response writer.** `respond()` (`src/adapters/remote.rs` 530–553) hard-codes `application/json`. Add a response path that takes a content type and extra headers. This is the same change `Set-Cookie` needs (§3.2), so do it once.

**Pools and rate.** Every TCP connection counts in the 4-slot unauthenticated pool from accept until its first authenticated reply or its close (`src/adapters/remote.rs` 701–707). A browser loads the page with parallel connections for `/`, `/app.css`, `/app.js`, `/info` and `/pair`, and idles them for up to 30 s (`IDLE_TIMEOUT`). Four idle browser connections would fill the pool, and a fifth connection, even from a phone that is trying to pair, is dropped at accept. Recommendations:
- Answer unauthenticated static responses with `Connection: close`, so the slot frees at once.
- Give the static set a rate window like `/info`'s (proposed 60/min per listener), in `Limits` (`src/adapters/remote.rs` 623–640). Today `limited` returns false for every path except `/pair` and `/info`.
- Measure the page load on a phone with the pool at 4 before accepting the design. This is a spike-sized question (the plan's §10 style).
- Note that `/pair` is 6/min per listener (`src/adapters/remote.rs` 583), so a stale-QR attempt that is refused by the page's own key check costs nothing against it.

**What the page shows with no code and no token.** The page's first `GET /info` decides:
- Fragment or query present and key matches: pairing flow (§2).
- No code, no cookie, `pairing_open` false: "Pairing is closed. On the Mac, open Remote devices and choose Allow pairing for 5 minutes, then scan its code."
- No code, no cookie, `pairing_open` true: "Scan the QR code on the Mac, or open the link it shows." The page cannot pair without the code.
- Cookie present but 401: "This browser is no longer paired" (revoked), or "The Mac's key changed; pair again" when the `/info` key differs from the stored one (§3.5).
- A key mismatch between the URL and `/info`: "This is not the Mac whose code was scanned." Nothing is sent.

### 3.5 Tests, and the spec sections to amend (decision 5)

**Test helper.** `http` in `tests/remote.rs` (245–297) returns only the status and body; it discards headers and sends no cookie. Extend it (or add `http_with`) to take extra request headers (`Cookie`, `Origin`) and return response headers. Nothing in the file asserts `Content-Type` today.

**New tests in `tests/remote.rs`** (names as proposals):
- `the_page_and_its_assets_answer_with_their_headers`: `GET /` returns 200, `text/html`, the CSP string, `no-store`, `nosniff`. Same for `/app.css` and `/app.js`. `GET /favicon.ico` returns 404 with no body. `GET /other` still returns 404 `not_found` (the existing test at 425–475 covers it; keep it).
- `the_page_references_only_served_paths`: parse `/api/v1/…` literals and `href`/`src` references out of the embedded page; for each, request it with a paired token and assert the answer is not 404 `not_found`. This is the check the issue asks for. It also catches a route added to the page and not to the host.
- `a_browser_pairs_by_cookie_and_the_cookie_reads_and_revokes`: pinned client; `GET /info`; `POST /pair` with the browser flag and the correct key and code; assert `Set-Cookie` has `HttpOnly; Secure; SameSite=Lax`; assert the body carries no `token`; `GET /api/v1/projects` with the cookie → 200; `RevokeDevice` through the dialog's action → next request 401.
- `a_cookie_from_another_origin_is_refused`: `Origin` set to a different origin on a cookie request → 403 or 401; no `Origin` (native) still works.
- The pool test (`stalled_strangers_fill_their_own_pool_and_never_a_paired_clients`, `tests/remote.rs` 3226) with page requests in the mix: a page's four static connections fill the unauthenticated pool and a paired client is unaffected; add a case for the no-code page being served while the pool is full.
- The pairing end to end with the host's own client, as the CLI does: the Rust `http` helper with the pinned config, a URL-form invite parsed the way `PairingInvite.parse` does, then `/pair`, then a read. Native clients keep the bearer path; the existing `pair` helper (406–423) covers that.
- The UI: the pairing window's `Copy invite` text is the URL form. Add a unit test on the URL builder, since the UI test (`tests/ui.rs` 7420–7505) asserts only the code's shape.

**Fixtures.** `client/Fixtures/` is shared with the Swift tests through `write_fixtures` (`tests/remote.rs` 1587). If `GET /api/v1/device` is added (§3.6), add `device.json` there and its decode check at 1828. The `pair.json` fixture changes only if the browser reply shape is a new variant.

**Spec sections the plan amends** (the issue names these; here is what changes):
- `docs/remote-client.md` §4.1: pools (the static set and the browser's parallel connections), the page's rate window, `Connection: close` for unauthenticated static.
- §4.3: the endpoint table gains `GET /`, `GET /app.css`, `GET /app.js`, `GET /api/v1/device` (§3.6), and the cookie on the existing auth column.
- §5: the pairing URL (fragment form), the browser trust statement (§3.1), the cookie (§3.2), the `Origin` check, and the sentence that the code is in the fragment so no server sees it.
- `docs/design.md` trust boundary (219–265): the redaction sentence already covers "a paired device" and needs no change; add the browser as a device kind.
- `docs/design.md` remote status (3107–3585): a new bullet when the work lands. Not now.

### 3.6 The device record and what the page can know about itself (decision 6)

- The browser is a `PairedDevice` like any other (§2). A name from the user agent (`Safari on iPhone`) can come from the page's own parse of `navigator.userAgent`, sent as `device_name` in the `/pair` body (the host already trims it to 64 chars, `src/core/remote.rs` 166–175). Or the host can read the `User-Agent` header, which it does not keep today (`src/adapters/remote.rs` 444–516). Page-side is simpler; the name is cosmetic, so either works.
- Scope is chosen in the pairing window (`src/ui/remote.rs` 215–217) and changed in the dialog (`src/ui/remote.rs` 251, `SetDeviceScope`). Unchanged.
- **Gap.** Nothing in the API tells an authenticated device its own scope or name. `/info` is pre-auth and must not carry them. `/pair` returns scope, but only once, and a page reloaded later has no scope. The issue wants "the device's scope and host name in the header", and the page cannot do that without a read. I recommend an additive `GET /api/v1/device` under read scope, returning `{name, scope, server_name}`. The issue's "no new read model unless the page cannot do without one" is met by this one. The `info` capabilities list (`src/app.rs` 3568–3603) can stay as it is.
- Consequence: a scope change made in the dialog reaches the page at its next load, or as a 403 `scope` on a send (`src/app.rs` 3159–3173, "raise its scope on the host"). The page should handle that message.

---

## 4. Other findings the plan has to carry

1. **Static responses bypass the frame by design, and that is a second allowlist.** The adapter's static table is a route table outside `app.rs`. Test both (§3.5).
2. **The unauthenticated pool is the page's weakest point.** See §3.4. The current design has no rate window for `/` and no `Connection: close`.
3. **`/pair` returns the token in JSON with `authenticated: false`** (`src/app.rs` 3661 for the token; the envelope call follows it). A browser cookie reply must omit it (§3.2).
4. **Frame-side `wrong_key` counts against the shared failure counter** (§3.1). The page must check the key first.
5. **The code is uppercased by the core and by `/pair`** (`src/core/remote.rs` 50; `src/app.rs` 3646). The page should uppercase too, so the typed form and the URL form agree.
6. **Redaction does not change**: the page reads through the same content routes (`/sessions/{id}/last-response`, `/screen`, `/conversation`), which mask before cut (§4.3, `src/app.rs` 3099–3110).
7. **No ALPN** (`src/adapters/remote.rs` 144–158): browsers use HTTP/1.1. This is fine. Do not add `h2` without re-checking the one-request-in-flight rule (§4.1).
8. **The WebSocket subscriber is one paired slot** for as long as the page is open (`src/adapters/remote.rs` 878–927, paired pool `MAX_PAIRED` 12). A phone with several tabs uses several slots. Fine at this scale; note it for the plan.
9. **The page's own `GET /` is unauthenticated and reveals nothing except the page.** The page's JS holds the logic, so a repo reader can see the protocol. That is the same as the native clients' documented protocol, so no secret is exposed.

---

## 5. What I could not check here (say these in the plan, not as facts)

- **Safari (iOS) behaviour** with a self-signed certificate: the warning's exception scope and stickiness; whether the exception covers `wss://` on the same host (if not, the events socket fails and the page would fall back to refetching on a timer); whether `connect-src 'self'` covers `wss:`; whether SAN-bearing self-signed leaves are accepted under the click-through; whether a 398-day rule changes the warning. All of this needs a phone.
- **`__Host-` cookie prefix** on an IP-literal origin.
- **Whether Safari shows a SHA-256 fingerprint** on the warning page's details.
- **Pool behaviour under a real page load** (§3.4). A stub client can reproduce it; the phone is the only real check.
- Nothing has been built or tested. `cargo test --locked --workspace`, `cargo clippy ... -D warnings`, and the Swift tests were not run.

---

## 6. Ways the acceptance criteria could fail, as the code stands

- "Open the pairing window, scan the QR with a phone": fails if `lan` is off (§3.1, the loopback bind). The QR address is then unreachable.
- "Accept the certificate warning, see the sessions": works only if the click-through is sticky and covers `wss://` on Safari (§5). The static `/` is served from the adapter, so it does not depend on the frame.
- "A session that starts waiting moves to the top without a reload": works on the existing diff and events (§2 step 4), provided the page subscribes before it snapshots and re-sorts on `changed {session}` and `changed {waiting}`.
- "Send a reply from the phone and watch it arrive": works through `POST /sessions/{id}/send` with a page-made op (§2 step 6). The reply is `accepted`, and the reply appears on `Stop`. The page must watch `conversation` to see it (§2 step 5).
- "Revoke the phone in the dialog and the page says so on its next request": works (§2 step 7). The socket also closes at once (`src/app.rs` 486–497), so the page may learn before its next request.

---

## 7. Decisions I would ask the owner to confirm

1. Fragment, not query, for the code in the URL (§3.1). It is a deviation from the issue's text.
2. `SameSite=Lax`, not `Strict` (§3.2). Also a deviation.
3. Click-through as the trust step, said in the dialog, versus the certificate download (§3.1). I recommend the click-through.
4. SAN and validity changes to the certificate, and whether the Mac's `.local` name is the stable one (§3.1).
5. `GET /api/v1/device` as an additive read (§3.6).
6. Dropping the host's JSON invite once the URL form is in, after one cut of parsing both (§3.3).
