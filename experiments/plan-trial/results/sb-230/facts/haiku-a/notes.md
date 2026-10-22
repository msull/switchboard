# Issue #230: a web control surface on the remote port. Investigation notes

Worktree: `/Users/sbgragent/.dispatch/worktrees/50e56c62`, HEAD `7fe8949` (Release 0.4.1).
Scope: read only. Nothing built, run, or changed in the tree. Browser and platform behaviour is not verified here; every such claim is an open question below.

Read: `CLAUDE.md`, `docs/design.md` (trust boundary; remote status), `docs/lessons.md`, `docs/remote-client.md` §0–5 and §10–13 (§6–9 not read in full), `src/adapters/remote.rs`, `src/app.rs` (`serve_remote` and the remote handlers), `src/ui/remote.rs`, `src/core/remote.rs`, `src/ports/remote.rs`, `control/src/lib.rs` (`SessionView`), `src/ui/cards.rs` (excerpt), the Swift text pairing path, and the `tests/remote.rs` HTTP helper.

---

## 1. Code the issue touches

### Transport (`src/adapters/remote.rs`)

- **:530–552 `respond()`**: the only response writer. It hard-codes `Content-Type: application/json` (line 545) and `Connection: keep-alive`. It takes a status and a body, with no way to add headers.
- **:389–393 `RemoteReply`**: `status`, `body`, `authenticated`. No headers field.
- **:444–514 `read_request`**: keeps method, path, query, body, `bearer` (from `Authorization` only, :487–490) and the WebSocket key (:491–494). It does not keep cookies, `Host`, `Origin`, `User-Agent` or `Sec-WebSocket-Protocol`.
- **:519–526 `respond_upgrade`**: writes the 101 with no subprotocol echo.
- **:770–867 `connection()`**: per connection. The path rate limit runs first (:788–795). Then the request goes to the frame over a 64-deep channel, and a full channel answers 503 (:810–825). The answer waits up to 60 s (:827). The connection moves to the paired pool on an authenticated reply (:835–849), and the reply is written at :864.
- **:567–568, :701–707**: pools of 4 unauthenticated and 12 paired. A socket is counted as unauthenticated at accept (:707). It moves to the paired pool only on an authenticated reply. A keep-alive socket that never authenticates holds its slot until it closes or its idle timeout (`IDLE_TIMEOUT`, :573, 30 s) expires.
- **:623–639 `Limits`**: only `/api/v1/pair` and `/api/v1/info` are rate-limited. Every other path is unlimited.
- **:95–128 `make_identity`**: `CertificateParams::new(Vec::<String>::new())`. The certificate has no subjectAltName, only a CN of `Switchboard <uuid>`. Its validity is rcgen's default (the doc comment says "never expires in practice").
- **:144–155 `tls_config`**: no ALPN is set.
- **:168–180 `pinned_client`**: the native client's SPKI verifier.
- **tests/remote.rs:245–298 `http()`**: the test client. It parses only `Content-Length` from the response, sends `Host: localhost`, and sends only an `Authorization` bearer. It cannot read `Set-Cookie`, `Content-Type` or any security header.

### Host dispatch (`src/app.rs`)

- **:3017–3123 `serve_remote`**: runs on the frame.
  - :3023–3025: any path without the `/api/v1/` prefix is refused with 404 through `refuse`. So `GET /` is a frame request today and gets a 404.
  - :3028 `info` and :3029 `pair` are served before authentication.
  - :3032 `authenticate`, then :3036 the `events` upgrade.
  - :3044–3117 the allowlist match, with a catch-all refusal at :3108–3116.
  - :3119 hides worker records unless `scope=all`.
- **:3127–3152 `authenticate`**: a bearer only. It returns 401 "pair this device first" with no bearer, and 401 "the token is not known; pair again" for an unknown or revoked token (:3142–3148). It also calls `DeviceSeen`.
- **:3159–3175 `remote_command`**: checks control scope (403 `scope`), then reads the body's `op`.
- **:2723–2762 `remote_subscribe`**: requires an upgrade. It queues `hello`, registers the subscriber, and returns `authenticated: true`.
- **:3568–3599 `remote_info`**: capabilities at :3580 (`read, control, draft, scope, tickets, evidence, create`). It has no field describing the caller, so no device can learn its own scope or name from it.
- **:3606–3673 `remote_pair`**: checks the SPKI (:3636) before the code (:3646). It creates the device with the window's scope, returns `{token, scope, server_id, device_id}` in an envelope with `authenticated: false` (:3672), and dispatches `Paired`, which closes the window.
- **:3730–3741 `envelope`**: `{epoch, revision, body}`.
- **:3754–3770 `refuse`**: logs method and path only (:3762–3766). A query string is therefore not logged by refusals.
- **:3662** is the only place `scope` is returned to a device (in the pair reply).

### Core and state

- **`src/core/remote.rs`**: the `OpenPairing`, `ClosePairing`, `PairingFailed`, `Paired`, `RevokeDevice` and `SetDeviceScope` arms. `new_device` trims the name to 64 characters and takes the scope from the window.
- **`src/core/action.rs:972–973`**: `PAIRING_WINDOW` is 5 minutes and `PAIRING_ATTEMPTS` is 3. The pairing window is in memory only, per the doc comment at :290.
- **`src/ports/remote.rs:96–107` `PairedDevice`**: `id`, `name`, `scope`, `token_sha256`, `created`, `last_seen` (serde default). It has no schema field and no kind field.
- **`src/app.rs` ~2783–2840**: startup. A missing key makes a new identity and clears the devices (the design doc says so too).

### Pairing window (`src/ui/remote.rs`)

- **:155–166**: the QR payload is JSON `{server_id, spki_sha256, code, addresses}`.
- **:157–159**: `addresses` holds one entry, `local_address()` plus the port.
- **:172 and :280–299**: the QR texture.
- **:199–206 "Copy invite"**: copies the JSON.
- **:70–76**: the redaction sentence in the dialog.
- **:110–113**: the "Allow devices on the local network" checkbox (`listen.lan`, off by default).
- **:214–224**: scope radio, Read by default. **:230–276**: device rows with scope, last seen, and Revoke.

### Wire view and card excerpt

- **`control/src/lib.rs:637` `SessionView`**: `card` (a string), `waiting`, `waiting_reason`, `trust_question`, `kind`, `resume_id`, `last_stop_at_ms`, `quiet_secs`, `last_activity_ms`, `transcript`. **It has no excerpt field.**
- **`src/ui/cards.rs:176–178` and `:558–575`**: an agent card's excerpt is computed in the UI from the transcript's last turn (first line of the final text, else the last activity, else the prompt), clamped to 160 characters.

### Native text path

- **`client/Sources/switchboard-remote/main.swift`**: the usage text and the `pair` case. One argument goes to `PairingInvite.parse`; three arguments are host, code and SPKI.
- **`client/Sources/SwitchboardProtocol/Info.swift`**, end of file: `PairingInvite` decodes JSON only (`Wire.decode`). `KeyFingerprint.short` is the owner-comparison form.

### Docs and other

- **`docs/remote-client.md`**: §4.1 bounds (:124–131); §4.3 endpoint table (:157–192) and the allowlist sentence (:194); §5 pairing (:302–319; the QR JSON at :309; "never in a URL" at :312); §10 status (:429–469); §11 tests (:471–510).
- **`docs/design.md`**: the trust-boundary paragraph (about :240–256); the remote client status (:3107 on).
- **`README.md:267`**: `SWITCHBOARD_REMOTE_PORT`. The Layout table and dev-aid list need entries for any `web/` file.
- **`docs/lessons.md:27–29`** ("Keep `ui.rs` dumb") and CLAUDE.md's rule that logic under `src/ui/` moves to the core. The planned page reconciles revisions in JavaScript; §4.4 of the spec puts that in the client.
- No `web/` directory exists.

---

## 2. The on-disk record and its rehydration, before the pane

**Host, today.**
- `identity.json`: `server_id`, `server_name`, the certificate (DER, base64), `spki_sha256`, `created`, `listen {lan, port}`. The private key is a Keychain item, `remote-key:<server_id>`. Startup restores these through `IdentityLoaded`.
- `devices.json`: the list of `PairedDevice`. A browser pairing adds one record like any other. Its `name` is whatever `device_name` the body carries, trimmed to 64 characters. Its scope is the window's scope. The host stores only `token_sha256`.
- Restart: `IdentityLoaded` restores the devices. `device_by_token` matches the hash. A browser that still holds its token therefore works again after a relaunch, provided the identity and its Keychain key survive.
- Keychain lost (or an unsigned rebuild, per CLAUDE.md): a new identity is made, the devices are cleared, and every token gets 401. The browser's certificate trust also breaks, since the certificate changes.
- Pairing window: in memory, gone on relaunch. Nothing to rehydrate.
- The page's assets are compiled in. They are not records.

**Browser, per the issue (none exists yet).** The token (cookie or page memory, decision 2). The pinned key from the URL, if the plan keeps it, and where. The browser's own certificate exception for the host's address and port.

**Session data is not new.** Projects, sessions and their state come from the existing records (`SessionRecord`, `views.json`) through the read models the headset already uses. The page's view state is in memory only.

**The pane, after that record.** The page loads `GET /`, which is a new static route. The script reads `code`, `key` and `id` from the query and calls `history.replaceState`. It then calls `GET /api/v1/info`, compares the key, and calls `POST /api/v1/pair` with `{code, device_name, spki_sha256}`. The reply's token is kept. Then the page calls `GET /api/v1/projects`, `GET /api/v1/waiting` and `GET /api/v1/projects/{id}/sessions` and renders groups, waiting ones first, marked from `waiting` and `waiting_reason`. It opens `/api/v1/events` with the token, applies `hello`, snapshots, and refetches on `changed`.

Three gaps show in that pane:
- The card's one-line excerpt is not on the wire.
- The device's scope is not readable after pairing, so the reply box cannot be hidden on reload.
- The session screen or last response is a separate request per session.

---

## 3. Findings from the tree

1. **Static responses need a writer that can set headers.** `respond()` is the only writer and hard-codes JSON; `RemoteReply` has no headers. The CSP, `Cache-Control`, `Referrer-Policy`, `Content-Type` and any `Set-Cookie` all need one. Every construction site of `RemoteReply` (in `app.rs` and tests; I did not count them) changes if the struct gains a field.
2. **Static assets can skip the frame.** Constant bytes need no core state. Answering them on the connection thread matches §4.1's rule that content work stays off the frame. On the frame, each page load costs drain slots (8 per pass) and channel depth, and a page load is several requests.
3. **The unauthenticated pool counts sockets, not requests.** A browser loading one page can open several sockets to one origin. Each one not yet carrying a bearer holds one of four slots from accept until it authenticates or times out. The pairing POST, or a new socket from the paired device, can therefore be closed at accept (:701–706). The socket count browsers use is [source]; the effect is [run].
4. **Token transport needs parser changes.** `read_request` reads only `Authorization`. A cookie needs cookie parsing. A subprotocol token needs parsing and an echo in `respond_upgrade` (:519–526), which the browser requires when it offers a subprotocol [source].
5. **Cookies are ambient, and nothing checks `Origin`.** `read_request` drops `Origin`. Today's bearer design is safe because browsers never attach `Authorization` on their own. Cookie-authenticated state changes (`send`, `resume`, `trust`, new session, drafts, dictation, ticket commands) and the `events` upgrade would ride along on any same-site request. Whether `SameSite=Strict` stops that depends on [source] questions below.
6. **No `Host` check.** `read_request` ignores `Host`. That matters for DNS rebinding against unauthenticated reads (`/info`, `/pair`) [source].
7. **The certificate has no SAN.** A browser checks the name and the validity period, not only the key. A click-through therefore accepts a warning with more than one cause, and the SPKI check the issue relies on is the native client's, not the browser's. Browser behaviour is [source].
8. **Cookies are not port-scoped.** Two instances on one machine at one address, on different ports (the `SWITCHBOARD_REMOTE_PORT` dev aid, README:267), would share a cookie name and overwrite each other's token. Name the cookie per `server_id` if one is used; the page can learn `server_id` from `/info`. [source] for the scoping rule.
9. **No endpoint tells a device its scope or name after pairing.** `scope` is returned only by `/pair` (:3662). `/info` is unauthenticated and has no caller field (:3568–3599). The issue's "device's scope in the header" and its "hide the reply box" need a new read, which the issue allows only when the page cannot do without one.
10. **The card's excerpt is not on the wire.** `SessionView` (`control/src/lib.rs:637`) has no excerpt, and §4.3 names a `last_message` on the Stop hook that the tree does not have. Today the UI derives the excerpt from the transcript (`cards.rs:558`). The page would need a new additive field or a second request per card.
11. **The QR carries one address.** `addresses` has one entry (`ui/remote.rs:157–164`), so the URL does too. The native path decodes JSON only (`Info.swift`, `PairingInvite.parse`), and the CLI's one-argument form assumes JSON. "Copy invite" copies JSON (`ui/remote.rs:199–206`). A URL changes that text and both native parsers unless one keeps JSON.
12. **The code reaches the host in a request line, and the page's own requests may leak it.** `refuse` logs path only, so no refusal logs the code. The static route must keep that rule. The page's script and CSS are same-origin subresources loaded while the URL still holds the query. Under the default referrer policy, same-origin requests carry the full URL in `Referer` [source]. A `Referrer-Policy: no-referrer` header on the page is needed.
13. **Browser history keeps the URL.** Navigation records the URL with its query in the browser's history, which `replaceState` does not rewrite [source]. The code is single-use and is spent by the first `/pair`, and the window closes after 5 minutes (`action.rs:972`). The residual exposure is a history entry for a spent code.
14. **The browser-side key check protects little.** The URL's key is genuine (it came from the host's screen). But the script and `/info` come from whoever answers the connection. An impostor serving its own page answers `/info` with whatever key it likes. This matches the issue's own caveat; nothing in the tree gives the browser a pin.
15. **The allowlist and the pairing rule change.** §5 says nothing but `/info` and `/pair` is served before pairing, and §4.3 says the allowlist is explicit. The static paths need a new explicit set, checked before `serve_remote`'s prefix test (:3023) or on the connection thread.
16. **Test helpers limit what can be tested.** `http()` (tests/remote.rs:245–298) reads no headers but `Content-Length`, so header assertions need the helper extended. `call()` runs requests on the test thread through the frame, and a connection-thread route has no frame hook. Existing pairing tests: `a_pairing_inside_the_window_gives_a_token…` (:477), `a_wrong_pin_fails_at_the_handshake…` (:1395), `pair_is_rate_limited_per_listener` (:1405), `the_allowlist_refuses_everything_else…` (:681). The client fixtures (27 JSON files in `client/Fixtures`) and `the_client_fixtures_decode_on_both_sides` (:1828) cover Swift and CLI shapes, not HTML or a fourth consumer.
17. **`PairedDevice` has no version or kind field** (`ports/remote.rs:96–107`). CLAUDE.md's `SCHEMA_VERSION` rule covers `model.rs`; `ports/remote.rs` types are outside it. `last_seen` is a precedent for a `#[serde(default)]` addition.
18. **The page reconciles revisions in JavaScript.** §4.4 puts that in the client, which is right for the native client. CLAUDE.md and `lessons.md` want logic in the core, not in UI code. The page is not under `src/ui/`, but the spirit applies.

---

## 4. What I did not check

- I did not build, run, or test anything. No `cargo test`, no live app, no `curl`, no browser.
- I read `tests/remote.rs` only at the helper and at test names. I did not read any test body or any fixture's contents.
- I did not read `remote_content_route` (:1947), `remote_draft`, `remote_dictation`, or the ticket handlers, beyond their signatures.
- I did not read `docs/remote-client.md` §6–9, the rest of the `design.md` remote status, or the visionOS app sources.
- I did not read the Swift `ConnectionManager`, `HostClient`, or the visionOS paste path, so the "headset pairs from Bonjour and a typed code" claim is not checked here.
- I did not count `RemoteReply` construction sites, and did not confirm how `devices.json` is serialized (whether it is a bare array).
- I did not check whether `tungstenite` 0.30 can write a subprotocol echo through `respond_upgrade`; the upgrade is written by hand.
- I did not check `scripts/bundle.sh` or the Keychain ACL behaviour on a rebuilt bundle.

---

## Open questions

- [source] Does Safari on iPhone or iPad, and Chrome on Android, let the owner proceed past a self-signed certificate that has no subjectAltName and only a CN, and does that exception persist per host, per host and port, or per session?
- [source] Does a proceeded-past certificate exception also apply to `wss://` on the same origin, and to `fetch`?
- [source] Does a browser refuse a self-signed leaf certificate whose validity runs to the rcgen default (year 4096; `adapters/remote.rs:95–100`), and does the answer differ when the owner clicks through?
- [owner] Decision 1: is the click-through the trust step (disclosed in the paired-devices dialog), or does the host offer its certificate for download and the owner installs it on the device first (which means a device to test the install on)?
- [owner] Is a click-through warning acceptable on the owner's own LAN for a surface that can send prompts to agents?
- [tree] If the certificate gains a SAN for the LAN address, does the spec treat that as a reissue over the same key (no rotation, since §5 pins the SPKI) or as a new identity? The tree has no reissue path; `identity.json` is written once (`docs/remote-client.md` §3.1).
- [source] Does `fetch` send a `Secure; HttpOnly; SameSite=Strict` cookie on same-origin requests and on the `wss://` handshake, under a click-through certificate?
- [source] Does a cookie set by `192.168.x.x:43187` reach the same address on another port? RFC 6265 §8.5 says cookies are not port-isolated; confirm the browser implementations.
- [source] Does SameSite treat a page on another port of the same IP, or on `localhost`, as same-site, so that `Strict` cookies still ride along? This decides whether an `Origin` check or a custom request header is needed.
- [source] Does a cross-origin WebSocket handshake carry `SameSite=Strict` cookies, and does the browser send `Origin` on the upgrade so the host can check it?
- [source] Does a non-simple request header (for example `X-Switchboard: 1`) force a CORS preflight that the host's refusal of `OPTIONS` blocks, so a cross-origin page cannot send a state-changing POST?
- [source] When a browser offers a `Sec-WebSocket-Protocol` value, must the 101 echo one of them or the handshake fails?
- [tree] Does any native client send `Authorization` such that a cookie branch in `authenticate` changes its behaviour? (The CLI and visionOS send bearers; `SwitchboardTransport` was not read.)
- [owner] Decision 2: is the cookie a session cookie (lost on browser close, so re-pair each time) or persistent, and for how long? The spec says tokens do not expire.
- [owner] Where the pinned key lives on the device: `localStorage`, a cookie, or nowhere. Without one, "identity changed" on a later visit can only show as the browser's own certificate warning.
- [tree] Is `localStorage` used for anything if the token is a cookie? §5 allows only per-viewer conveniences there.
- [owner] Decision 3: is the JSON invite still needed by the CLI, the headset, or anything else, or does the URL replace it everywhere?
- [tree] The headset's paste path and `SwitchboardClientCore` decode the invite; not read, so whether they can take a URL is unknown.
- [source] Does iOS Camera open an `https://` URL with an IP and a self-signed certificate straight into Safari, and does the warning then appear inside Safari, or does a banner need a tap first?
- [run] Does the QR stay scannable with the URL form (a 43-character key hash, an 8-character code, the address), and at what size? The current JSON form is not measured either.
- [tree] The exact static path set (`/`, the script, the stylesheet, any favicon). The tree shows only one prefix check (`app.rs:3023`) and no static route.
- [tree] Do static routes run before `authenticate` on the connection thread, and does the allowlist test (`tests/remote.rs:681`) need a new case?
- [source] Does a CSP of `connect-src 'self'` cover the same origin's `wss:` scheme in Safari and Chrome, or is an explicit `wss:` source needed?
- [source] Does a `Referrer-Policy: no-referrer` response header apply to the page's own script and stylesheet loads?
- [owner] What the page says when opened without a code and without a token (copy and wording, a taste call).
- [tree] Static GETs reach `MAX_UNAUTHENTICATED` through accept (:701–707). The plan must choose a separate static pool, a larger cap, or answering statics without a pool slot; §4.1's wording needs an amendment either way.
- [run] How many sockets Safari and Chrome open to one origin while loading the page and then the WebSocket, and whether a socket refused at accept is retried.
- [run] Whether an idle keep-alive browser socket blocks a pairing POST during the 30 s idle timeout (pool exhaustion).
- [tree] Whether static GETs need a rate window. `Limits::limited` (:631–638) matches only two paths.
- [source] The right cache headers for a page embedded in the binary. Without `Last-Modified` or `ETag` browsers should not cache heuristically, but `Cache-Control: no-cache` should be explicit.
- [tree] The size of the embedded page and whether statics need a size cap, since the 1 MiB cap is on the frame's JSON bodies.
- [tree] Whether CLAUDE.md's "Adding a feature" steps (action, core test, widget, UI test, script line, README) apply to a non-egui page. The issue's test plan (`tests/remote.rs` only) is a departure that the plan should say it makes.
- [tree] Whether `tests/remote.rs`'s `http()` helper gets a headers return, and what it looks like.
- [run] Whether the embedded script can be exercised at all in CI. No browser test harness exists.
- [owner] Is a browser test tool (Node, jsdom, Playwright or similar) acceptable as a dependency? The issue forbids one for the page itself, not for tests.
- [owner] Does the page's JavaScript reconcile logic stay in the page, or does the plan want a Rust-side projection so the logic sits in the core?
- [tree] Does the page count as a fourth consumer of the fixtures, and does `the_client_fixtures_decode_on_both_sides` extend to the HTML or the page's JSON reads?
- [tree] Whether `PairedDevice` gains a `kind` or `agent` field, and whether a `#[serde(default)]` addition is enough (precedent: `last_seen`, `ports/remote.rs:105`).
- [source] Does `navigator.userAgent` identify an iPad? Desktop-mode iPadOS reports as a Mac. The UA header is not kept by `read_request` today.
- [tree] Should the host derive a device's name from the `User-Agent` header (needs `read_request` to keep it), or trust `device_name` as sent (`app.rs:~3650`, `core/remote.rs` `new_device`, 64-character trim)?
- [owner] Name wording in the dialog, and whether browser devices are listed differently from native ones.
- [tree] Scope for a browser device is the window's scope, Read by default. A device cannot learn its scope after pairing (finding 9). Which new read, if any, the plan adds (`GET /api/v1/device`, or a caller field on an existing read)?
- [tree] Revocation and "pair again" look the same to the page: both are the 401 at `app.rs:3142–3148`. Does the plan need a distinct error code, an additive change to `refuse`?
- [tree] "Identity changed, pair again": the host deletes devices only on key rotation (`app.rs` ~2802–2840). Nothing on the host detects a client's stored key. This is a client-side rule the page must implement, so the question is which pinned key it stores (see the storage question above).
- [owner] Should the page appear only when the LAN checkbox is on? With it off, the listener is loopback, and the phone cannot reach the page at all.
- [tree] Confirm that statics on the connection thread leave the frame's 4 ms budget (design §10) untouched, and that nothing else in `serve_remote` changes.
- [owner] Is a second instance on the same machine (the dev aid) a case the page must handle, or is the cookie name per `server_id` enough?
- [run] The acceptance flow end to end: a phone on the same LAN, the owner's click-through, a session moving to the top on `changed {waiting}`, a reply sent and seen, a revoke shown on the next request. Not run; needs a phone and a LAN.
- [owner] Restart-survives-a-browser-pairing needs a signed bundle (`scripts/bundle.sh`), since an unsigned rebuild loses the Keychain key and every pairing (CLAUDE.md, platform rules). That needs the owner's signing identity on this Mac.
- [source] The issue's own claims are not verified and are listed here as questions: a phone's camera opens a URL in the browser; `localStorage` is readable by any script on the origin while a cookie marked `HttpOnly` is not; browsers cannot pin an SPKI; a browser sends a cookie on the WebSocket handshake.
