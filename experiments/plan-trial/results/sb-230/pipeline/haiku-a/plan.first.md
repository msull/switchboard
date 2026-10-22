# Issue #230: a web control surface on the remote port

Plan against worktree `/Users/sbgragent/.dispatch/worktrees/1ea22e2f`, HEAD `7fe8949`. Every reference below was read in that worktree. Browser behaviour marked **verify** is not known from the code and is settled by the spike in "Spike first".

## Where this departs from the issue

Each claim the code does not bear out, and what the plan does instead.

1. **"Each with its kind, state and the one-line excerpt the card shows."** The card's excerpt is computed in the UI from the transcript (`src/ui/cards.rs` 176–185 picks the caption; `agent_excerpt` 558–575). The session view has `card` (the word), `waiting` and `waiting_reason` (`control/src/lib.rs` 637–660), and no excerpt. The plan: the list shows kind, `card` and `waiting_reason`; the excerpt appears in the session view, from `last-response`. No excerpt field is added to the wire this cut. If the owner wants the list to match the card exactly, that is a later `SessionView` field computed by the app from its conversation cache.
2. **"The code never stays in the address bar or the history"** with the code in the query. `history.replaceState` clears the address bar only after the page's script runs. The browser sends the full URL in the GET request line to whichever host answers it, before any script runs, so a stale QR whose address now belongs to another host gets the code. The plan puts the pairing values in the **fragment** (`/#code=…&key=…&id=…`). A fragment is never sent in a request. This is a deviation from the issue's text.
3. **"Open the pairing window, scan the QR."** With the local network off the port binds loopback (`src/app.rs` 2962), yet the window opens and its QR carries `local_address()` regardless (`src/ui/remote.rs` 157), and `OpenPairing` has no `lan` check (`src/core/remote.rs` 46–52). The phone cannot reach that address. The plan: `OpenPairing` is refused with lan off, with the message "Turn on Allow devices on the local network first", and the dialog disables the button.
4. **"The page keeps the token."** `/pair` puts `token` in the JSON body (`src/app.rs` 3656–3661). A script that reads the reply holds the token, so a browser cannot be given the body. The plan: a browser asks with `browser: true` and receives a cookie and no token (decision 2).
5. **"Identity changed, pair again" versus "revoked."** A revoked token and a rotated key both get the same 401, "the token is not known; pair again" (`src/app.rs` 3142–3147); rotation clears `devices.json`. The page cannot tell them apart from the reply. The plan: the page keeps the server's SPKI hash in `localStorage` (not secret) and compares it with `/info` on a 401. A different key is "identity changed"; the same key is "revoked".
6. **"The device's scope and host name in the header."** The host name is in `/info` (`server_name`, `src/app.rs` 3568–3596), which is pre-auth. Nothing lets a paired device read its own scope; `/pair` returns it once. The plan adds `GET /api/v1/device`.
7. **"A session that starts waiting moves to the top without a reload."** This does hold. `waiting` is a diff resource (`src/core/revision.rs` 56) and `changed {waiting}` plus `changed {session}` reach a subscriber. No host change is needed for it.

Nothing else in the issue's "What exists" section needs a departure: the QR fields, the five-minute and three-failure window (`src/ui/remote.rs` 194; `src/core/remote.rs` 58–63), the three protocol speakers, and the `/api/v1/` allowlist (`src/app.rs` 3023–3025) are as described.

## The record

- **`devices.json` is unchanged.** A browser is a `PairedDevice` (`src/ports/remote.rs` 96–110). Its `name` is the page's own parse of the user agent (`Safari on iPhone`), sent as `device_name`, which `remote_pair` already reads (`src/app.rs` 3650) and `new_device` trims to 64 characters (`src/core/remote.rs` 166–175). Its `scope` is the open window's scope (decision 6). Its token hash is stored as now. Nothing about the browser's cookie is stored on the host beyond that hash.
- **`identity.json` is unchanged** unless the spike rejects the certificate (decision 1). The certificate has no SANs and rcgen's default validity (`src/adapters/remote.rs` 91–102). If Safari refuses such a leaf outright rather than warning, the fix is to re-issue the certificate over the same Keychain key with a finite validity, replacing the `certificate` field only. The SPKI hash, and so every native pin, does not change.
- **Browser-side state**, in `localStorage` and wrapped in try/catch: the server id, the SPKI hash last seen, and the device name. Never the token; the cookie is HttpOnly (decision 2).
- **Schema versions, as they are today:** `SCHEMA_VERSION` 16 (`src/core/model.rs` 12), `VIEWS_SCHEMA_VERSION` 7 (`src/core/model.rs` 612), `IDENTITY_SCHEMA_VERSION` 1 (`src/ports/remote.rs` 17). `devices.json` has no version field. **No bump is planned.** If the spike forces an identity change, that change still needs no bump, since the `certificate` field already exists; if the implementer adds any field to `model.rs` the bump is that file's value at implementation time plus one.

**Decision 1, browser trust.** The click-through is the trust step. The dialog says so: "A browser trusts this Mac only through the certificate warning you accept. Accept it only on your local network, and only for the address shown here." The click-through does not prove identity; it binds the browser to the connection's key for that address. The `key` check in the page stops a genuine page from sending the code to a host with a different key. It cannot stop a page the click let through. The host does not offer its certificate for download: that needs a route outside `/api/v1/` before pairing, which §5 forbids. The rule in `docs/remote-client.md` §5 holds: before pairing, a non-loopback bind answers only `/info`, `/pair` and the page's own files.

**Decision 2, the token reaches the page through a cookie.** The `/pair` reply to a body with `browser: true` sets `Set-Cookie: switchboard-<server_id>=<token>; HttpOnly; Secure; SameSite=Lax; Path=/; Max-Age=34560000` and leaves `token` out of the body. Native clients send no `browser` field and keep the bearer header and the body token (the issue's requirement). `SameSite=Lax`, not the issue's `Strict`, because a link opened from Messages or Mail is a cross-site navigation: `Strict` drops the cookie and the paired phone shows "not paired". `Lax` still withholds the cookie from cross-site `fetch`, `POST` and WebSocket, which are the state-changing requests. The cookie name is per installation, so two installations on one address do not collide. Cookies are keyed by host, not port; the dev case of two instances on one address with different ports is covered only because each instance's name differs.

`authenticate` (`src/app.rs` 3127) accepts the bearer first and the cookie second. A cookie-authenticated request with an `Origin` header that is not `https://<Host header>` is refused with 403 `origin`. The Host header is not kept today (`src/adapters/remote.rs` 444–516), so `read_request` keeps `Cookie`, `Host` and `Origin`. The host's `/pair` reply stays HTTP 200 with the cookie; the 401 for a revoked token is unchanged.

**Spec amendment.** `docs/remote-client.md` §5 says the token is "sent only as a bearer header … never in a URL" and that `/pair` answers "with the token". Both gain the browser's cookie path. The token still never travels in a URL.

## Signals

No new event. The page uses what exists: `hello` (epoch and revision, `src/app.rs` 2738), `changed` and `removed` from the diff pass for `waiting`, `session`, `projects`, `sessions` and `conversation`, and `resync_required` (`docs/remote-client.md` §4.4). The page sends `watch {kind: "conversation", id}` for a Claude Code session and `watch {kind: "screen", id}` for any other, as §4.4 describes. A watch set is capped at 128 keys globally, so the page unwatches when a session view closes.

## Actions and effects

No new `AppAction` or `Effect` variant, and no port change. The frame and adapter changes:

- **`OpenPairing` with lan off** (`src/core/remote.rs` 46–52): core refuses the window and calls `self.error(...)`, the same path as the three-failure message at 61. Test in `src/core/tests.rs`.
- **`invite_url`**: a pure function in `src/core/remote.rs` that builds `https://{address}:{port}/#code={code}&key={spki_sha256}&id={server_id}`. The address is IPv4 only, because `local_address()` (`src/adapters/remote.rs` 962) binds `0.0.0.0` and returns an IPv4 address. The UI passes the address in.
- **`/pair` for a browser**: `remote_pair` (`src/app.rs` 3606–3670) reads `browser`. The reply carries `Set-Cookie` and no `token`. `RemoteReply` gains `headers` (`src/adapters/remote.rs` 389–395), and `respond` writes them and takes a content type (`src/adapters/remote.rs` 530–553, which hard-codes `application/json` today). The 28 literal `RemoteReply { … }` sites in `src/app.rs` move to a `RemoteReply::json(status, body, authenticated)` constructor, a mechanical change.
- **`GET /api/v1/device`** (read scope): `{device_id, name, scope}`, built as `remote_info` builds its body (`src/app.rs` 3568). Served under the same `authenticate`. Scope changes reach the page at its next request, as `403 scope` already does (`src/app.rs` 3166–3173).
- **Static set, in the adapter** (decision 4). `src/adapters/remote_page.rs` holds `include_str!` of `web/index.html`, `web/app.css` and `web/app.js`. `connection()` answers a `GET` to `/`, `/app.css`, `/app.js` or `/favicon.ico` before it reaches `requests.try_send` (`src/adapters/remote.rs` 770–812). `/favicon.ico` is 404 with no body, so the frame never sees a page request. Any other method or path keeps the existing frame path.
- **Rate window and pool** (decision 4). `Limits` (`src/adapters/remote.rs` 623–640) gains a `page` window of 60 per minute per listener for the three static paths. A rate-limited page load gets 429 as text.
- **Unauthenticated replies close the connection.** Every reply with `authenticated: false` carries `Connection: close`, and `connection()` returns after writing it. A slot in the 4-entry unauthenticated pool is then held for one request, not for up to 30 s of idle keep-alive (`IDLE_TIMEOUT`, `src/adapters/remote.rs` 573). This is a §4.1 change, stated in the spec amendment. Decision 4 holds the pool at 4 unless the spike shows a page load failing; if it does, the bound moves to 8, an owner decision recorded in §4.1.
- **Unauthenticated page with no code and no token** is answered by the page, not the host: the page's first `GET /info` decides (see UI).

## UI

**Pairing window** (`src/ui/remote.rs` 141–200). The address line and the key line stay. The QR encodes `invite_url`, as does the text beside it; "Copy invite" (line 201) copies the same URL. The line at 194 becomes "Scan with the phone's camera, or copy the link into the device. Grants {scope} scope. Open for five minutes or three wrong codes." With lan off, the button is disabled and reads "Allow devices on the local network above first". With lan on and no local address (`local_address()` is `None`), the window shows "No local address; check the network" and no QR.

**Devices dialog** (`src/ui/remote.rs` 229–276). Adds the trust sentence from decision 1. Rows are unchanged: name, scope (the combo at 243–253), last seen, Revoke.

**The page** (`web/`, a second client, no framework, no build step).

- *Load.* `app.js` runs first. It reads `location.hash` into `URLSearchParams`, then calls `history.replaceState(null, '', '/')`, then does anything else. The code is uppercased, as the core and `/pair` do (`src/core/remote.rs` 50; `src/app.rs` 3646).
- *Pairing.* With the fragment present, the page fetches `GET /api/v1/info`, and compares `key` with `spki_sha256` (case-insensitive). On a mismatch it shows "This is not the Mac whose code was scanned" and sends nothing, so the window's failure counter does not move (`src/app.rs` 3636–3645). On a match it posts `POST /api/v1/pair` with `{code, spki_sha256, browser: true, device_name}`. On 200 the cookie is set and the page forgets the fragment. On 403 `wrong_code` it shows "The code does not match. The window closes after three wrong codes." On 403 `pairing_closed` it shows "Pairing is closed on the Mac".
- *No fragment, no cookie.* `/info` decides. If `pairing_open` is true: "Scan the code on the Mac, or open the link it shows." If false: "Pairing is closed. On the Mac, open Remote devices and choose Allow pairing for 5 minutes."
- *With a cookie.* The page opens `wss://…/api/v1/events` first, waits for `hello`, then snapshots `GET /api/v1/projects`, `GET /api/v1/projects/{p}/sessions` for each, `GET /api/v1/waiting` and `GET /api/v1/device`. Sessions are grouped by project, with the waiting ones first and marked.
- *Session view.* `GET /api/v1/sessions/{id}`. A Claude Code session shows `GET …/last-response`; anything else shows `GET …/screen`, refreshed on `changed`. A reply box posts `POST …/sessions/{id}/send` with the body §4.5 gives. The box is shown when `scope` is `control` and hidden otherwise. A 403 `scope` on send says "This device reads only. Raise it on the Mac."
- *401.* Compare the stored SPKI hash with `/info`: a different key shows "The Mac's key changed; pair again"; the same key shows "This browser is no longer paired" (revoked). Nothing is sent after either.
- *Socket loss.* Reconnect with backoff. `resync_required` and a changed `epoch` in `hello` both refetch everything.
- *Status line.* Header shows the host name from `/info` and the device's `name` and scope from `/device`.

Not in this cut: forget-this-browser (the owner revokes on the Mac), dispatch tickets, drafts, dictation, runs, pinned files.

## Spike first

CLAUDE.md requires a spike directory before building on a risky mechanism. The browser questions are not answerable from the code. Add `spikes/23-browser-page/` (the next free number after `spikes/22-remote-port`) with a README recording commands, numbers and a recommendation. Its questions, asked on an iPhone with Safari and on a second Mac:

1. The certificate warning: is the exception per address and sticky across reload and relaunch; does it cover `wss://` on the same host; does Safari **refuse** a self-signed leaf with no SANs and a validity to year 4096, or only warn (decision 1)?
2. Does `Secure; HttpOnly; SameSite=Lax` on an IP-literal origin store the cookie and send it on `fetch` and on the WebSocket upgrade; does a link opened from Messages carry it?
3. Does `connect-src 'self'` allow `wss:` on the same host under the CSP in decision 4?
4. A cold page load of `/`, `/app.css`, `/app.js` and `/info` in parallel, with `Connection: close` on unauthenticated replies and the pool at 4: does any request reset?

Outcomes: if (1) refuses the certificate, the re-issue in "The record" comes into this commit. If (3) blocks `wss:`, the page polls `waiting` and the session every five seconds, and the acceptance line "moves to the top without a reload" is amended to say so. If (4) resets, the pool bound moves as above.

## Tests

- **`tests/remote.rs`**, with a `http_with` helper beside `http` (245–297) that takes request headers (`Cookie`, `Origin`, `Host`) and returns response headers. `http` discards them today.
  - `the_page_and_its_assets_answer_with_their_headers` (new): `GET /`, `/app.css`, `/app.js` answer 200 with the right `Content-Type`, the exact CSP string, `Cache-Control: no-store`, `X-Content-Type-Options: nosniff`, `Referrer-Policy: no-referrer`, and `Connection: close`. `GET /favicon.ico` is 404 with no body. `GET /other` is still 404 `not_found` (the allowlist test at 682 extended).
  - `the_page_references_only_served_paths` (new): scan `web/app.js` and `web/index.html` for `/api/v1/…` literals and `href`/`src`. Request each with a paired cookie and assert none answers 404 `not_found`. This catches a route added to the page and not the host.
  - `a_browser_pairs_by_cookie_and_the_cookie_reads_and_revokes` (new): pinned client, `GET /info`, `POST /pair` with `browser: true`. Assert `Set-Cookie` carries `HttpOnly; Secure; SameSite=Lax`, the body has no `token`, `GET /api/v1/device` and `GET /api/v1/projects` answer 200 with the cookie, and after `RevokeDevice` the next request is 401.
  - `a_cookie_from_another_origin_is_refused` (new): `Origin` of another origin on a cookie request → 403 `origin`; the same request with the bearer and no `Origin` → 200.
  - `a_pairing_by_the_invite_url_pairs_through_the_host_client` (new): parse the URL from `invite_url`'s output the way `PairingInvite.parse` does, the way the CLI does, then `/pair`, then a read.
  - `stalled_strangers_fill_their_own_pool_and_never_a_paired_clients` (3226) extended: four page loads' connections hold the unauthenticated pool; a paired client still reads; and the no-code page answers while the pool is full (a page load that is not refused is the page's own proof).
  - `a_wrong_pin_fails_at_the_handshake_before_any_request` (1395) and `pair_is_rate_limited_per_listener` (1406) run unchanged.
  - `info_is_served_to_anyone_and_every_other_read_is_refused_unpaired` (426): `/` now answers, so its expectations change; the rest stay.
  - `the_client_fixtures_decode_on_both_sides` (1829) and `write_fixtures` (1587): add `device.json`. `pair.json` changes only if the browser reply is a new shape, which it is not on the native path.
- **Core, `src/core/tests.rs`**: `OpenPairing` with lan off refused and no window; with lan on, a window opens. `invite_url` for a sample identity gives the exact string.
- **Adapter units, `src/adapters/remote.rs`**: `read_request` keeps `Cookie`, `Host`, `Origin`; `respond` writes headers and content type; the `page` window's 61st request in a minute is limited.
- **UI, `tests/ui.rs`**: the dialog test at 7424 seeds `Listen::default()` (lan off). Under the new guard it must seed lan on, or it will show the refusal. Add: a pairing window with lan off shows the "first" message and no QR; with lan on, the QR and the Copy invite text are the same URL.
- **Swift, `client/Tests/SwitchboardProtocolTests/FixturesTests.swift`**: `testInviteParsesTheQRText` (157) gains a URL case. The inline JSON stays for one cut. `PairingInvite.parse` (`client/Sources/SwitchboardProtocol/Info.swift` 113–116) accepts a `https://` URL, reads `key`, `code` and `id` from the fragment, and takes `addresses` from the host and port. Run on macOS; CI on Linux does not compile this target.
- **Manual.** The phone run in the issue's acceptance list is recorded in the spike README, not automated: scan, accept, see the sessions, a waiting session moving, a reply arriving, revocation.

## Docs and status

- `docs/remote-client.md`: §4.1 (the static pool and `Connection: close` on unauthenticated replies, the page's rate window), §4.3 (`GET /`, `/app.css`, `/app.js`, `/favicon.ico`, `GET /api/v1/device`, `POST /api/v1/pair` with `browser`, the 403 `origin`), §5 (the fragment form, the browser trust statement, the cookie).
- `docs/design.md`: trust boundary (219–265) gains "a browser is a paired device" in the device-kind sentence. The remote client status section (3107) gains a dated bullet when the commit lands. Open questions (3672) get the Safari answers from the spike.
- `README.md`: Layout rows for `web/` (new), `src/adapters/remote_page.rs` (new), the `src/adapters/remote.rs` row (383), the `src/ui/remote.rs` row (423), the `tests/remote.rs` row (437), and the `client/` row (446). Dev aids: none new.
- `spikes/README.md`: a row for `23-browser-page`.
- `CHANGELOG.md`: a line under `[Unreleased]` (line 9).
- No `src/script.rs` line: `pairing` (621) and `remote-devices` (734) already reach the states the page needs.

## Order

Each step leaves the app running; the commit is the sum.

1. Spike `spikes/23-browser-page/` and its README (decision gate).
2. `OpenPairing` lan guard and `invite_url` in core, with tests; the pairing window and dialog text in the UI.
3. Adapter: `read_request` headers, `RemoteReply.headers` and `RemoteReply::json`, `respond` with content type, `Connection: close` on unauthenticated replies.
4. Static table and `remote_page.rs`, with `web/` placeholder pages (the page shows "Pairing is closed" for every state); the header and rate tests.
5. `/pair` browser reply, cookie in `authenticate`, Origin check, `GET /api/v1/device`.
6. The real `web/` page.
7. Swift URL case; docs; README; CHANGELOG.

**Commit message:** `Serve a web control surface from the remote port and pair browsers by link`

**Contract additions** (none to `src/core/model.rs`, `src/core/action.rs` or `src/ports/`):

- Wire, additive on `/api/v1`: `GET /api/v1/device` → `{device_id, name, scope}`; `POST /api/v1/pair` takes `browser: true` and then answers with `Set-Cookie` and no `token`; `GET /`, `/app.css`, `/app.js`, `/favicon.ico` answered by the adapter; 403 `origin` for a cookie request with a foreign `Origin`; cookie `switchboard-<server_id>` accepted by `authenticate`.
- Core, additive pure function: `invite_url` in `src/core/remote.rs`.
- Behaviour, no variant change: `OpenPairing` refused with lan off.
- Adapter types (not the shared contract, named so the commit says them): `RemoteRequest` gains `cookie`, `host`, `origin`; `RemoteReply` gains `headers`; `RemoteReply::json`; `remote_page` module; `Limits` gains a `page` window.
- Swift, additive: `PairingInvite.parse` accepts the URL form.
