# Issue #230: a web control surface on the remote port

Line numbers are from this worktree (`dispatch/230-a-web-control-surface-the-remote-port-se`), checked for this plan. Facts cited as **[settled]** are lines under Settled in `facts.md`. Anything else is marked where it is used.

## Where this departs from the issue

1. **The code moves out of the query string and into the URL fragment, which keeps it out of the host's logs and caches but does not stop an on-path attacker.** The issue's URL is `/?code=…&key=…&id=…`. The tree sends the request target to the host unchanged: `read_request` splits the query into `RemoteRequest.query` (`src/adapters/remote.rs` 496–502). A GET that carries `?code=` therefore puts the code in the request line the host reads, and in any log or cache on the way, before any script runs. The plan uses `https://<address>:<port>/#code=…&key=…&id=…`. A fragment is not part of the request target (RFC 9112 §3.2: origin-form is `absolute-path [ "?" query ]`), so the code stays out of the request line, the host's logs and the host's caches. The fragment does not stop the attack Spec §5 names ("the code alone would let an intermediary … take the token", `docs/remote-client.md` 302–320): whoever answers the address still serves the page script, which reads the fragment and posts the code to that host. Decision 1 says what the plan does about that. The RFC line is cited here as its own source; `facts.md` is not edited.
2. **"The one-line excerpt the card shows" does not exist as a field.** `SessionView.card` is the card's word (`control/src/lib.rs` 645–646) **[settled]**. The card's second line is `waiting_reason` (`src/ui/cards.rs` 197). The page shows `waiting_reason` when `waiting` is true and `card` otherwise. No new view field.
3. **The device's own scope in the header.** No route returns a calling device's scope (`src/app.rs` 3031–3125; facts §3 and "Settled"). The page stores the scope the `/pair` reply grants, and learns a change from a 403 with code `scope` (`src/app.rs` 3166–3173) **[settled]**.
4. **Revoked and "identity changed" look the same on the wire.** Both reach the 401 "the token is not known; pair again" (`src/app.rs` 3146). The host stays as it is. The page stores the key it paired with, and on a 401 it fetches `/info`. A different key means "identity changed, pair again"; the same key means "no longer paired with this Mac". The issue's "revoked" message is delivered this way.
5. **A browser cannot pin the host's key, so nothing on the page authenticates the host.** The issue says the key check protects a genuine page. It cannot protect the first visit. The page cannot read the key of its own TLS connection, and `/info` is JSON the answering server writes (`src/app.rs` `remote_info`, 3568), so an impostor can echo the genuine key. The key in the URL checks one thing: that the host answering the address is the Mac whose code was scanned. The acceptance is therefore "accept the warning, see the sessions", and what the warning trusts is said in the devices dialog (Decision 1).

## Record

No persisted record or schema changes; a browser pairing writes an ordinary `PairedDevice` (`src/ports/remote.rs` 96): `name` from the page, `scope` from the pairing window, `token_sha256` of the cookie's token. `SCHEMA_VERSION` stays **16** (`src/core/model.rs` 12), `VIEWS_SCHEMA_VERSION` **7** (612), and `IDENTITY_SCHEMA_VERSION` **1** (`src/ports/remote.rs` 17). The certificate reissue in Decision 1, if taken, rewrites the existing `certificate` field from the same Keychain key and adds no field.

- The page's own state lives in the browser. It holds only `server_id`, the key it paired with (`spki_sha256`), the scope and the host name, none of them secret. It goes in `localStorage` in a `try`/`catch`, and the page works without it (a missing scope or key falls back to `/info` and to a 403 correction).
- The bearer token is never stored by the page. It lives only in the `__Host-sb` cookie (Decision 2), which the page's script cannot read.

## Actions and effects

### Port: the page's files and the connection thread

A new module `src/adapters/remote_page.rs` holds the embedded files and the response rules. It is listed in the README Layout table.

- **Files**: `web/index.html`, `web/app.js`, `web/app.css`, embedded with `include_str!`. No build step, no bundler, no package manager.
- **Paths**, GET only:
  - `/` → `index.html`, `text/html; charset=utf-8`
  - `/app.js` → `text/javascript; charset=utf-8`
  - `/app.css` → `text/css; charset=utf-8`

  Any other method or path is answered as today (through the frame, 404). The query on `/` is ignored, so an old `?code=` link still loads the page, and the host never reads the code in it.
- **Headers** on every page response:
  - `Cache-Control: no-store` (stricter than `no-cache`; the issue asks for no caching across versions).
  - `Content-Security-Policy: default-src 'none'; script-src 'self'; style-src 'self'; connect-src 'self' wss://<Host>; base-uri 'none'; form-action 'none'; frame-ancestors 'none'`. There is no inline script, no inline style and no external resource. The socket is allowed to the one host and port the request came in on: `<Host>` is the request's `Host` header as the browser sent it (an IPv4 address, a bracketed IPv6 literal, or a name). The header is built per response, so the policy names the address the phone actually dialled. A `Host` containing anything outside `[A-Za-z0-9.:\[\]-]`, or no `Host`, gets 400 and no page, so no header injection reaches the response. A bare `'self'` is not relied on for the socket, because `'self'` does not reliably cover `wss:` in every browser (facts "Primary sources", CSP `connect-src`); `wss://<Host>` names it explicitly. The IPv6 bracket is part of the host-source, so the same string serves both.
  - `X-Content-Type-Options: nosniff`, `Referrer-Policy: no-referrer`, `Connection: close`.
- **Order in `connection`** (`src/adapters/remote.rs` 770): `read_request` (444, line 783), then the `limited` check (788, which answers 429 and continues the loop; it returns false for static paths, so they are not counted), then the static answer, then the frame queue (`reply_tx` and `requests.try_send`, which follow). The static answer writes its response on the connection thread and closes the connection; the frame is never involved. `respond` (530) gains a header parameter; its fixed JSON head (545) stays the same for JSON.
- **Pools**: a static request takes an unauthenticated slot at accept, as `/info` and `/pair` already do (`MAX_UNAUTHENTICATED`, 567). The accept loop (699–706) cannot tell a static request from any other before it reads one. **For the owner (issue decision 4, owner Q6):** a LAN flood of `/` can churn the four slots and starve `/info` and `/pair`. That exposure already exists for `/info` and `/pair`, and §4.1 accepts it. No window is built. The phone measurement (run item) decides whether four slots are enough, and a result that says they are not sends this plan back to the owner.
- **Before pairing**: the page is served with no code and no token, as the issue asks, and it shows nothing private (owner Q7).

### Pairing: the URL, the cookie and the Origin check (Decision 2)

- **Request**: the browser's `/pair` body is the native body plus `"via": "browser"`. The native clients never send `via`, so they keep receiving the token in the body. `remote_pair` builds that body (`src/app.rs` 3606–3672); `token` goes into it at 3661 and the reply is sent at 3672. `PairReply` in `client/Sources/SwitchboardProtocol/Info.swift` reads it.
- **Reply for `via: browser`**: `Set-Cookie: __Host-sb=<token>; HttpOnly; Secure; SameSite=Strict; Path=/; Max-Age=31536000`, and a body of `{scope, server_id, device_id}` with no `token`. `Max-Age` is set so the cookie survives a browser restart; the host still revokes by record.
  - Why a cookie: `HttpOnly` keeps the token from script on the page, which `localStorage` could not do. `SameSite=Strict` keeps it off cross-site requests.
  - **What the cookie does not isolate**: `__Host-` does not isolate ports. RFC 6265 §8.5 says cookies "do not provide isolation by port". Any other `Secure` service on this host, on any port, that reads cookies is sent `__Host-sb`. The cookie is sent only to this host's address, so a service on another machine never gets it. This cost is an owner decision (Unresolved table).
- **Reading the cookie**: `read_request` (`src/adapters/remote.rs` 444–511) keeps `Cookie`, `Host` and `Origin` on `RemoteRequest` (360), and `RemoteIncoming` (398) carries them to the frame. Neither type has a copy in `src/ports/` (checked). `authenticate` (`src/app.rs` 3127) reads only `request.bearer` today (3128–3129); the `__Host-sb` cookie is a second branch after it. Cookie auth is what the page uses.
- **Origin check**: a request authenticated by cookie that is not a GET (`POST` and any other method), and the `/api/v1/events` upgrade (`remote_subscribe`, `src/app.rs` 2723), must carry an `Origin` equal to `https://<Host>` exactly. `<Host>` is the request's `Host` header with its port: `192.0.2.7:43187`, or `[fd00::7]:43187` for IPv6. A missing `Origin` or a different one is 403 `origin`. The comparison is string equality. The URL carries an IP address, never a name, because the certificate has no name to match (`src/adapters/remote.rs` 102), so `<Host>` is always an IP. Bearer requests are exempt, since native clients send neither an `Origin` nor a cookie. Browsers send `Origin` on a WebSocket handshake, and the server must check it itself (facts "Primary sources", Origin).
- **Token and device name**: the page sends `device_name` derived from `navigator.userAgent` (for example `Safari on iPhone`). `new_device` caps it at 64 characters (`src/core/remote.rs` 166–170). The page sends the name; the host does not parse a user agent. The name is a label, not an identity.

### Pairing: the trust step (Decision 1, For the owner)

**What the first visit trusts.** The browser shows a certificate warning, and the owner accepts it. The browser then keeps that certificate for that address. Nothing on the page verifies the host: the page cannot read its own connection's key, and an on-path attacker who answers the address takes the code and the token. The attacker relays `/pair` to the Mac, the Mac checks the key in the URL, which the attacker holds, and issues the token to the attacker. The 5-minute window is the exposure, and it exists only on an untrusted network. The plan states this in the devices dialog rather than hiding it behind a check that does not work.

**Default (the plan builds this):** the click-through, stated plainly as the trust step, plus two checks that are real even though they are not authentication:

1. The page reads the fragment into memory and calls `history.replaceState(null, '', '/')` before any network call.
2. It calls `GET /api/v1/info` and compares the `spki_sha256` there with the key in the URL. This catches a different Mac answering the address (a stale QR after a DHCP change, or another Mac on the same address). It does not catch an impostor, who would echo the key.
3. Only when they match does it post `/api/v1/pair` with the code.

**The one mechanism that authenticates the host** is installing the Mac's certificate on the phone as a trusted profile, which needs a new route serving that certificate. The owner decides whether to take it. It costs an install before scanning, and the plan does not build it because the issue asks for the click-through.

If a phone cannot get past the warning at all, the iOS run item below names the fallback.

### LAN switch

The page is served only when the port listens. The port listens on the LAN only when "Allow devices on the local network" is on (`src/ui/remote.rs` 110–112; `docs/design.md` 3143–3148, "the port does not listen" unless the owner allows the local network). The plan keeps this one switch (owner Q8, default). The window shows the URL only when `identity.listen.lan` is on; otherwise it says "Turn on Allow devices on the local network to pair a phone".

## UI

### Pairing window (`src/ui/remote.rs`, `pairing` at 141)

- The QR payload (the `serde_json::json!` payload at 157–165) becomes the URL. The builder is a pure function, `invite_url(server_id, spki_sha256, code, ip, port) -> String`, with IPv6 bracketed. The `addresses` array is dropped from the QR, since a URL carries one host (`local_address`, `src/adapters/remote.rs` 962–970, returns one address, **[settled]**).
- The URL is also shown as selectable text beside the QR, not only under "Copy invite". This is the fallback for a phone whose camera will not open the URL (Unresolved table): the owner types or copies it into the phone's browser.
- "Copy invite" (around 201) copies the URL. The JSON form is not produced.
- The window keeps its `Key <short hash>` line, so the owner can compare it with the key in the URL on the phone if they choose.
- Scope: the window's scope choice stays.

### Devices dialog ("Remote devices…")

- The devices list is `devices` at `src/ui/remote.rs` 230. A browser device shows its name from `device_name` and its scope, last seen and Revoke, as native ones do.
- The host block, where "Allow devices on the local network" sits, is at 85–100. One sentence under the "Allow pairing" button: "A phone pairs by scanning the code. The first visit trusts the certificate the phone is shown, and nothing checks the host's identity." (The sentence is the owner's to keep or cut with Decision 1.)

### The page (`web/index.html`, `web/app.js`, `web/app.css`)

Behaviour, in order:

1. **Start.** Read the fragment and clear it, then run Decision 1 steps 1–3. With no code in the fragment and no cookie session (the page learns this from a 401 on `/waiting`), show "Pair this device from the Mac" with the host name from `/info`, and no sessions (owner Q7, default).
2. **Pairing.** A 200 from `/pair` stores `server_id`, `spki_sha256` and `scope` in `localStorage`, and the page clears the in-memory code.
3. **Sessions.** `GET /api/v1/projects`, then `GET /api/v1/projects/{id}/sessions` for each, grouped by project. Waiting sessions (`waiting: true`) are sorted to the top of their group and to the top of the page, and marked. Each card shows the kind, the state (`card`, or `waiting_reason` when waiting), and `last_activity_ms` as a relative time.
4. **Live.** Open `wss://<location.host>/api/v1/events`. On `hello`, record the revision. On `changed` or `removed` for a session, refetch that session's entry and the `waiting` collection. On an `epoch` change, refetch everything. On `resync_required`, reconnect and refetch. Replies are applied only if their revision is at or past the held one for that key (spec 146).
5. **Session view.** A Claude Code session shows `GET /api/v1/sessions/{id}/last-response`; anything else shows `GET /api/v1/sessions/{id}/screen?lines=…`. Refetch on `changed` for conversation or screen kinds for that id.
6. **Reply.** A box posts to `POST /api/v1/sessions/{id}/send` with the `op` idempotency key the route expects (`src/app.rs` 3154–3158 comment; the body field names are read from the wire type at implementation, not from this plan). The box is shown only when `scope` is `control`. A 403 with code `scope` hides it and says "This device reads only; raise its scope on the host".
7. **Errors.**
   - 401: fetch `/info`. A different key shows "Identity changed, pair again" and stops. The same key shows "This device is no longer paired with this Mac" (revoked, or the record is gone).
   - 503 or 429: a short "The Mac is busy; retrying" and a retry with backoff.

## Spec and design amendments

- `docs/remote-client.md`:
  - §5 first bullet (`302–320`, the line at 306): "Require pairing before anything but `/info`, `/pair` and the page's three files."
  - §5 pairing (302–320): the browser path does not pin. Its first visit trusts the certificate the address presents, and the code goes to whichever host answers. The spec's "verifies the key before the code is sent" holds for native clients only.
  - §5 token bullet (312): the bearer token is still never in a URL. Add that the pairing code may be in the URL fragment, which the host never receives, and that the fragment does not stop an on-path attacker.
  - §4.3 table (157–220): rows for `GET /` and the three files, with scope "none (before pairing)".
  - Document the browser path in §4.1, §4.3, §5, §11 and §13.
- `docs/design.md`: the Trust boundary (219) and the "Remote client (2026-10-12)" status section (3107 onward) get one line for this item, "A2 — the web control surface", with what is built and the known gaps (the trust step, the iOS run items).
- `README.md`: the Layout table gets rows for `src/adapters/remote_page.rs` and `web/`; the data directory contents are unchanged (no new file); no new dev-aid environment variable.
- `CHANGELOG.md`: one line under Unreleased.

## Tests

`tests/remote.rs`:

- **The page's headers, and answered without the frame.** `GET /` returns 200, `text/html; charset=utf-8`, `Cache-Control: no-store`, `nosniff`, `Connection: close`, and a CSP whose `connect-src` names the `Host` the test sent (`wss://127.0.0.1:<port>`). The test does not call `serve_pending`, so the 200 shows the static path is answered on the connection thread. `/app.js` and `/app.css` return their types. Helper `get` (327) is extended to read response headers, or a raw read in the `http` helper (245).
- **Bad Host.** A request with no `Host`, or with `Host: a"b`, gets 400 and no page.
- **The query is inert.** `GET /?code=ABCD1234&key=…` serves the same bytes as `GET /`, and the reply carries no `code`.
- **The page only uses served paths.** A test reads `web/app.js` and `web/index.html`, extracts every `/api/v1/…` literal, and asserts each is answered by `serve_remote` (not `not_found`) once paired. This is the check the issue asks for, done by literal scan.
- **Other paths stay refused.** The existing `the_allowlist_refuses_everything_else_with_no_side_effect` (682) and `info_is_served_to_anyone_and_every_other_read_is_refused_unpaired` (426) cover this, and are not changed (facts "Settled": neither asserts on `/`).
- **Pairing from the URL, end to end, with the host's own client.** Use `pair` (406) with a `via: browser` variant. Check: the reply has `Set-Cookie` with `__Host-sb=`, `HttpOnly`, `Secure`, `SameSite=Strict`; the body has no `token`; `GET /api/v1/waiting` with the cookie returns 200; the same request with no cookie returns 401.
- **Origin.** `GET /api/v1/events` with the cookie and `Origin: https://127.0.0.1:<port>` upgrades (101). With `Origin: https://evil.example` it returns 403 `origin`; with no `Origin` it also returns 403 `origin`. A cookie POST with a foreign or missing `Origin` is 403. A native bearer request with no `Origin` is unaffected.
- **Revoked and rotated.** Revoke the device (`RevokeDevice`): the cookie gets 401 "the token is not known; pair again", and `/info`'s key is the same. A key rotation (`load_identity`, `src/app.rs` 2782) gives the 401 and a different `/info` key.
- **Pool turnover.** Four static requests in sequence, each closed by the host, leave the pool free for a fifth.
- **Fixtures.** `write_fixtures` (1587) writes the invite as the URL form too. `client/Tests/SwitchboardProtocolTests/FixturesTests.swift` (line 161) decodes both the URL and the JSON form on both sides.

`src/ui/remote.rs` (unit): `invite_url` for an IPv4 address, an IPv6 address (brackets), and a missing address (empty, the window shows "no address").

`tests/ui.rs`: `remote_devices_dialog_opens_pairing_and_revokes_a_device` (7424) asserts that "Copy invite" (7474) copies a text starting `https://` and containing `#code=`, and that the window shows "Key" with the short hash and the URL as text. With LAN off, the window shows the "Turn on…" line and no URL.

Not automated: the page's JavaScript, the cookie's behaviour on a phone and the warning. There is no browser harness, as the issue says. These are checked by the acceptance steps on a phone, recorded in the commit's PR body, and the open run items in the table below are closed by that run.

## Unresolved facts and owner decisions

| Item | Status | Plan default | Fallback if the answer goes the other way |
|---|---|---|---|
| Trust step: what the first visit trusts (Decision 1) | For the owner | Click-through, stated as the trust step; key in URL catches a wrong Mac only | Install the Mac's certificate as a trusted profile: the one option that authenticates the host, at the cost of an install before scanning; needs a new route |
| iOS accepts the no-SAN, year-4096 certificate for a click-through | Run item | Click-through as is | If the phone refuses the click-through, reissue the certificate from the Keychain key with a SAN (new code, same SPKI) |
| Phone camera opens an `https://` URL with a bad certificate | Run item (facts Unresolved) | URL in the QR | The URL as selectable text in the window (in the plan); the owner types it into the phone's browser |
| Browser sends the `Secure` `SameSite=Strict` cookie on the WebSocket upgrade for a self-signed origin | Run item (facts Unresolved) | Cookie | Subprotocol echo (token in the handshake, which only the addressed server sees), with the token held in script-readable storage, so `HttpOnly` is lost |
| `__Host-sb` cookie read by other `Secure` services on this host, any port (RFC 6265 §8.5) | For the owner | Cookie, with this cost written down | The subprotocol echo above |
| `wss://` covered by the click-through on the same origin | Run item | Assumed | If not, the warning recurs on the socket; the page says "accept the warning again" |
| Browser's parallel connections and idle keep-alive hold of the 4-slot pool | Run item (facts Unresolved) | Four slots, `Connection: close` on static | None built; the phone measurement decides, and a result that four is too few returns to the owner (see Pools) |
| Pool for static paths (issue decision 4) | Owner (Q6) | Static paths share the four unauthenticated slots; no window | No window is built; the flood exposure is the same as `/info` and `/pair` today |
| Run items on storage and history: iOS storage eviction for `localStorage` and cookies on an IP origin; history, autocomplete or tab restore keeping the pre-`replaceState` URL | Run item (facts Unresolved) | Cookie with `Max-Age`; fragment, so the code is out of the request line | Re-pair from the Mac's window. The QR code is single-use and expires in five minutes, which bounds what a history entry holds |
| One QR or two (issue decision 3) | Owner (Q3) | URL in the QR, in "Copy invite" and as text; `PairingInvite.parse` accepts both forms | Keep a second "Copy JSON" button |

## Swift edits

The URL form is a Swift change, and it is one function plus two strings:

- `client/Sources/SwitchboardProtocol/Info.swift` (`PairingInvite.parse`, 114): accepts the URL form in addition to the JSON. Both callers keep working unchanged: `client/Sources/switchboard-remote/main.swift` 111 and `client/Sources/SwitchboardUI/BrowserView.swift` 106 and 883 call `parse` and do not change.
- `client/Sources/switchboard-remote/main.swift` line 11, the usage text: `pair '<invite json>'` becomes `pair '<invite url or json>'`.
- `client/Tests/SwitchboardProtocolTests/FixturesTests.swift` (161): one case for the URL form.

No CLAUDE.md edit. The CLAUDE.md contract is `src/core/model.rs`, `src/core/action.rs` and `src/ports/`, and none of them changes. The wire form is in `docs/remote-client.md` §5, which the spec amendments above change.

## Commit and contract additions

Commit message (one commit, Rust and Swift together):

`Serve a browser control page on the remote port and pair it from a URL whose code is in the fragment`

Contract additions: **none.** `src/core/model.rs`, `src/core/action.rs` and `src/ports/` are unchanged. The `RemoteRequest` fields (`cookie`, `host`, `origin`) and the `RemoteReply` headers are adapter types in `src/adapters/remote.rs`, not the shared contract.
