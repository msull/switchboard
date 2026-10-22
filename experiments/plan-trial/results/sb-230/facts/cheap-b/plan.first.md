# Issue #230: a web control surface on the remote port

Line numbers are from this worktree (`dispatch/230-a-web-control-surface-the-remote-port-se`) as checked for this plan. Facts cited as **[settled]** are lines under Settled in `facts.md`. Anything else is marked where it is used.

## Where this departs from the issue

1. **The code moves out of the query string and into the URL fragment.** The issue's URL is `/?code=…&key=…&id=…`. The tree sends the request target to the host unchanged: `read_request` splits the query into `RemoteRequest.query` (`src/adapters/remote.rs` 496–502), and the request line is read before any script runs. A GET that carries `?code=` therefore hands the code to whoever answers the phone's first TLS connection, and the page's key comparison comes too late to stop it. Spec §5 forbids exactly this ("the code alone would let an intermediary … take the token", `docs/remote-client.md` 302–320). The plan uses `https://<address>:<port>/#code=…&key=…&id=…`. A fragment is not part of the request target (RFC 9112 §3.2: origin-form is `absolute-path [ "?" query ]`), so the code never reaches the host's request line, its logs or its caches. This line is not in `facts.md`; the implementer adds it there before the commit.
2. **"The one-line excerpt the card shows" does not exist as a field.** `SessionView.card` is the card's word (`control/src/lib.rs` 645–646) **[settled]**. The card's second line is `waiting_reason` (`src/ui/cards.rs` 197). The page shows `waiting_reason` when `waiting` is true and `card` otherwise. No new view field.
3. **The device's own scope in the header.** No route returns a calling device's scope (`src/app.rs` 3031–3125; facts §3 and "Settled"). The page stores the scope the `/pair` reply grants, and learns a change from a 403 with code `scope` (`src/app.rs` 3166–3173) **[settled]**.
4. **Revoked and "identity changed" look the same on the wire.** Both reach the 401 "the token is not known; pair again" (`src/app.rs` 3146). The host stays as it is. The page stores the key it paired with, and on a 401 it fetches `/info`. A different key means "identity changed, pair again"; the same key means "no longer paired with this Mac". The issue's "revoked" message is delivered this way.
5. **"The key check protects the page" holds only if the page is genuine.** The issue says so. On a first visit the phone trusts whatever answers, so a page served by an impostor is not checked by anything. The plan adds an owner-visible key confirmation (Decision 1). This changes the acceptance: "accept the warning, see the sessions" becomes "accept the warning, confirm the key matches the Mac, see the sessions". If the owner refuses the step, the fallback in Decision 1 applies.
6. **The pairing window shows no text beside the QR.** `src/ui/remote.rs` shows the code, `Key <short hash>`, `Address`, and a "Copy invite" button (`:201`). It does not display the payload. The plan keeps this layout; the copied text becomes the URL.
7. **IPv6.** The window formats the address as `{ip}:{port}` (`src/ui/remote.rs` 158). A URL needs `[addr]:port`. The URL builder brackets IPv6.
8. **The static files cannot be served the way the tree serves other paths.** Today every path other than `/api/v1/` is queued to the frame and refused 404 (`src/app.rs` 3023). The plan answers the page's own files on the connection thread, with no frame round-trip and no core state. This is a change to the port, not to the issue.

Checked and not departed from: the headset pairs by lookup plus typed code (`client/Sources/SwitchboardClientCore/ConnectionManager.swift` 474) and the CLI takes the invite as one argument (`client/Sources/switchboard-remote/main.swift` 111). A `changed {waiting}` event exists: `ResourceKind::Waiting` (`src/core/revision.rs` 56), emitted from `diff_pass` (`src/app.rs` 2614–2640), tested at `tests/remote.rs` 2764 **[settled]**. Dispatch ticket routes already exist on this port (`src/app.rs` 3078–3098); the page does not call them.

## Record

No change to any persisted record.

- `SCHEMA_VERSION` stays **16** (`src/core/model.rs` 12). Nothing in a workspace record changes.
- `VIEWS_SCHEMA_VERSION` stays **7** (`src/core/model.rs` 612).
- `IDENTITY_SCHEMA_VERSION` stays **1** (`src/ports/remote.rs` 17). The fallback certificate reissue (Decision 1) rewrites the existing `certificate` field from the same Keychain key; it adds no field.
- `devices.json` has no version field; it is a bare JSON array of `PairedDevice` (`src/adapters/remote.rs` 83–90, `src/ports/remote.rs` 96). A browser pairing writes one `PairedDevice` exactly as a native one does: `name` from the page, `scope` from the pairing window, `token_sha256` of the cookie's token. No new field.
- The page's own state lives in the browser, not on the host. It holds only what the page needs to render and to recognise a changed key: `server_id`, the pinned key (`spki_sha256`), the scope, the host name. None of it is a secret. It goes in `localStorage` in a `try`/`catch`, and the page works without it (a missing scope or key falls back to `/info` and to a 403 correction).
- The bearer token is never stored by the page. It lives only in the `__Host-` cookie (Decision 2), which the page's script cannot read.

## Signals

No new signal. The page uses what the spec already defines:

- `hello {epoch, revision}` (`docs/remote-client.md` 238) and `changed`/`removed` with the metadata kinds (240). A `waiting` change arrives as `changed {waiting}` **[settled]**.
- On an `epoch` change, the page refetches everything. On `resync_required`, it reconnects and refetches (spec 249–250 area; the rule is §4.4 and §11).
- Replies are applied by revision per resource key (spec 146), so an older reply does not overwrite a newer one.

## Actions and effects

### Core: nothing new

`src/core/` is unchanged. Pairing reuses `OpenPairing` (window, five minutes, `PAIRING_WINDOW` at `src/core/action.rs` 972), `PairingFailed` (three attempts, `PAIRING_ATTEMPTS` at 973), `Paired`, `RevokeDevice` (`src/core/remote.rs` 86–90) and `SetDeviceScope` (93–100). No `AppAction` or `Effect` is added.

### Port: the page's files and the connection thread

A new module `src/adapters/remote_page.rs` holds the embedded files and the response rules. It is listed in the README Layout table.

- **Files**: `web/index.html`, `web/app.js`, `web/app.css`, embedded with `include_str!`. No build step, no bundler, no package manager.
- **Paths**, GET only:
  - `/` → `index.html`, `text/html; charset=utf-8`
  - `/app.js` → `text/javascript; charset=utf-8`
  - `/app.css` → `text/css; charset=utf-8`
  Any other method or path is answered as today (through the frame, 404). The query on `/` is ignored, so an old `?code=` link still loads the page and the code it carries is not read by the host.
- **Headers** on every page response:
  - `Cache-Control: no-store` (stricter than `no-cache`; it covers the query-less page too, and the issue asks for no caching across versions).
  - `Content-Security-Policy: default-src 'none'; script-src 'self'; style-src 'self'; connect-src 'self' wss:; base-uri 'none'; form-action 'none'; frame-ancestors 'none'`. There is no inline script, no inline style and no external resource. `connect-src` lists `wss:` explicitly, because `'self'` does not reliably cover the WebSocket in every browser (facts "Primary sources", CSP `connect-src`). `wss:` is a scheme source, so the static policy needs no host name, which the server cannot know for every address.
  - `X-Content-Type-Options: nosniff`, `Referrer-Policy: no-referrer`, `Connection: close`.
- **Where it is answered**: in `connection` (`src/adapters/remote.rs` 770), after `read_request` (444) and before `requests.try_send`. The response is written with the connection thread; the frame is never involved. `respond` (530) gains a header parameter; its fixed head (545) stays the same for JSON.
- **Pools**: a static request counts against the unauthenticated pool like any other unauthenticated connection (`MAX_UNAUTHENTICATED`, 567), and the connection is closed after the reply, so the slot turns over at once. There is no rate window for static paths; the responses are constant bytes and the pool bounds them. **For the owner (issue decision 4, owner Q6):** if a phone shows a fifth parallel connection dropped at accept (`src/adapters/remote.rs` 699–706), the fallback is `MAX_UNAUTHENTICATED = 8`; the spec §4.1 number changes with it.
- **Before pairing**: the page is served with no code and no token, as the issue asks, and it shows nothing private (issue decision 4; owner Q7).

### Pairing: the URL, the cookie and the Origin check (Decision 2)

- **Request**: the browser's `/pair` body is the native body plus `"via": "browser"`. The native clients never send `via`, so they keep receiving the token in the body (`src/app.rs` 3606, `PairReply` in `client/Sources/SwitchboardProtocol/Info.swift`).
- **Reply for `via: browser`**: `Set-Cookie: __Host-sb=<token>; HttpOnly; Secure; SameSite=Strict; Path=/; Max-Age=31536000`, and a body of `{scope, server_id, device_id}` with no `token`. `Max-Age` is set so the cookie survives a browser restart; the host still revokes by record.
  - Why a cookie: `HttpOnly` keeps the token from script on the page, which the issue's `localStorage` option could not do. `__Host-` pins the cookie to this host and port's origin (the lead in facts "Primary sources", Cookies, is covered by the prefix). `SameSite=Strict` keeps it off cross-site requests.
  - Why not the subprotocol: RFC 6455 §4.2.2 makes the server echo one offered value in the 101, so the token would appear in the handshake headers (facts "Primary sources", Subprotocol). That is the fallback only.
- **Reading the cookie**: `read_request` (`src/adapters/remote.rs` 444–511) keeps `Cookie`, `Host` and `Origin` on `RemoteRequest` (360). `authenticate` (`src/app.rs` 3127) takes the bearer first, as now, and the `__Host-sb` cookie second. Cookie auth is what the page uses.
- **Origin check**: for a request authenticated by cookie that is not a GET, and for the `/api/v1/events` upgrade (`remote_subscribe`, `src/app.rs` 2723), an `Origin` header that is present and differs from `https://<Host>` is refused with 403 code `origin`. Browsers send `Origin` on a WebSocket handshake and the server must check it itself (facts "Primary sources", Origin). Bearer requests from native clients are not checked, as they send no `Origin`.
- **Token and device name**: the page sends `device_name` derived from `navigator.userAgent` (for example `Safari on iPhone`). `new_device` caps it at 64 characters (`src/core/remote.rs` 166–170). The page sends the name; the host does not parse a user agent. The name is a label, not an identity (facts Unresolved, user agent).

### Pairing: the trust step (Decision 1, For the owner)

What the click-through buys: the browser connects to the host at all, and it keeps that one certificate for that address. What it does not buy: any proof of who answered on the first visit. A certificate with no subject alternative name (facts "Settled", `src/adapters/remote.rs` 102) and a validity to the year 4096 (`rcgen-0.14.10/src/certificate.rs` 83–90) are both doubtful for iOS (facts Unresolved, first item), and the phone's behaviour is a run item.

**Plan default**: the warning is accepted, and then the owner confirms the key:

1. The page loads, reads the fragment, clears it with `history.replaceState` before anything else, and calls `GET /api/v1/info`.
2. The page shows the short hash of the `spki_sha256` the host presented (from `/info`, not from the URL). Beside it the Mac's window already shows `Key <short hash>` (`src/ui/remote.rs`, pairing section). The owner taps "Matches the Mac" only if they agree.
3. The page compares `/info`'s key with the URL's key. A mismatch is "this is not the Mac whose code was scanned" and nothing is posted.
4. Only then does it post `/api/v1/pair` with the code.

This is the only check that an impostor on the network cannot satisfy: it would have to show the owner the Mac's key, which it does not have.

**Fallback if the owner declines the tap**: the click-through and the automatic comparison (step 3) only. The devices dialog then says in plain words that the first visit trusts whatever answers the address, and that the owner should confirm the key before scanning on an untrusted network. The fallback is built as the same flow with step 2 removed.

**Fallback if a phone cannot get past the warning at all** (facts Unresolved, the iOS click-through; a run item): reissue the certificate from the existing Keychain key with a subject alternative name. `make_identity` (`src/adapters/remote.rs` 95–128) makes a new key and cannot be reused; the reissue is a new function that takes the key PEM from the Keychain and signs a new certificate (SPKI, so the pin in `identity.json` and in clients is unchanged, `src/adapters/remote.rs` 119). The name is the current LAN address, so a DHCP change means a new warning; a stable `.local` name is an open question with no source in the facts file and is not planned here.

### LAN switch

The page is served only when the port listens. The port listens on the LAN only when "Allow devices on the local network" is on (`src/ui/remote.rs` 110–112; `docs/design.md` 3130–3160). The plan keeps this one switch (owner Q8, default). The window shows the URL only when `identity.listen.lan` is on; otherwise it says "Turn on Allow devices on the local network to pair a phone".

## UI

### Pairing window (`src/ui/remote.rs`, `pairing` at 141)

- The QR payload (around 157–165, the `serde_json::json!` payload) becomes the URL. The builder is a pure function, `invite_url(server_id, spki_sha256, code, ip, port) -> String`, with IPv6 bracketed. The `addresses` array is dropped from the QR, since a URL carries one host (`local_address`, `src/adapters/remote.rs` 962–970, returns one address, **[settled]**).
- "Copy invite" (around 201) copies the URL. The JSON form is not produced.
- The window keeps its `Key <short hash>` line; the browser confirmation in Decision 1 reads it.
- Scope: the window's scope choice stays (owner Q9, default yes).

### Devices dialog ("Remote devices…", `src/ui/remote.rs` 85–100 area for the host block and the devices list around 230)

- A browser device shows its name from `device_name` and its scope, last seen and Revoke, as native ones do.
- One sentence under the "Allow pairing" button: "A phone pairs by scanning the code. The first visit trusts the certificate the phone is shown; confirm the key it shows matches the key here before you tap Matches." (Wording follows the Decision 1 default; the fallback removes the second half.)

### The page (`web/index.html`, `web/app.js`, `web/app.css`)

Behaviour, in order:

1. **Start.** Read `location.hash` into memory. Call `history.replaceState(null, '', '/')` before any network call. If the hash has no code and there is no cookie session (the page learns this from a 401 or from a `/waiting` 401), show "Pair this device from the Mac" with the host name from `/info`, and no sessions (owner Q7, default).
2. **Pairing.** As Decision 1, steps 1–4. The post carries `{code, spki_sha256, device_name, via: "browser"}`. A 200 stores `server_id`, `spki_sha256`, `scope` in `localStorage`, and the page clears the in-memory code.
3. **Sessions.** `GET /api/v1/projects`, then `GET /api/v1/projects/{id}/sessions` for each, grouped by project. Waiting sessions (`waiting: true`) are sorted to the top of their group and to the top of the page, and marked. Each card shows the kind, the state (`card`, or `waiting_reason` when waiting), and `last_activity_ms` as a relative time.
4. **Live.** Open `wss://<location.host>/api/v1/events`. On `hello`, record the revision. On `changed` or `removed` for a session, refetch that session's entry and the `waiting` collection. On an `epoch` change, refetch everything. On `resync_required`, reconnect and refetch. Replies are applied only if their revision is at or past the held one for that key (spec 146).
5. **Session view.** A Claude Code session shows `GET /api/v1/sessions/{id}/last-response`; anything else shows `GET /api/v1/sessions/{id}/screen?lines=…`. Refetch on `changed` for conversation or screen kinds for that id.
6. **Reply.** A box posts to `POST /api/v1/sessions/{id}/send` with the `op` idempotency key the route expects (`src/app.rs` 3154–3158 comment; the body field names are read from the wire type at implementation, not from this plan). The box is shown only when `scope` is `control`. A 403 with code `scope` hides it and says "This device reads only; raise its scope on the host".
7. **Errors.**
   - 401: fetch `/info`. A different key shows "Identity changed, pair again" and stops. The same key shows "This device is no longer paired with this Mac" (revoked, or the record is gone).
   - 503 or 429: a short "The Mac is busy; retrying" and a retry with backoff.
   - Never show a token, a code or a key beyond the short hash.

The page has no framework, no build step and no third-party script. It is plain JavaScript, written as the surrounding `web/` code is written. Its text is in the page, not in the host (the host does not render HTML).

## Spec and design amendments

- `docs/remote-client.md`:
  - §5 first bullet (`302–320`, the line at 306): "Require pairing before anything but `/info`, `/pair` and the page's three files."
  - §5 token bullet (312): the bearer token is still never in a URL. Add that the pairing code may be in the URL fragment, which the host never receives.
  - §5 pairing (302–320): add the browser trust step (Decision 1) and the cookie (Decision 2).
  - §4.1 (118–139): the static pool rule and, if the owner's fallback is taken, the new `MAX_UNAUTHENTICATED`.
  - §4.3 table (157–220): rows for `GET /` and the three files, with scope "none (before pairing)".
  - §11 (471–511): browser cases.
  - §13 (524–553): a row for the cookie and the fragment as explicit decisions.
- `docs/design.md`: the Trust boundary (219) and the "Remote client (2026-10-12)" status section (3107 onward) get one line for this item, "A2 — the web control surface", with what is built and the known gaps (the trust step, the iOS run items).
- `README.md`: the Layout table gets rows for `src/adapters/remote_page.rs` and `web/`; the data directory contents are unchanged (no new file); no new dev-aid environment variable.
- `CHANGELOG.md`: one line under Unreleased.
- `facts.md`: a line for the fragment rule (RFC 9112 §3.2) and for the `Set-Cookie` and `Origin` behaviour the plan relies on, as they are settled by the time of the commit.

## Tests

Core: none (no core change).

`tests/remote.rs`:

- **The page's headers.** `GET /` returns 200, `text/html; charset=utf-8`, `Cache-Control: no-store`, the exact CSP string, `nosniff`, `Connection: close`. `/app.js` and `/app.css` return their types. Helper `get` (327) is extended to read response headers, or a raw read in the `http` helper (245).
- **Answered without the frame.** The `/` test does not call `serve_pending`; a 200 with no frame pass shows the static path is answered on the connection thread.
- **The query is inert.** `GET /?code=ABCD1234&key=…` serves the same bytes as `GET /`, and the reply carries no `code`.
- **The page only uses served paths.** A test reads `web/app.js` and `web/index.html`, extracts every `/api/v1/…` literal, and asserts each is answered by `serve_remote` (not `not_found`) once paired. This is the check the issue asks for, done by literal scan.
- **Other paths stay refused.** `/index.html`, `/api/v2/projects` and `/env/resolve` return 404 `not_found`. The existing `the_allowlist_refuses_everything_else_with_no_side_effect` (682) and `info_is_served_to_anyone_and_every_other_read_is_refused_unpaired` (426) are not changed (facts "Settled": neither asserts on `/`).
- **Pairing from the URL, end to end, with the host's own client.** Use `pair` (406) with a `via: browser` variant. Check: the reply has `Set-Cookie` with `__Host-sb=`, `HttpOnly`, `Secure`, `SameSite=Strict`; the body has no `token`; `GET /api/v1/waiting` with the cookie returns 200; the same request with no cookie returns 401.
- **Native pairing unchanged.** The existing `a_pairing_inside_the_window_gives_a_token_that_reads_every_view` (477) passes unchanged; a native `/pair` still gets `token` in the body.
- **Origin.** `GET /api/v1/events` with the cookie and `Origin: https://evil.example` returns 403 `origin`; with `Origin: https://<Host>` it upgrades (101). A cookie POST with a foreign `Origin` is 403. A native bearer request with no `Origin` is unaffected.
- **Revoked and rotated.** Revoke the device (`RevokeDevice`): the cookie gets 401 "the token is not known; pair again", and `/info`'s key is the same. The page's message rule is the JS side (manual). A key rotation (`load_identity`, `src/app.rs` 2782) gives the 401 and a different `/info` key.
- **Pool turnover.** Four static requests in sequence, each closed by the host, leave the pool free for a fifth; a fifth held open while four are idle is the existing drop behaviour (`src/adapters/remote.rs` 699–706), asserted only as the fallback check.
- **Fixtures.** `write_fixtures` (1587) writes the invite as the URL form too. `client/Tests/SwitchboardProtocolTests/FixturesTests.swift` (line 161, per facts) decodes it on both sides, and `PairingInvite.parse` accepts both the URL and the JSON form (`client/Sources/SwitchboardProtocol/Info.swift` 114).

`src/ui/remote.rs` (unit): `invite_url` for an IPv4 address, an IPv6 address (brackets), and a missing address (empty, the window shows "no address").

`tests/ui.rs`: `remote_devices_dialog_opens_pairing_and_revokes_a_device` (7424) asserts that "Copy invite" (7474) copies a text starting `https://` and containing `#code=`, and that the window shows "Key" with the short hash. With LAN off, the window shows the "Turn on…" line and no URL.

Not automated: the page's JavaScript, the cookie's behaviour on a phone and the warning. There is no browser harness, as the issue says. These are checked by the acceptance steps on a phone, recorded in the commit's PR body, and the open run items in the table below are closed by that run.

## Unresolved facts and owner decisions

| Item | Status | Plan default | Fallback if the answer goes the other way |
|---|---|---|---|
| Trust step: click-through plus key confirmation (Decision 1) | For the owner | Confirmation tap | Click-through and automatic compare only; dialog says so |
| iOS accepts the no-SAN, year-4096 certificate for a click-through | Run item | Click-through as is | Reissue from the Keychain key with a SAN (new code, same SPKI) |
| Browser sends the `Secure` `SameSite=Strict` cookie on the WebSocket upgrade for a self-signed origin | Run item (facts Unresolved) | Cookie | Subprotocol echo (`sb.<token>`) with the token in `localStorage` |
| `wss://` covered by the click-through on the same origin | Run item | Assumed | If not, the warning recurs on the socket; the page says "accept the warning again" |
| Browser's parallel connections and idle keep-alive hold of the 4-slot pool | Run item (facts Unresolved) | Four slots, `Connection: close` on static | Raise `MAX_UNAUTHENTICATED` to 8 (spec §4.1 changes) |
| History, autocomplete or tab restore keeps the pre-`replaceState` URL | Run item | Fragment, so the code is never in a request | None needed for the code; a fragment in history still holds the code, so the owner's QR is single-use anyway |
| Storage eviction on iOS for `localStorage` and cookies on an IP origin | Run item (facts Unresolved) | Cookie with `Max-Age` | Re-pair from the Mac's window |
| Revoked and rotated messages | Owner (Q4) | Page compares stored key with `/info` | Host adds a `revoked` code on the 401; page shows one message |
| Pool for unpaired page loads (issue decision 4) | Owner (Q6) | Static on the connection thread, unauthenticated pool | Separate pool or a rate window for static paths |
| What the page shows with no code and no token | Owner (Q7) | "Pair this device from the Mac" plus host name | Show the window's instructions only |
| LAN switch for the page | Owner (Q8) | Same switch; window says so | A separate page-only switch: a new optional field in `identity.json`, which would need a schema step (`IDENTITY_SCHEMA_VERSION` 1 → 2, from the value in `src/ports/remote.rs` at implementation time, plus one) |
| Scope for the browser pairing | Owner (Q9) | Chosen in the window | Page default scope |
| One QR or two (issue decision 3) | Owner (Q3) | URL in the QR and in "Copy invite"; `PairingInvite.parse` accepts both forms | Keep a second "Copy JSON" button |
| Device name from the user agent | Facts Unresolved | Best-effort label | Owner renames in the dialog (no rename route exists today; the label stays as sent) |
| Multiple addresses | Tree (single `local_address`) | One address | A second address needs a typed URL; not in this cut |

## Commit and contract additions

Commit message (one commit, Rust and Swift together):

`Serve a browser control page on the remote port and pair it from a URL whose code is in the fragment`

Contract additions: **none.** `src/core/model.rs`, `src/core/action.rs` and `src/ports/` are unchanged. The `RemoteRequest` fields (`host`, `origin`, `cookie`) and the `RemoteReply` headers are adapter types in `src/adapters/remote.rs`, not the shared contract.
